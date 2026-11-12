//! Entity consolidation and candidate blocking (07).
use crate::embed::Embedder;
use crate::error::Result;
use crate::extractor::{Complete, ENTITY_TYPES, json_to_y};
use crate::graph::{self, MENTIONS, RELATIONS, entity_summary, find_entity_note, folder, section_lines, stem, strip_md, truthy};
use crate::search::{Cache, Entry, cosine, load_cache, save_cache};
use crate::text::{casefold, display, minute_stamp, strip};
use crate::vault::{Vault, get, parse, read_lenient, str_list};
use crate::yaml::{Y, dump_block};
use chrono::NaiveDateTime;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

const SIM_THRESHOLD: f64 = 0.85;
pub const K_NEIGHBORS: usize = 5;
const MAX_CLUSTER: usize = 10;
pub const SLICE_SIZE: usize = 200;
pub const MAX_ENTITIES_PER_CALL: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub struct Ent {
    pub name: String,
    pub ty: String,
    pub path: String,
    pub aliases: Vec<String>,
    pub summary: String,
}

/// gather_entities(vault) (07 §1).
pub fn gather(v: &Vault) -> Result<Vec<Ent>> {
    if !v.resolve("Claude/Graph")?.is_dir() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for (abs, rel) in crate::search::walk(v, Some("Claude/Graph"))? {
        let text = read_lenient(&abs)?;
        let meta = parse(&text)?;
        if truthy(get(&meta, "merged_into")) || truthy(get(&meta, "retired")) {
            continue;
        }
        let ty = match get(&meta, "entity") {
            t if truthy(t) => display(t.unwrap_or(&Y::Null)),
            _ => "topic".into(),
        };
        out.push(Ent {
            name: stem(&rel).to_string(),
            ty,
            path: strip_md(&rel).to_string(),
            aliases: str_list(get(&meta, "aliases")),
            summary: entity_summary(&text),
        });
    }
    Ok(out)
}

pub fn identity_text(e: &Ent) -> String {
    strip(&format!("{}\n{}\n{}", e.name, e.aliases.join(", "), e.summary)).to_string()
}

pub fn identity_hash(e: &Ent) -> String {
    crate::search::content_hash(&identity_text(e))
}

pub fn vectors_path(state_dir: &Path) -> PathBuf {
    state_dir.join("entity_vectors.json")
}

/// compute_entity_vectors (07 §2.2): one embed call for all stale entities.
pub fn compute_entity_vectors(state_dir: &Path, ents: &[Ent], emb: &dyn Embedder, persist: bool) -> Result<HashMap<String, Vec<f64>>> {
    let path = vectors_path(state_dir);
    let mut cache = load_cache(&path);
    let mut out = HashMap::new();
    let mut todo: Vec<(&Ent, String, String)> = Vec::new();
    for e in ents {
        let h = identity_hash(e);
        match cache.get(&e.path).filter(|c| c.hash == h) {
            Some(c) => {
                out.insert(e.path.clone(), c.vec.clone());
            }
            None => todo.push((e, h, identity_text(e))),
        }
    }
    if !todo.is_empty() {
        let texts: Vec<&str> = todo.iter().map(|(_, _, t)| t.as_str()).collect();
        for ((e, hash, _), vec) in todo.iter().zip(emb.embed(&texts)?) {
            cache.insert(e.path.clone(), Entry { hash: hash.clone(), vec: vec.clone() });
            out.insert(e.path.clone(), vec);
        }
        if persist {
            save_cache(&path, &cache)?;
        }
    }
    Ok(out)
}

/// prune_entity_vectors(live paths) (07 §2.2).
pub fn prune_entity_vectors(state_dir: &Path, live: &[String]) -> Result<usize> {
    let path = vectors_path(state_dir);
    let cache = load_cache(&path);
    let kept: Cache = cache.iter().filter(|(k, _)| live.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let pruned = cache.len() - kept.len();
    if pruned > 0 {
        save_cache(&path, &kept)?;
    }
    Ok(pruned)
}

/// Candidate pairs (07 §2.3); k is K_NEIGHBORS outside tests.
pub fn candidate_pairs(slice: &[Ent], all: &[Ent], vecs: &HashMap<String, Vec<f64>>, k: usize) -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    for s in slice {
        let Some(sv) = vecs.get(&s.path) else { continue };
        let mut near: Vec<(f64, &str)> = all
            .iter()
            .filter(|o| o.ty == s.ty && o.path != s.path)
            .filter_map(|o| vecs.get(&o.path).map(|ov| (cosine(sv, ov), o.path.as_str())))
            .filter(|(c, _)| *c >= SIM_THRESHOLD)
            .collect();
        near.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.1.cmp(a.1)));
        for (_, p) in near.into_iter().take(k) {
            let (a, b) = if s.path.as_str() < p { (s.path.clone(), p.to_string()) } else { (p.to_string(), s.path.clone()) };
            pairs.insert((a, b));
        }
    }
    pairs
}

/// Clusters of paths (07 §2.4 steps 1–3).
pub fn clusters(pairs: &BTreeSet<(String, String)>) -> Vec<Vec<String>> {
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    fn find(p: &mut BTreeMap<String, String>, x: &str) -> String {
        let up = p.get(x).cloned().unwrap_or_else(|| x.to_string());
        if up == x {
            return up;
        }
        let root = find(p, &up);
        p.insert(x.to_string(), root.clone());
        root
    }
    for (a, b) in pairs {
        parent.entry(a.clone()).or_insert_with(|| a.clone());
        parent.entry(b.clone()).or_insert_with(|| b.clone());
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
        }
    }
    let keys: Vec<String> = parent.keys().cloned().collect();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for k in keys {
        let r = find(&mut parent, &k);
        groups.entry(r).or_default().push(k);
    }
    let mut groups: Vec<Vec<String>> = groups.into_values().map(|mut g| {
        g.sort();
        g
    }).collect();
    groups.sort_by(|a, b| a[0].cmp(&b[0]));
    let mut out = Vec::new();
    for g in groups {
        let n = g.len();
        if n <= MAX_CLUSTER {
            out.push(g);
            continue;
        }
        let k = n.div_ceil(MAX_CLUSTER);
        let (base, extra) = (n / k, n % k);
        let mut it = g.into_iter();
        for i in 0..k {
            out.push(it.by_ref().take(base + usize::from(i < extra)).collect());
        }
    }
    out
}

/// candidate_clusters (07 §2.4).
pub fn candidate_clusters(slice: &[Ent], all: &[Ent], vecs: &HashMap<String, Vec<f64>>) -> Vec<Vec<Ent>> {
    clusters(&candidate_pairs(slice, all, vecs, K_NEIGHBORS))
        .into_iter()
        .map(|c| c.iter().filter_map(|p| all.iter().find(|e| e.path == *p).cloned()).collect::<Vec<_>>())
        .filter(|c| c.len() >= 2)
        .collect()
}

/// batch_clusters (07 §2.5).
pub fn batch_clusters(clusters: Vec<Vec<Ent>>, cap: usize) -> Vec<Vec<Ent>> {
    let mut out: Vec<Vec<Ent>> = Vec::new();
    let mut cur: Vec<Ent> = Vec::new();
    for c in clusters {
        if !cur.is_empty() && cur.len() + c.len() > cap {
            out.push(std::mem::take(&mut cur));
        }
        cur.extend(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// select_slice (07 §2.6) → (slice, new cursor, used_backstop).
pub fn select_slice(
    ents: &[Ent],
    checked: &HashMap<String, String>,
    cursor: Option<&str>,
    size: usize,
    backstop_due: bool,
) -> (Vec<Ent>, Option<String>, bool) {
    let mut sorted = ents.to_vec();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut slice: Vec<Ent> =
        sorted.iter().filter(|e| checked.get(&e.path) != Some(&identity_hash(e))).take(size).cloned().collect();
    let (mut new_cursor, mut used) = (cursor.map(String::from), false);
    if backstop_due && slice.len() < size {
        let start = cursor.map_or(0, |c| sorted.iter().position(|e| e.path.as_str() > c).unwrap_or(sorted.len()));
        for e in sorted[start..].iter().chain(&sorted[..start]) {
            if slice.len() >= size {
                break;
            }
            if slice.iter().any(|s| s.path == e.path) {
                continue;
            }
            slice.push(e.clone());
            new_cursor = Some(e.path.clone());
            used = true;
        }
    }
    (slice, new_cursor, used)
}

/// The adjudication prompt (07 §3.1), byte for byte.
pub const PROMPT: &str = include_str!("prompts/consolidation.txt");

pub fn prompt(batch: &[Ent]) -> String {
    let listing: Vec<String> = batch
        .iter()
        .map(|e| format!("{} | {} | {}", e.ty, e.name, if e.aliases.is_empty() { "-".to_string() } else { e.aliases.join(", ") }))
        .collect();
    let (head, tail) = PROMPT.split_once("{listing}").expect("template has {listing}");
    [head, &listing.join("\n"), tail].concat()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Merge {
    pub ty: String,
    pub canonical: String,
    pub duplicates: Vec<String>,
}

fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => true,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(Value::Bool(true)) => false,
    }
}

fn shown(v: Option<&Value>) -> String {
    if falsy(v) { String::new() } else { display(&json_to_y(v.unwrap_or(&Value::Null))) }
}

/// Validating merges (07 §3.2).
pub fn validate(reply: &Value, all: &[Ent]) -> Vec<Merge> {
    let known: BTreeSet<(String, String)> = all.iter().map(|e| (e.ty.clone(), casefold(&e.name))).collect();
    let items = match reply.get("merges") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_object).collect(),
        _ => vec![],
    };
    let mut out = Vec::new();
    for m in items {
        let ty = shown(m.get("type")).to_lowercase();
        let canonical = strip(&shown(m.get("canonical"))).to_string();
        let cf = casefold(&canonical);
        let raw: Vec<String> = match m.get("duplicates") {
            v if falsy(v) => vec![],
            Some(Value::String(s)) => s.chars().map(String::from).collect(),
            Some(Value::Array(a)) => a.iter().map(|d| display(&json_to_y(d))).collect(),
            Some(other) => vec![display(&json_to_y(other))],
            None => vec![],
        };
        let duplicates: Vec<String> =
            raw.iter().map(|d| strip(d).to_string()).filter(|d| !d.is_empty() && casefold(d) != cf).collect();
        let ok = !canonical.is_empty()
            && !duplicates.is_empty()
            && ENTITY_TYPES.contains(&ty.as_str())
            && known.contains(&(ty.clone(), cf.clone()))
            && duplicates.iter().all(|d| known.contains(&(ty.clone(), casefold(d))));
        if ok {
            out.push(Merge { ty, canonical, duplicates });
        }
    }
    out
}

fn merge_id(m: &Merge) -> (String, String, Vec<String>) {
    let mut d: Vec<String> = m.duplicates.iter().map(|x| casefold(x)).collect();
    d.sort();
    (m.ty.clone(), casefold(&m.canonical), d)
}

/// adjudicate_batches (07 §3.3) → (merges, skipped).
pub fn adjudicate(backend: &dyn Complete, batches: &[Vec<Ent>], all: &[Ent]) -> (Vec<Merge>, i64) {
    let (mut merges, mut skipped) = (Vec::<Merge>::new(), 0);
    for b in batches {
        match backend.complete_json(&prompt(b)) {
            Ok(reply) => {
                for m in validate(&reply, all) {
                    if !merges.iter().any(|k| merge_id(k) == merge_id(&m)) {
                        merges.push(m);
                    }
                }
            }
            Err(_) => skipped += 1,
        }
    }
    (merges, skipped)
}

fn dup_note(v: &Vault, ty: &str, d: &str) -> Result<Option<String>> {
    let live = |rel: &str| -> Result<Option<String>> {
        let meta = parse(&v.read(rel)?)?;
        Ok((!truthy(get(&meta, "merged_into")) && !truthy(get(&meta, "retired"))).then(|| rel.to_string()))
    };
    if let Some(rel) = graph::entity_rel(ty, d)
        && v.resolve(&rel)?.is_file()
    {
        return live(&rel);
    }
    let Some(f) = folder(ty) else { return Ok(None) };
    let Ok(rd) = std::fs::read_dir(v.resolve(&format!("Claude/Graph/{f}"))?) else { return Ok(None) };
    let mut files: Vec<String> =
        rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| crate::vault::is_md(n)).collect();
    files.sort();
    match files.into_iter().find(|n| casefold(strip_md(n)) == casefold(d)) {
        Some(n) => live(&format!("Claude/Graph/{f}/{n}")),
        None => Ok(None),
    }
}

/// Applying a merge (07 §4).
pub fn apply_merge(v: &Vault, m: &Merge, now: &NaiveDateTime) -> Result<()> {
    let canon = find_entity_note(v, &m.ty, &m.canonical)?;
    for d in &m.duplicates {
        let dup = dup_note(v, &m.ty, d)?;
        let (Some(canon), Some(dup)) = (canon.as_deref(), dup) else { continue };
        if dup == canon {
            continue;
        }
        let text = v.read(&dup)?;
        for line in section_lines(&text, MENTIONS) {
            let marker = match line.find('|') {
                Some(i) => format!("{}|", &line[..i]),
                None => format!("{line}|"),
            };
            graph::insert_line(v, canon, MENTIONS, line, &marker)?;
        }
        for line in section_lines(&text, RELATIONS) {
            graph::insert_line(v, canon, RELATIONS, line, line)?;
        }
        let mut cands = vec![d.clone()];
        cands.extend(str_list(get(&parse(&text)?, "aliases")));
        graph::merge_aliases(v, canon, &cands)?;
        let s = |x: &str| Y::Str(x.to_string());
        let yaml = dump_block(&[
            (s("created"), s(&minute_stamp(now))),
            (s("agent"), s("claude")),
            (s("entity"), s(&m.ty)),
            (s("merged_into"), s(strip_md(canon))),
            (s("tags"), Y::Seq(vec![s(&format!("graph/{}", m.ty))])),
        ]);
        v.write(&dup, &format!("---\n{yaml}---\n\n# {d}\n\nMerged into [[{}]].\n", stem(canon)), true, false)?;
    }
    Ok(())
}

/// run (07 §5).
pub fn run(v: &Vault, state_dir: &Path, backend: &dyn Complete, apply: bool, emb: &dyn Embedder, now: &NaiveDateTime) -> Result<Value> {
    let ents = gather(v)?;
    let mut result = json!({"entities": ents.len(), "proposed": [], "applied": false, "merged_entities": 0, "skipped_batches": 0});
    if ents.is_empty() {
        return Ok(result);
    }
    let vecs = compute_entity_vectors(state_dir, &ents, emb, apply)?;
    let batches = batch_clusters(candidate_clusters(&ents, &ents, &vecs), MAX_ENTITIES_PER_CALL);
    let (merges, skipped) = adjudicate(backend, &batches, &ents);
    result["proposed"] = merges.iter().map(|m| json!({"type": m.ty, "canonical": m.canonical, "duplicates": m.duplicates})).collect();
    result["skipped_batches"] = json!(skipped);
    if apply && !merges.is_empty() {
        let mut merged = 0;
        for m in &merges {
            apply_merge(v, m, now)?;
            merged += m.duplicates.len();
        }
        result["merged_entities"] = json!(merged);
        result["applied"] = json!(true);
        graph::rebuild(v, &state_dir.join("graph.db"))?;
    }
    Ok(result)
}
