//! The graph (05): entity notes under Claude/Graph/ and the graph.db cache.
use crate::error::{Error, Kind, Result};
use crate::notes::safe_filename;
use crate::text::{casefold, display, minute_stamp, split_keep_ends, split_lines, strip};
use crate::vault::{Vault, body, get, parse, read_lenient, str_list};
use crate::yaml::{Y, dump_block, dump_flow_leaf};
use chrono::NaiveDateTime;
use regex::Regex;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

pub const TYPES: [(&str, &str); 6] = [
    ("person", "People"),
    ("organization", "Organizations"),
    ("domain", "Domains"),
    ("topic", "Topics"),
    ("project", "Projects"),
    ("source", "Sources"),
];
const REDIRECT_MAX_DEPTH: usize = 5;
pub const MENTIONS: &str = "## Mentions";
pub const RELATIONS: &str = "## Relations";

pub fn folder(ty: &str) -> Option<&'static str> {
    TYPES.iter().find(|(t, _)| *t == ty).map(|(_, f)| *f)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub name: String,
    pub ty: String,
    pub aliases: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub from: String,
    pub from_type: String,
    pub rel: String,
    pub to: String,
    pub to_type: String,
    /// Kept from the reply; the graph does not store it (05 §2.9).
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extraction {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

/// Entity rel path for (type, name); None for an unknown type.
pub fn entity_rel(ty: &str, name: &str) -> Option<String> {
    Some(format!("Claude/Graph/{}/{}.md", folder(ty)?, safe_filename(name)))
}

pub fn strip_md(p: &str) -> &str {
    p.strip_suffix(".md").unwrap_or(p)
}

/// File name without ".md".
pub fn stem(rel: &str) -> &str {
    strip_md(rel.rsplit('/').next().unwrap_or(rel))
}

/// Last path component minus its last extension.
fn py_stem(p: &str) -> &str {
    let name = p.rsplit('/').next().unwrap_or(p);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Truthy frontmatter value (05 §0).
pub fn truthy(v: Option<&Y>) -> bool {
    match v {
        None | Some(Y::Null) | Some(Y::Bool(false)) | Some(Y::Int(0)) => false,
        Some(Y::Float(f)) => *f != 0.0,
        Some(Y::Str(s)) => !s.is_empty(),
        Some(Y::Seq(s)) => !s.is_empty(),
        Some(Y::Map(m)) => !m.is_empty(),
        Some(_) => true,
    }
}

/// section(text, header) (05 §2.3).
pub fn section<'a>(text: &'a str, header: &str) -> &'a str {
    let Some(start) = text.find(header) else { return "" };
    let after = start + header.len();
    match text[after..].find("\n## ") {
        Some(i) => &text[start..after + i],
        None => &text[start..],
    }
}

pub fn section_lines<'a>(text: &'a str, header: &str) -> Vec<&'a str> {
    split_lines(section(text, header)).into_iter().filter(|l| l.starts_with("- ")).collect()
}

/// entity summary(text) (05 §2.3).
pub fn entity_summary(text: &str) -> String {
    let b = body(text);
    let b = b.find(MENTIONS).map_or(b, |i| &b[..i]);
    let kept: Vec<&str> = split_lines(b).into_iter().filter(|l| !l.starts_with("# ")).collect();
    strip(&kept.join("\n")).to_string()
}

pub static MENTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^- \[\[([^\]|]+)\|[^\]]+\]\](?:\s+[\x{2014}-]\s+(.*))?$").expect("static regex"));
pub static RELATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^- (\w+) \[\[([^\]|]+)\|[^\]]+\]\]$").expect("static regex"));

/// resolve_redirect(entity path) (05 §2.4).
pub fn resolve_redirect(v: &Vault, path: &str) -> Option<String> {
    let mut seen = BTreeSet::new();
    let mut p = path.to_string();
    for _ in 0..=REDIRECT_MAX_DEPTH {
        if !seen.insert(p.clone()) {
            return None;
        }
        let abs = v.resolve(&format!("{p}.md")).ok().filter(|a| a.is_file())?;
        let meta = parse(&read_lenient(&abs).ok()?).ok()?;
        if truthy(get(&meta, "retired")) {
            return None;
        }
        match get(&meta, "merged_into") {
            m if !truthy(m) => return Some(p),
            Some(m) => p = display(m),
            None => return None,
        }
    }
    None
}

/// find_entity_note(type, name) (05 §2.5).
pub fn find_entity_note(v: &Vault, ty: &str, name: &str) -> Result<Option<String>> {
    let Some(f) = folder(ty) else { return Ok(None) };
    let dir = v.resolve(&format!("Claude/Graph/{f}"))?;
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(None) };
    let mut files: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| crate::vault::is_md(n))
        .collect();
    files.sort();
    let (want, want_safe) = (casefold(name), casefold(&safe_filename(name)));
    for file in files {
        let text = read_lenient(&dir.join(&file))?;
        let meta = parse(&text)?;
        let st = casefold(strip_md(&file));
        let alias_hit = str_list(get(&meta, "aliases")).iter().any(|a| casefold(a) == want);
        if st != want && st != want_safe && !alias_hit {
            continue;
        }
        let rel = format!("Claude/Graph/{f}/{file}");
        if !truthy(get(&meta, "merged_into")) {
            return Ok(Some(rel));
        }
        let target = get(&meta, "merged_into").map(display).unwrap_or_default();
        return Ok(Some(resolve_redirect(v, &target).map_or(rel, |p| format!("{p}.md"))));
    }
    Ok(None)
}

/// The §2.2 new-note template.
pub fn template(e: &Entity, aliases: &[String], now: &NaiveDateTime) -> String {
    let s = |v: &str| Y::Str(v.to_string());
    let yaml = dump_block(&[
        (s("created"), s(&minute_stamp(now))),
        (s("agent"), s("claude")),
        (s("entity"), s(&e.ty)),
        (s("aliases"), Y::Seq(aliases.iter().map(|a| s(a)).collect())),
        (s("tags"), Y::Seq(vec![s(&format!("graph/{}", e.ty))])),
    ]);
    let mut out = format!("---\n{yaml}---\n\n# {}\n\n", e.name);
    if !e.summary.is_empty() {
        out.push_str(&format!("{}\n\n", e.summary));
    }
    out.push_str("## Mentions\n\n## Relations\n");
    out
}

/// Rewrites a note's frontmatter mapping in flow-leaf style, keeping the rest.
pub fn rewrite_frontmatter(text: &str, meta: &[(Y, Y)]) -> Option<String> {
    let end = text.get(3..)?.find("\n---")? + 3;
    Some(format!("---\n{}---{}", dump_flow_leaf(meta), &text[end + 4..]))
}

/// Sets key in a mapping, keeping order; a new key goes last.
pub fn set_key(meta: &mut Vec<(Y, Y)>, key: &str, value: Y) {
    match meta.iter_mut().find(|(k, _)| matches!(k, Y::Str(s) if s == key)) {
        Some(slot) => slot.1 = value,
        None => meta.push((Y::Str(key.into()), value)),
    }
}

/// merge_aliases(rel, candidates) (05 §2.6).
pub fn merge_aliases(v: &Vault, rel: &str, candidates: &[String]) -> Result<bool> {
    let text = v.read(rel)?;
    let mut meta = parse(&text)?;
    let current = str_list(get(&meta, "aliases"));
    let mut known: BTreeSet<String> = current.iter().map(|a| casefold(a)).collect();
    known.insert(casefold(stem(rel)));
    let added: Vec<String> = candidates.iter().filter(|c| !c.is_empty() && !known.contains(&casefold(c))).cloned().collect();
    if added.is_empty() {
        return Ok(false);
    }
    let all: Vec<Y> = current.into_iter().chain(added).map(Y::Str).collect();
    set_key(&mut meta, "aliases", Y::Seq(all));
    let Some(new) = rewrite_frontmatter(&text, &meta) else { return Ok(false) };
    v.write(rel, &new, true, false)?;
    Ok(true)
}

/// upsert_entity(entity) (05 §2.7) → (rel, created).
pub fn upsert_entity(v: &Vault, e: &Entity, now: &NaiveDateTime) -> Result<(String, bool)> {
    let Some(found) = find_entity_note(v, &e.ty, &e.name)? else {
        let rel = entity_rel(&e.ty, &e.name).ok_or_else(|| Error::invalid(format!("unknown entity type: {}", e.ty)))?;
        v.write(&rel, &template(e, &e.aliases, now), false, false)?;
        return Ok((rel, true));
    };
    let text = v.read(&found)?;
    let meta = parse(&text)?;
    if truthy(get(&meta, "retired")) {
        let mut taken: BTreeSet<String> = e.aliases.iter().map(|a| casefold(a)).collect();
        taken.insert(casefold(&e.name));
        let mut aliases = e.aliases.clone();
        aliases.extend(str_list(get(&meta, "aliases")).into_iter().filter(|a| !taken.contains(&casefold(a))));
        v.write(&found, &template(e, &aliases, now), true, false)?;
        return Ok((found, true));
    }
    let mut candidates = e.aliases.clone();
    if casefold(&e.name) != casefold(stem(&found)) {
        candidates.push(e.name.clone());
    }
    merge_aliases(v, &found, &candidates)?;
    Ok((found, false))
}

/// insert_line(rel, header, line, marker) (05 §2.8).
pub fn insert_line(v: &Vault, rel: &str, header: &str, line: &str, marker: &str) -> Result<bool> {
    let mut text = v.read(rel)?;
    if !text.contains(header) {
        text = format!("{}\n\n{header}\n", text.trim_end_matches(crate::text::is_ws));
    }
    if section(&text, header).contains(marker) {
        return Ok(false);
    }
    let h = text.find(header).unwrap_or(0) + header.len();
    let new = match text[h..].find("\n## ") {
        Some(i) => format!("{}\n{line}\n{}", text[..h + i].trim_end_matches(crate::text::is_ws), &text[h + i..]),
        None => format!("{}\n{line}\n", text.trim_end_matches(crate::text::is_ws)),
    };
    v.write(rel, &new, true, false)?;
    Ok(true)
}

pub fn add_mention(v: &Vault, entity_rel: &str, note: &str, evidence: &str) -> Result<bool> {
    let n = strip_md(note);
    let mut line = format!("- [[{n}|{}]]", stem(note));
    if !evidence.is_empty() {
        line.push_str(&format!(" \u{2014} {evidence}"));
    }
    insert_line(v, entity_rel, MENTIONS, &line, &format!("[[{n}|"))
}

pub fn add_relation(v: &Vault, src: &str, rel: &str, dst: &str) -> Result<bool> {
    let d = strip_md(dst);
    let line = format!("- {rel} [[{d}|{}]]", stem(dst));
    insert_line(v, src, RELATIONS, &line, &format!("- {rel} [[{d}|"))
}

pub fn remove_mention(v: &Vault, entity_rel: &str, note: &str) -> Result<bool> {
    let text = v.read(entity_rel)?;
    let marker = format!("[[{}|", strip_md(note));
    let lines = split_keep_ends(&text);
    let kept: String = lines.iter().filter(|l| !l.contains(&marker)).copied().collect();
    if kept.len() == text.len() {
        return Ok(false);
    }
    v.write(entity_rel, &kept, true, false)?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ApplyCounts {
    pub entities_created: i64,
    pub entities_merged: i64,
    pub mentions_added: i64,
    pub relations_added: i64,
}

/// apply(note path, extraction) (05 §2.9).
pub fn apply(v: &Vault, note: &str, x: &Extraction, now: &NaiveDateTime) -> Result<ApplyCounts> {
    let mut c = ApplyCounts::default();
    let mut memo: HashMap<(String, String), String> = HashMap::new();
    let mut ensure = |e: &Entity, c: &mut ApplyCounts| -> Result<String> {
        let key = (e.ty.clone(), casefold(&e.name));
        if let Some(rel) = memo.get(&key) {
            return Ok(rel.clone());
        }
        let (rel, created) = upsert_entity(v, e, now)?;
        if created {
            c.entities_created += 1;
        } else {
            c.entities_merged += 1;
        }
        memo.insert(key, rel.clone());
        Ok(rel)
    };
    for e in &x.entities {
        let rel = ensure(e, &mut c)?;
        if add_mention(v, &rel, note, &e.summary)? {
            c.mentions_added += 1;
        }
    }
    let bare = |name: &str, ty: &str| Entity { name: name.into(), ty: ty.into(), aliases: vec![], summary: String::new() };
    for r in &x.relations {
        let src = ensure(&bare(&r.from, &r.from_type), &mut c)?;
        let dst = ensure(&bare(&r.to, &r.to_type), &mut c)?;
        if add_relation(v, &src, &r.rel, &dst)? {
            c.relations_added += 1;
        }
    }
    Ok(c)
}

// ------------------------------------------------------------- graph.db

const SCHEMA: &str = "CREATE TABLE entities (path TEXT, name TEXT, type TEXT, summary TEXT, aliases TEXT);
CREATE TABLE edges (src_path TEXT, rel TEXT, dst_path TEXT);
CREATE TABLE mentions (entity_path TEXT, note_path TEXT, evidence TEXT);";

fn db_err(db: &Path) -> impl Fn(rusqlite::Error) -> Error + '_ {
    move |e| Error::new(Kind::IoError, format!("{e}: {}", db.display()))
}

/// rebuild(vault, db) (05 §3.2).
pub fn rebuild(v: &Vault, db: &Path) -> Result<()> {
    let dir = db.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| Error::io(&e, dir))?;
    let tmp = dir.join("graph.tmp");
    if tmp.exists() {
        std::fs::remove_file(&tmp).map_err(|e| Error::io(&e, &tmp))?;
    }
    let err = db_err(db);
    let mut conn = Connection::open(&tmp).map_err(&err)?;
    conn.execute_batch(SCHEMA).map_err(&err)?;
    let txn = conn.transaction().map_err(&err)?;
    let graph_dir = v.resolve("Claude/Graph")?;
    let notes = if graph_dir.is_dir() { crate::search::walk(v, Some("Claude/Graph"))? } else { vec![] };
    for (abs, rel) in notes {
        let text = read_lenient(&abs)?;
        let meta = parse(&text)?;
        if truthy(get(&meta, "merged_into")) || truthy(get(&meta, "retired")) {
            continue;
        }
        let path = strip_md(&rel).to_string();
        let ty = match get(&meta, "entity") {
            t if truthy(t) => display(t.unwrap_or(&Y::Null)),
            _ => "topic".into(),
        };
        let name = split_lines(&text)
            .into_iter()
            .find_map(|l| l.strip_prefix("# ").filter(|r| !r.is_empty()))
            .map_or_else(|| stem(&rel).to_string(), |n| strip(n).to_string());
        let after = text.find("\n# ").map_or(text.as_str(), |i| &text[i + 3..]);
        let summary = split_lines(after)
            .into_iter()
            .skip(1)
            .find(|l| !strip(l).is_empty() && !l.starts_with('#'))
            .map(|l| strip(l).to_string())
            .unwrap_or_default();
        let aliases = crate::json::compact(&str_list(get(&meta, "aliases")));
        txn.execute("INSERT INTO entities VALUES (?1, ?2, ?3, ?4, ?5)", params![path, name, ty, summary, aliases]).map_err(&err)?;
        for line in split_lines(section(&text, MENTIONS)) {
            if let Some(c) = MENTION_RE.captures(strip(line)) {
                let ev = c.get(2).map_or("", |m| m.as_str());
                txn.execute("INSERT INTO mentions VALUES (?1, ?2, ?3)", params![path, strip(&c[1]), ev]).map_err(&err)?;
            }
        }
        for line in split_lines(section(&text, RELATIONS)) {
            if let Some(c) = RELATION_RE.captures(strip(line)) {
                txn.execute("INSERT INTO edges VALUES (?1, ?2, ?3)", params![path, &c[1], strip(&c[2])]).map_err(&err)?;
            }
        }
    }
    txn.commit().map_err(&err)?;
    drop(conn);
    std::fs::rename(&tmp, db).map_err(|e| Error::io(&e, db))
}

pub struct EntityRow {
    pub path: String,
    pub name: String,
    pub ty: String,
    pub summary: String,
    pub aliases: Vec<String>,
}

/// The whole cache, read in rowid order.
pub struct Graph {
    pub entities: Vec<EntityRow>,
    pub edges: Vec<(String, String, String)>,
    pub mentions: Vec<(String, String, String)>,
}

fn opt(r: &rusqlite::Row, i: usize) -> rusqlite::Result<String> {
    Ok(r.get::<_, Option<String>>(i)?.unwrap_or_default())
}

impl Graph {
    pub fn load(db: &Path) -> Result<Graph> {
        let err = db_err(db);
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(&err)?;
        let triples = |sql: &str| -> Result<Vec<(String, String, String)>> {
            let mut st = conn.prepare(sql).map_err(&err)?;
            let rows = st.query_map([], |r| Ok((opt(r, 0)?, opt(r, 1)?, opt(r, 2)?))).map_err(&err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(&err)
        };
        let edges = triples("SELECT src_path, rel, dst_path FROM edges ORDER BY rowid")?;
        let mentions = triples("SELECT entity_path, note_path, evidence FROM mentions ORDER BY rowid")?;
        let mut st = conn.prepare("SELECT path, name, type, summary, aliases FROM entities ORDER BY rowid").map_err(&err)?;
        let rows = st
            .query_map([], |r| {
                Ok(EntityRow {
                    path: opt(r, 0)?,
                    name: opt(r, 1)?,
                    ty: opt(r, 2)?,
                    summary: opt(r, 3)?,
                    aliases: serde_json::from_str(&opt(r, 4)?).unwrap_or_default(),
                })
            })
            .map_err(&err)?;
        let entities = rows.collect::<rusqlite::Result<_>>().map_err(&err)?;
        Ok(Graph { entities, edges, mentions })
    }

    fn name_of(&self, path: &str) -> String {
        self.entities.iter().rev().find(|e| e.path == path).map_or_else(|| py_stem(path).to_string(), |e| e.name.clone())
    }

    pub fn record(&self, e: &EntityRow) -> Value {
        let relations: Vec<Value> =
            self.edges.iter().filter(|(s, _, _)| *s == e.path).map(|(_, rel, dst)| json!({"rel": rel, "to": self.name_of(dst)})).collect();
        json!({
            "name": e.name, "type": e.ty, "path": format!("{}.md", e.path), "summary": e.summary, "aliases": e.aliases,
            "relations": relations, "mention_count": self.mentions.iter().filter(|m| m.0 == e.path).count(),
        })
    }

    pub fn note_entity_paths(&self, note: &str) -> Vec<String> {
        let n = strip_md(note);
        self.mentions.iter().filter(|m| m.1 == n).map(|m| m.0.clone()).collect::<BTreeSet<_>>().into_iter().collect()
    }

    pub fn find(&self, query: &str, ty: Option<&str>) -> Vec<Value> {
        let q = casefold(query);
        self.entities
            .iter()
            .filter(|e| casefold(&e.name).contains(&q) || e.aliases.iter().any(|a| casefold(a).contains(&q)))
            .filter(|e| ty.is_none_or(|t| t.is_empty() || e.ty == t))
            .map(|e| self.record(e))
            .collect()
    }

    pub fn entities_at(&self, paths: &[String]) -> Vec<Value> {
        paths.iter().filter_map(|p| self.entities.iter().find(|e| e.path == *p)).map(|e| self.record(e)).collect()
    }

    pub fn stats(&self) -> Value {
        let mut types: BTreeMap<&str, i64> = BTreeMap::new();
        for e in &self.entities {
            *types.entry(&e.ty).or_insert(0) += 1;
        }
        let types: Map<String, Value> = types.into_iter().map(|(k, n)| (k.to_string(), json!(n))).collect();
        json!({"entities": types, "edges": self.edges.len(), "mentions": self.mentions.len()})
    }

    /// related_notes(path, hops, limit) (05 §3.4).
    pub fn related(&self, path: &str, hops: i64, limit: i64) -> Result<Vec<Value>> {
        if limit < 0 {
            return Err(Error::invalid(format!("limit must be >= 0, got {limit}")));
        }
        let lookup = strip_md(path);
        let mut reached: Vec<(String, usize, String)> = Vec::new(); // (entity, hop, chain)
        for m in self.mentions.iter().filter(|m| m.1 == lookup) {
            if !reached.iter().any(|r| r.0 == m.0) {
                reached.push((m.0.clone(), 1, self.name_of(&m.0)));
            }
        }
        let mut frontier: Vec<usize> = (0..reached.len()).collect();
        for hop in 2..=hops.max(1) as usize {
            let mut next = Vec::new();
            for &i in &frontier {
                let (ent, chain) = (reached[i].0.clone(), reached[i].2.clone());
                let mut neigh: BTreeSet<(String, String)> = BTreeSet::new();
                for (s, rel, d) in &self.edges {
                    if *s == ent {
                        neigh.insert((rel.clone(), d.clone()));
                    }
                    if *d == ent {
                        neigh.insert((rel.clone(), s.clone()));
                    }
                }
                for (rel, other) in neigh {
                    if reached.iter().any(|r| r.0 == other) {
                        continue;
                    }
                    let c = format!("{chain} ({rel}) {}", self.name_of(&other));
                    reached.push((other, hop, c));
                    next.push(reached.len() - 1);
                }
            }
            frontier = next;
        }
        let mut results: Vec<(String, f64, usize, String)> = Vec::new(); // (path, score, best hop, via)
        for (ent, hop, chain) in &reached {
            for m in self.mentions.iter().filter(|m| m.0 == *ent) {
                if m.1 == lookup || m.1.starts_with("Claude/Graph/") {
                    continue;
                }
                let p = format!("{}.md", m.1);
                match results.iter_mut().find(|r| r.0 == p) {
                    Some(r) => {
                        r.1 += 1.0 / *hop as f64;
                        if *hop < r.2 {
                            r.2 = *hop;
                            r.3 = chain.clone();
                        }
                    }
                    None => results.push((p, 1.0 / *hop as f64, *hop, chain.clone())),
                }
            }
        }
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        Ok(results.into_iter().take(limit as usize).map(|(p, _, _, via)| json!({"path": p, "via": via})).collect())
    }
}

/// The graph cache path, or the "not built" VaultError.
pub fn require_db(state_dir: &Path) -> Result<std::path::PathBuf> {
    let db = state_dir.join("graph.db");
    if db.exists() { Ok(db) } else { Err(Error::vault("Graph cache not built yet \u{2014} run index_brain first.")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_and_summary() {
        let t = "# Foo\n\nLine one.\n\n---\n\nLine two.\n\n## Mentions\n- a\n\n## Relations\n- b\n";
        assert_eq!(entity_summary(t), "Line one.\n\n---\n\nLine two.");
        assert_eq!(section(t, MENTIONS), "## Mentions\n- a\n");
        assert_eq!(section_lines(t, RELATIONS), ["- b"]);
        assert_eq!(section(t, "## Nope"), "");
    }

    #[test]
    fn line_grammar() {
        let c = MENTION_RE.captures("- [[Projects/X|X]] \u{2014} it was there").unwrap();
        assert_eq!((&c[1], &c[2]), ("Projects/X", "it was there"));
        assert!(MENTION_RE.captures("- [[Projects/X|X]]").unwrap().get(2).is_none());
        let r = RELATION_RE.captures("- works_at [[Claude/Graph/Organizations/Acme|Acme]]").unwrap();
        assert_eq!((&r[1], &r[2]), ("works_at", "Claude/Graph/Organizations/Acme"));
    }
}
