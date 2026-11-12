//! Graph deletion and orphan cleanup (12).
use crate::error::{Kind, Result};
use crate::graph::{self, Graph, MENTIONS, RELATIONS, RELATION_RE, resolve_redirect, section_lines, stem, strip_md, truthy};
use crate::indexer::{Manifest, scan_notes};
use crate::text::{display, minute_stamp, split_keep_ends, strip};
use crate::vault::{Vault, get, parse, read_lenient};
use crate::yaml::{Y, dump_flow_leaf};
use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub const MAX_RETRACTIONS: usize = 100;
pub const MAX_RELATION_FIXES: i64 = 200;
pub const MAX_PENDING_RETIREMENTS: usize = 200;
const MASS_DELETION_MIN: usize = 20;
const MASS_DELETION_MAX_FRACTION: f64 = 0.10;
pub const RETIRE_ORPHAN: &str = "orphaned \u{2014} no mentions or relations";
pub const RETIRE_DEAD_STUB: &str = "merge redirect target gone";
const ORPHAN_REASON: &str = "orphaned: no mentions or relations";

fn tracked(m: &Manifest) -> BTreeSet<String> {
    m.hashes.keys().chain(m.failures.keys()).cloned().collect()
}

/// deleted_notes (12 §2) → (sorted deleted paths, tracked count).
pub fn deleted_notes(v: &Vault, state_dir: &Path) -> Result<(Vec<String>, usize)> {
    let t = tracked(&Manifest::load(state_dir)?);
    let scan = scan_notes(v)?;
    Ok((t.iter().filter(|p| !scan.contains_key(*p)).cloned().collect(), t.len()))
}

pub fn suspect(deleted: usize, tracked: usize) -> bool {
    deleted >= MASS_DELETION_MIN && deleted as f64 > MASS_DELETION_MAX_FRACTION * tracked as f64
}

fn graph_notes(v: &Vault) -> Result<Vec<(std::path::PathBuf, String)>> {
    if !v.resolve("Claude/Graph")?.is_dir() {
        return Ok(vec![]);
    }
    crate::search::walk(v, Some("Claude/Graph"))
}

/// retract_deleted(limit, force) (12 §3).
pub fn retract_deleted(v: &Vault, state_dir: &Path, limit: usize, force: bool) -> Result<Value> {
    let (deleted, n_tracked) = deleted_notes(v, state_dir)?;
    if !force && suspect(deleted.len(), n_tracked) {
        return Ok(json!({"retracted_notes": 0, "removed_mentions": 0, "remaining": deleted.len(), "suspect_mass_deletion": true}));
    }
    let todo: Vec<&String> = deleted.iter().take(limit).collect();
    let db = state_dir.join("graph.db");
    let g = if db.exists() { Some(Graph::load(&db)?) } else { None };
    let mut m = Manifest::load(state_dir)?;
    let mut removed = 0;
    for note in &todo {
        let entities: Vec<String> = match &g {
            Some(g) => g.note_entity_paths(note),
            None => {
                let marker = format!("[[{}|", strip_md(note));
                let mut found = Vec::new();
                for (abs, rel) in graph_notes(v)? {
                    if read_lenient(&abs)?.contains(&marker) {
                        found.push(strip_md(&rel).to_string());
                    }
                }
                found.sort();
                found
            }
        };
        for e in entities {
            match graph::remove_mention(v, &format!("{e}.md"), note) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(err) if err.is(Kind::VaultError) => {}
                Err(err) => return Err(err),
            }
        }
        m.hashes.shift_remove(*note);
        m.failures.shift_remove(*note);
    }
    if !todo.is_empty() {
        m.save(state_dir)?;
    }
    Ok(json!({"retracted_notes": todo.len(), "removed_mentions": removed, "remaining": deleted.len() - todo.len()}))
}

#[derive(Clone, PartialEq)]
enum Target {
    Gone,
    Stub(Option<String>),
    Live,
}

fn status(v: &Vault, path: &str) -> Target {
    let Ok(abs) = v.resolve(&format!("{path}.md")) else { return Target::Gone };
    if !abs.is_file() {
        return Target::Gone;
    }
    let Ok(meta) = read_lenient(&abs).and_then(|t| parse(&t)) else { return Target::Gone };
    if truthy(get(&meta, "retired")) {
        Target::Gone
    } else if truthy(get(&meta, "merged_into")) {
        Target::Stub(resolve_redirect(v, path))
    } else {
        Target::Live
    }
}

fn frozen(meta: &[(Y, Y)]) -> bool {
    truthy(get(meta, "merged_into")) || truthy(get(meta, "retired"))
}

fn last_segment(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// repair_relations(limit, dry) (12 §4.1) → (fixed, removed).
pub fn repair_relations(v: &Vault, limit: i64, dry: bool) -> Result<(i64, i64)> {
    let (mut fixed, mut removed) = (0i64, 0i64);
    let mut memo: HashMap<String, Target> = HashMap::new();
    for (abs, rel) in graph_notes(v)? {
        if fixed + removed >= limit {
            break;
        }
        let text = read_lenient(&abs)?;
        if frozen(&parse(&text)?) {
            continue;
        }
        let lines = split_keep_ends(&text);
        let mut out: Vec<String> = Vec::new();
        let mut changed = false;
        for line in &lines {
            let caps = RELATION_RE.captures(strip(line));
            let Some(c) = caps.filter(|_| fixed + removed < limit) else {
                out.push(line.to_string());
                continue;
            };
            let target = strip(&c[2]).to_string();
            let st = memo.entry(target.clone()).or_insert_with(|| status(v, &target)).clone();
            match st {
                Target::Live => out.push(line.to_string()),
                Target::Stub(Some(canon)) => {
                    let new = format!("- {} [[{canon}|{}]]\n", &c[1], last_segment(&canon));
                    if out.contains(&new) || lines.contains(&new.as_str()) {
                        removed += 1;
                    } else {
                        out.push(new);
                        fixed += 1;
                    }
                    changed = true;
                }
                Target::Stub(None) | Target::Gone => {
                    removed += 1;
                    changed = true;
                }
            }
        }
        if changed && !dry {
            v.write(&rel, &out.concat(), true, false)?;
        }
    }
    Ok((fixed, removed))
}

/// retire_note(rel, now, reason) (12 §5).
pub fn retire_note(v: &Vault, rel: &str, now: &NaiveDateTime, reason: &str) -> Result<()> {
    let text = v.read(rel)?;
    let mut meta = parse(&text)?;
    graph::set_key(&mut meta, "retired", Y::Str(minute_stamp(now)));
    let summary = graph::entity_summary(&text);
    let mut out = format!("---\n{}---\n\n# {}\n\n", dump_flow_leaf(&meta), stem(rel));
    if !summary.is_empty() {
        out.push_str(&format!("{summary}\n\n"));
    }
    out.push_str(&format!("Retired: {reason}.\n"));
    v.write(rel, &out, true, false)?;
    Ok(())
}

/// flatten_stubs(now, dry) (12 §4.2) → (flattened, retired_stubs).
pub fn flatten_stubs(v: &Vault, now: &NaiveDateTime, dry: bool) -> Result<(i64, i64)> {
    let (mut flattened, mut retired) = (0, 0);
    for (abs, rel) in graph_notes(v)? {
        let text = read_lenient(&abs)?;
        let mut meta = parse(&text)?;
        if !truthy(get(&meta, "merged_into")) || truthy(get(&meta, "retired")) {
            continue;
        }
        let target = get(&meta, "merged_into").map(display).unwrap_or_default();
        if status(v, &target) == Target::Live {
            continue;
        }
        let fin = resolve_redirect(v, &target);
        match (fin, dry) {
            (Some(_), true) => flattened += 1,
            (None, true) => retired += 1,
            (Some(f), false) => {
                graph::set_key(&mut meta, "merged_into", Y::Str(f.clone()));
                let out = format!("---\n{}---\n\n# {}\n\nMerged into [[{}]].\n", dump_flow_leaf(&meta), stem(&rel), last_segment(&f));
                v.write(&rel, &out, true, false)?;
                flattened += 1;
            }
            (None, false) => {
                retire_note(v, &rel, now, RETIRE_DEAD_STUB)?;
                retired += 1;
            }
        }
    }
    Ok((flattened, retired))
}

/// find_orphans (12 §6).
pub fn find_orphans(v: &Vault) -> Result<Vec<Value>> {
    let mut inbound: BTreeSet<String> = BTreeSet::new();
    let mut cands: Vec<(String, String, String, bool)> = Vec::new();
    for (abs, rel) in graph_notes(v)? {
        let text = read_lenient(&abs)?;
        for line in section_lines(&text, RELATIONS) {
            if let Some(c) = RELATION_RE.captures(strip(line)) {
                inbound.insert(strip(&c[2]).to_string());
            }
        }
        let meta = parse(&text)?;
        if frozen(&meta) {
            continue;
        }
        let ty = match get(&meta, "entity") {
            t if truthy(t) => display(t.unwrap_or(&Y::Null)),
            _ => "topic".into(),
        };
        let supported = !section_lines(&text, MENTIONS).is_empty() || !section_lines(&text, RELATIONS).is_empty();
        cands.push((strip_md(&rel).to_string(), stem(&rel).to_string(), ty, supported));
    }
    Ok(cands
        .into_iter()
        .filter(|(p, _, _, supported)| !supported && !inbound.contains(p))
        .map(|(path, name, ty, _)| json!({"path": path, "name": name, "type": ty, "reason": ORPHAN_REASON}))
        .collect())
}

/// update_retirement_proposals(block, orphans, limit) (12 §6).
pub fn update_retirement_proposals(block: &mut Map<String, Value>, orphans: &[Value], limit: usize) -> Vec<Value> {
    let live: BTreeSet<&str> = orphans.iter().filter_map(|o| o["path"].as_str()).collect();
    let mut pending: Vec<Value> = block
        .get("pending_retirements")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|p| p["path"].as_str().is_some_and(|x| live.contains(x))).cloned().collect())
        .unwrap_or_default();
    for o in orphans {
        if pending.len() >= limit {
            break;
        }
        if !pending.iter().any(|p| p["path"] == o["path"]) {
            pending.push(o.clone());
        }
    }
    block.insert("pending_retirements".into(), Value::Array(pending.clone()));
    pending
}

/// apply_retirements(paths, now) (12 §6).
pub fn apply_retirements(v: &Vault, state_dir: &Path, paths: Option<&[String]>, now: &NaiveDateTime) -> Result<Value> {
    let mut retired: Vec<String> = Vec::new();
    for o in find_orphans(v)? {
        let p = o["path"].as_str().unwrap_or_default().to_string();
        if paths.is_some_and(|keep| !keep.contains(&p)) {
            continue;
        }
        retire_note(v, &format!("{p}.md"), now, RETIRE_ORPHAN)?;
        retired.push(p);
    }
    if !retired.is_empty() {
        graph::rebuild(v, &state_dir.join("graph.db"))?;
    }
    retired.sort();
    Ok(json!({"retired": retired}))
}

/// apply_repairs(now, force) (12 §7).
pub fn apply_repairs(v: &Vault, state_dir: &Path, now: &NaiveDateTime, force: bool) -> Result<Value> {
    let ret = retract_deleted(v, state_dir, MAX_RETRACTIONS, force)?;
    let (fixed, removed) = repair_relations(v, MAX_RELATION_FIXES, false)?;
    let (flattened, retired_stubs) = flatten_stubs(v, now, false)?;
    let n = |k: &str| ret[k].as_i64().unwrap_or(0);
    if n("removed_mentions") + n("retracted_notes") + fixed + removed + flattened + retired_stubs > 0 {
        graph::rebuild(v, &state_dir.join("graph.db"))?;
    }
    Ok(json!({
        "retract": ret,
        "relations": {"fixed": fixed, "removed": removed},
        "stubs": {"flattened": flattened, "retired_stubs": retired_stubs},
    }))
}

/// prune_checked_hash(block, live) (12 §8).
pub fn prune_checked_hash(block: &mut Map<String, Value>, live: &BTreeSet<String>) -> usize {
    let Some(Value::Object(checked)) = block.get_mut("checked_hash") else { return 0 };
    let before = checked.len();
    checked.retain(|k, _| live.contains(k));
    before - checked.len()
}

/// The report-only CLI view: {deleted_notes, orphans}.
pub fn report(v: &Vault, state_dir: &Path) -> Result<Value> {
    Ok(json!({"deleted_notes": deleted_notes(v, state_dir)?.0, "orphans": find_orphans(v)?}))
}
