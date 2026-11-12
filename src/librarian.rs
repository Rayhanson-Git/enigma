//! The Librarian caretaker sweep (08).
use crate::cleanup;
use crate::consolidate::{self, MAX_ENTITIES_PER_CALL, SLICE_SIZE};
use crate::embed::Embedder;
use crate::error::{Error, Kind, Result};
use crate::extractor::{Cli, Complete, Extract, json_to_y};
use crate::graph::{self, Graph};
use crate::indexer::{self, Manifest, Options};
use crate::mover::second_stamp;
use crate::text::{minute_stamp, quoted};
use crate::vault::Vault;
use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub const STATE_FILE: &str = "librarian_state.json";
pub const REPORT_NOTE: &str = "Claude/Librarian.md";
pub const REPORT_SEED: &str = "# Librarian\n\nCaretaker sweep reports (newest last). See constitution.\n";
pub const BACKSTOP_MIN_INTERVAL_DAYS: i64 = 14;
pub const REPORT_MAX_SWEEPS: usize = 30;
pub const MAX_INDEX_ROUNDS: usize = 40;

/// What a sweep runs with; None means "build the configured CLI inside the step".
pub struct Deps<'a> {
    pub extractor: Option<&'a dyn Extract>,
    pub consolidator: Option<&'a dyn Complete>,
    pub emb: &'a dyn Embedder,
}

pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_FILE)
}

/// status() (08 §2): the stored state, or a marker when none is readable.
pub fn status(state_dir: &Path) -> Result<Value> {
    let p = state_path(state_dir);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"status": "no sweep yet"})),
        Err(e) => return Err(Error::io(&e, &p)),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => Ok(v),
        Err(e) => Ok(json!({"status": "state file unreadable", "error": e.to_string(), "path": p.to_string_lossy()})),
    }
}

fn default_state() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [("last_sweep", Value::Null), ("steps", json!({})), ("health", json!({})), ("errors", json!({})), ("consolidation", json!({}))] {
        m.insert(k.into(), v);
    }
    m
}

/// Load (08 §2): absent → default; unparseable → InvalidData.
pub fn load_state(state_dir: &Path) -> Result<Map<String, Value>> {
    let p = state_path(state_dir);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default_state()),
        Err(e) => return Err(Error::io(&e, &p)),
    };
    serde_json::from_slice::<Map<String, Value>>(&bytes)
        .map_err(|e| Error::new(Kind::InvalidData, format!("unreadable Librarian state {}: {e}", p.display())))
}

/// Save (16 §7): indented File JSON through a temp file and an atomic rename.
pub fn save_state(state_dir: &Path, state: &Map<String, Value>) -> Result<()> {
    let p = state_path(state_dir);
    let tmp = state_dir.join(format!("{STATE_FILE}.tmp"));
    std::fs::write(&tmp, crate::json::indented(state))
        .and_then(|_| crate::faults::os("state_rename"))
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::io(&e, &p)
        })
}

fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn index_step(v: &Vault, st: &Path, deps: &Deps, apply: bool, now: &NaiveDateTime) -> Result<Value> {
    if !apply {
        let pending = indexer::pending_notes(&Manifest::load(st)?, &indexer::scan_notes(v)?).0.len();
        return Ok(json!({"pending": pending}));
    }
    let owned;
    let ex: &dyn Extract = match deps.extractor {
        Some(e) => e,
        None => {
            owned = Cli::extraction(None)?;
            &owned
        }
    };
    let mut totals: Option<Map<String, Value>> = None;
    for _ in 0..MAX_INDEX_ROUNDS {
        let c = serde_json::to_value(indexer::run(v, st, ex, Some(deps.emb), &Options::default(), now)?).unwrap_or_default();
        let round = c.as_object().cloned().unwrap_or_default();
        let t = match totals.take() {
            None => round,
            Some(mut t) => {
                for (k, val) in round {
                    let next = if k == "remaining" || k == "skipped" { val } else { json!(n(&Value::Object(t.clone()), &k) + val.as_i64().unwrap_or(0)) };
                    t.insert(k, next);
                }
                t
            }
        };
        if n(&c, "remaining") == 0 {
            return Ok(Value::Object(t));
        }
        totals = Some(t);
    }
    let remaining = totals.as_ref().map_or(0, |t| n(&Value::Object(t.clone()), "remaining"));
    Err(Error::new(Kind::Internal, format!("index did not drain after {MAX_INDEX_ROUNDS} rounds; {remaining} notes still pending")))
}

fn cache_step(v: &Vault, st: &Path, apply: bool, index: Option<&Value>, organize: Option<&Value>) -> Result<Value> {
    if index.is_some_and(|i| n(i, "processed") > 0) {
        return Ok(json!({"rebuilt": true, "by": "index"}));
    }
    if organize.is_some_and(|o| o["cache_rebuilt"] == json!(true)) {
        return Ok(json!({"rebuilt": true, "by": "organize"}));
    }
    let db = st.join("graph.db");
    if !db.exists() {
        if !apply {
            return Ok(json!({"rebuilt": false, "by": "none"}));
        }
        graph::rebuild(v, &db)?;
        return Ok(json!({"rebuilt": true, "by": "librarian"}));
    }
    Ok(json!({"rebuilt": false, "by": "none"}))
}

fn pending_repairs(v: &Vault, st: &Path, now: &NaiveDateTime) -> Result<Value> {
    let (deleted, tracked) = cleanup::deleted_notes(v, st)?;
    let (f, r) = cleanup::repair_relations(v, cleanup::MAX_RELATION_FIXES, true)?;
    let (fl, rs) = cleanup::flatten_stubs(v, now, true)?;
    Ok(json!({
        "deleted_notes": deleted.len(), "dangling_relations": f + r, "stub_chains": fl + rs,
        "suspect_mass_deletion": cleanup::suspect(deleted.len(), tracked),
    }))
}

fn obj_mut<'a>(state: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let slot = state.entry(key.to_string()).or_insert_with(|| json!({}));
    if !slot.is_object() {
        *slot = json!({});
    }
    slot.as_object_mut().expect("just made an object")
}

fn cleanup_step(v: &Vault, st: &Path, state: &mut Map<String, Value>, apply: bool, auto: bool, now: &NaiveDateTime) -> Result<Value> {
    if !apply {
        let deleted = cleanup::deleted_notes(v, st)?.0.len();
        let orphans = cleanup::find_orphans(v)?.len();
        let pending = pending_repairs(v, st, now)?;
        return Ok(json!({
            "applied": false, "auto_repair": auto, "deleted_pending": deleted, "orphans": orphans,
            "retracted_notes": 0, "removed_mentions": 0, "fixed_relations": 0, "removed_relations": 0,
            "flattened_stubs": 0, "retired_stubs": 0, "pruned_cache_keys": 0, "pending_retirements": 0,
            "pending_repairs": pending,
        }));
    }
    let (ret, fixed, removed, flattened, retired_stubs, mut pending) = if auto {
        let ret = cleanup::retract_deleted(v, st, cleanup::MAX_RETRACTIONS, false)?;
        let (f, r) = cleanup::repair_relations(v, cleanup::MAX_RELATION_FIXES, false)?;
        let (fl, rs) = cleanup::flatten_stubs(v, now, false)?;
        if n(&ret, "removed_mentions") + f + r + fl + rs > 0 {
            graph::rebuild(v, &st.join("graph.db"))?;
        }
        (ret, f, r, fl, rs, Value::Null)
    } else {
        let deleted = cleanup::deleted_notes(v, st)?.0.len();
        let ret = json!({"retracted_notes": 0, "removed_mentions": 0, "remaining": deleted, "suspect_mass_deletion": false});
        (ret, 0, 0, 0, 0, pending_repairs(v, st, now)?)
    };
    let orphans = cleanup::find_orphans(v)?;
    let block = obj_mut(state, "cleanup");
    let pending_list = cleanup::update_retirement_proposals(block, &orphans, cleanup::MAX_PENDING_RETIREMENTS);
    block.insert("last_pass".into(), json!(second_stamp(now)));
    let live: Vec<String> = consolidate::gather(v)?.into_iter().map(|e| e.path).collect();
    let live_set: BTreeSet<String> = live.iter().cloned().collect();
    let mut pruned = consolidate::prune_entity_vectors(st, &live)?;
    if let Some(Value::Object(cb)) = state.get_mut("consolidation")
        && !cb.is_empty()
    {
        pruned += cleanup::prune_checked_hash(cb, &live_set);
    }
    if pending.is_null() && ret["suspect_mass_deletion"] == json!(true) {
        pending = pending_repairs(v, st, now)?;
    }
    Ok(json!({
        "applied": true, "auto_repair": auto, "pending_repairs": pending, "deleted_pending": n(&ret, "remaining"),
        "orphans": orphans.len(), "retracted_notes": n(&ret, "retracted_notes"), "removed_mentions": n(&ret, "removed_mentions"),
        "fixed_relations": fixed, "removed_relations": removed, "flattened_stubs": flattened, "retired_stubs": retired_stubs,
        "pruned_cache_keys": pruned, "pending_retirements": pending_list.len(),
    }))
}

pub fn backstop_due(marker: Option<&Value>, now: &NaiveDateTime) -> Result<bool> {
    let Some(m) = marker.and_then(Value::as_str).filter(|m| !m.is_empty()) else { return Ok(false) };
    let then = NaiveDateTime::parse_from_str(m, "%Y-%m-%d %H:%M:%S")
        .map_err(|_| Error::invalid(format!("time data {} does not match format '%Y-%m-%d %H:%M:%S'", crate::text::quote_str(m))))?;
    Ok((*now - then).num_days() >= BACKSTOP_MIN_INTERVAL_DAYS)
}

fn consolidate_step(v: &Vault, st: &Path, state: &mut Map<String, Value>, deps: &Deps, apply: bool, now: &NaiveDateTime) -> Result<Value> {
    let ents = consolidate::gather(v)?;
    if ents.is_empty() {
        return Ok(json!({"ran": false, "reason": "no entities", "proposed": [], "skipped_batches": 0}));
    }
    let mut block: Map<String, Value> = state.get("consolidation").and_then(Value::as_object).cloned().unwrap_or_default();
    let checked: HashMap<String, String> = block
        .get("checked_hash")
        .and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    let cursor = block.get("cursor").and_then(Value::as_str).map(String::from);
    let due = backstop_due(block.get("backstop_last_advance"), now)?;
    let vecs = consolidate::compute_entity_vectors(st, &ents, deps.emb, apply)?;
    let (slice, new_cursor, used) = consolidate::select_slice(&ents, &checked, cursor.as_deref(), SLICE_SIZE, due);
    let stamp = json!(second_stamp(now));
    if slice.is_empty() {
        if apply && !block.contains_key("backstop_last_advance") {
            block.insert("backstop_last_advance".into(), stamp);
            state.insert("consolidation".into(), Value::Object(block));
        }
        return Ok(json!({"ran": false, "reason": "nothing to check", "proposed": [], "skipped_batches": 0}));
    }
    let batches = consolidate::batch_clusters(consolidate::candidate_clusters(&slice, &ents, &vecs), MAX_ENTITIES_PER_CALL);
    let owned;
    let backend: &dyn Complete = match deps.consolidator {
        Some(c) => c,
        None => {
            owned = Cli::consolidation(None)?;
            &owned
        }
    };
    let (merges, skipped) = consolidate::adjudicate(backend, &batches, &ents);
    let proposed: Vec<Value> = merges.iter().map(|m| json!({"type": m.ty, "canonical": m.canonical, "duplicates": m.duplicates})).collect();
    let reason = if used { format!("backstop ({})", slice.len()) } else { format!("{} unchecked", slice.len()) };
    if apply {
        let mut checked: Map<String, Value> = checked.into_iter().map(|(k, v)| (k, json!(v))).collect();
        for e in &slice {
            checked.insert(e.path.clone(), json!(consolidate::identity_hash(e)));
        }
        block.insert("checked_hash".into(), Value::Object(checked));
        block.insert("cursor".into(), new_cursor.map_or(Value::Null, Value::String));
        block.insert("pending_proposals".into(), Value::Array(proposed.clone()));
        block.insert("last_pass".into(), stamp.clone());
        if used || !block.contains_key("backstop_last_advance") {
            block.insert("backstop_last_advance".into(), stamp);
        }
        state.insert("consolidation".into(), Value::Object(block));
    }
    Ok(json!({"ran": true, "reason": reason, "proposed": proposed, "skipped_batches": skipped}))
}

fn check(f: impl FnOnce() -> Result<Value>) -> Value {
    f().unwrap_or_else(|e| json!({"error": e.to_string()}))
}

/// Health checks (08 §4), each isolated.
pub fn health(v: &Vault, st: &Path, state: &Map<String, Value>, steps: &Map<String, Value>, errors: &Map<String, Value>) -> Map<String, Value> {
    let mut h = Map::new();
    // the sweep writes the caretaker notes after the index step embedded the vault (Q23)
    let stale = || Ok(json!(crate::search::stale_notes(v, st)?.iter().filter(|p| !indexer::CARETAKER_NOTES.contains(&p.as_str())).count()));
    h.insert("stale_embeddings".into(), check(stale));
    h.insert(
        "manifest_drift".into(),
        check(|| {
            let hashes: BTreeSet<String> = Manifest::load(st)?.hashes.keys().cloned().collect();
            let scanned: BTreeSet<String> = indexer::scan_notes(v)?.keys().cloned().collect();
            Ok(json!({
                "deleted_but_tracked": hashes.difference(&scanned).collect::<Vec<_>>(),
                "present_but_untracked": scanned.difference(&hashes).collect::<Vec<_>>(),
            }))
        }),
    );
    let db = st.join("graph.db");
    h.insert(
        "orphaned_entities".into(),
        check(|| {
            if !db.exists() {
                return Ok(json!([]));
            }
            let g = Graph::load(&db)?;
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for (e, note, _) in &g.mentions {
                if seen.insert((e.clone(), note.clone())) && !v.resolve(&format!("{note}.md")).is_ok_and(|p| p.is_file()) {
                    out.push(json!({"entity": e, "missing_note": note}));
                }
            }
            Ok(Value::Array(out))
        }),
    );
    h.insert(
        "cache_consistency".into(),
        check(|| {
            let md = consolidate::gather(v)?.len();
            if !db.exists() {
                return Ok(json!({"db_entities": null, "md_entities": md, "consistent": md == 0}));
            }
            let dbn = Graph::load(&db)?.entities.len();
            Ok(json!({"db_entities": dbn, "md_entities": md, "consistent": dbn == md}))
        }),
    );
    let organize = steps.get("organize").and_then(|o| o["proposals"].as_array()).map_or(0, Vec::len);
    let consolidated = match steps.get("consolidate") {
        Some(c) if c["ran"] == json!(true) => c["proposed"].as_array().map_or(0, Vec::len),
        _ => state.get("consolidation").and_then(|c| c["pending_proposals"].as_array()).map_or(0, Vec::len),
    };
    h.insert("pending_proposals".into(), json!(organize + consolidated));
    h.insert("invalid_sheet_rows".into(), check(|| Ok(json!(crate::sheets::invalid_rows(v)?))));
    h.insert(
        "pending_retirements".into(),
        json!(state.get("cleanup").and_then(|c| c["pending_retirements"].as_array()).map_or(0, Vec::len)),
    );
    h.insert("sweep_errors".into(), Value::Object(errors.clone()));
    h
}

/// The per-step summary stored in the state (08 §6).
pub fn summary(steps: &Map<String, Value>) -> Map<String, Value> {
    let pick = |v: &Value, keys: &[&str]| -> Value {
        let mut m = Map::new();
        for k in keys {
            if let Some(x) = v.get(*k) {
                m.insert(k.to_string(), x.clone());
            }
        }
        Value::Object(m)
    };
    let len = |v: &Value, k: &str| v[k].as_array().map_or(0, Vec::len);
    let mut s = Map::new();
    for (name, val) in steps {
        let stored = if val.is_null() {
            Value::Null
        } else {
            match name.as_str() {
                "index" => pick(val, &["processed", "failed", "remaining", "pending"]),
                "organize" => json!({"moved": len(val, "moved"), "proposals": len(val, "proposals"), "skipped": len(val, "skipped")}),
                "cleanup" => {
                    let keys = ["retracted_notes", "fixed_relations", "removed_relations", "flattened_stubs", "retired_stubs", "pruned_cache_keys", "pending_retirements"];
                    let mut c = pick(val, &keys);
                    c["auto_repair"] = val.get("auto_repair").cloned().unwrap_or(json!(false));
                    c["pending_repairs"] = val.get("pending_repairs").cloned().unwrap_or(Value::Null);
                    c
                }
                "consolidate" => {
                    let skipped = val.get("skipped_batches").cloned().unwrap_or(json!(0));
                    json!({"ran": val["ran"], "reason": val["reason"], "proposed": len(val, "proposed"), "skipped_batches": skipped})
                }
                _ => val.clone(),
            }
        };
        s.insert(name.clone(), stored);
    }
    s
}

/// run_sweep(apply, max_moves, now, auto_cleanup) (08 §5).
pub fn run_sweep(v: &Vault, st: &Path, deps: &Deps, apply: bool, max_moves: Option<i64>, now: &NaiveDateTime, auto_cleanup: bool) -> Result<Value> {
    let mut state = load_state(st)?;
    let mut steps = Map::new();
    let mut errors = Map::new();
    let mut record = |steps: &mut Map<String, Value>, name: &str, r: Result<Value>| match r {
        Ok(val) => {
            steps.insert(name.into(), val);
        }
        Err(e) => {
            steps.insert(name.into(), Value::Null);
            errors.insert(name.into(), json!(e.to_string()));
        }
    };
    record(&mut steps, "index", index_step(v, st, deps, apply, now));
    record(&mut steps, "organize", crate::organizer::run(v, st, deps.emb, apply, max_moves, now));
    let (idx, org) = (steps.get("index").filter(|x| !x.is_null()).cloned(), steps.get("organize").filter(|x| !x.is_null()).cloned());
    record(&mut steps, "cache", cache_step(v, st, apply, idx.as_ref(), org.as_ref()));
    record(&mut steps, "cleanup", cleanup_step(v, st, &mut state, apply, auto_cleanup, now));
    record(&mut steps, "consolidate", consolidate_step(v, st, &mut state, deps, apply, now));
    let steps_view: Map<String, Value> = steps.iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k.clone(), x.clone())).collect();
    let h = health(v, st, &state, &steps_view, &errors);
    let result = json!({"steps": steps, "health": h, "errors": errors, "applied": apply});
    if apply {
        state.insert("last_sweep".into(), json!(second_stamp(now)));
        state.insert("steps".into(), Value::Object(summary(result["steps"].as_object().expect("steps is a map"))));
        state.insert("health".into(), result["health"].clone());
        state.insert("errors".into(), result["errors"].clone());
        save_state(st, &state)?;
        append_report(v, &report_section(&result, now))?;
    }
    Ok(result)
}

// ----------------------------------------------------------------- report

fn mark(x: i64) -> &'static str {
    if x == 0 { "\u{2713}" } else { "\u{26a0}" }
}

fn as_stored(v: Option<&Value>) -> String {
    match v {
        None => "0".into(),
        Some(Value::Object(_)) => quoted(&json_to_y(v.unwrap_or(&Value::Null))),
        Some(x) => crate::text::display(&json_to_y(x)),
    }
}

/// The report section (08 §7.1).
pub fn report_section(result: &Value, now: &NaiveDateTime) -> String {
    let s = &result["steps"];
    let mut out = vec![format!("## Sweep {}", minute_stamp(now))];
    out.push(match &s["index"] {
        Value::Null => "- index: FAILED".into(),
        i if i.get("pending").is_some() => format!("- index: {} pending (dry-run)", n(i, "pending")),
        i => format!("- index: processed {}, failed {}, remaining {}", n(i, "processed"), n(i, "failed"), n(i, "remaining")),
    });
    let len = |v: &Value, k: &str| v[k].as_array().map_or(0, Vec::len);
    out.push(match &s["organize"] {
        Value::Null => "- organize: FAILED".into(),
        o => format!("- organize: moved {}, proposals {}, skipped {}", len(o, "moved"), len(o, "proposals"), len(o, "skipped")),
    });
    out.push(match &s["cache"] {
        Value::Null => "- cache: FAILED".into(),
        c if c["rebuilt"] == json!(true) => format!("- cache: rebuilt ({})", c["by"].as_str().unwrap_or_default()),
        _ => "- cache: fresh, no rebuild needed".into(),
    });
    let c = &s["cleanup"];
    out.push(if c.is_null() {
        "- cleanup: FAILED".into()
    } else if c["applied"] != json!(true) {
        format!("- cleanup: {} deleted notes pending, {} orphans (dry-run)", n(c, "deleted_pending"), n(c, "orphans"))
    } else if c["auto_repair"] != json!(true) {
        let p = &c["pending_repairs"];
        format!(
            "- cleanup: report-only \u{2014} {} deleted notes, {} dangling relations, {} stub chains await \u{60}cleanup --apply-repairs\u{60}; {} retirement proposals",
            n(p, "deleted_notes"),
            n(p, "dangling_relations"),
            n(p, "stub_chains"),
            n(c, "pending_retirements")
        )
    } else {
        format!(
            "- cleanup: retracted {} notes, fixed {} relations, {} retirement proposals",
            n(c, "retracted_notes"),
            n(c, "fixed_relations") + n(c, "removed_relations"),
            n(c, "pending_retirements")
        )
    });
    if c["pending_repairs"]["suspect_mass_deletion"] == json!(true) {
        out.push("- cleanup: \u{26a0} so many tracked notes are missing that the vault may be only partly present; retraction held".into());
    }
    out.push(match &s["consolidate"] {
        Value::Null => "- consolidate: FAILED".into(),
        k if k["ran"] == json!(true) => format!("- consolidate: ran ({}) \u{2014} {} merge proposals", k["reason"].as_str().unwrap_or_default(), len(k, "proposed")),
        k => format!("- consolidate: skipped ({})", k["reason"].as_str().unwrap_or_default()),
    });
    let h = &result["health"];
    let int_or = |v: &Value| v.as_i64().unwrap_or(-1);
    let a = int_or(&h["stale_embeddings"]);
    let b = match &h["manifest_drift"] {
        Value::Object(m) if !m.contains_key("error") => (m.get("deleted_but_tracked").and_then(Value::as_array).map_or(0, Vec::len)
            + m.get("present_but_untracked").and_then(Value::as_array).map_or(0, Vec::len)) as i64,
        _ => -1,
    };
    let orph = h["orphaned_entities"].as_array().map_or(-1, |a| a.len() as i64);
    let consistent = if h["cache_consistency"]["consistent"] == json!(true) { "\u{2713}" } else { "\u{26a0}" };
    let d = int_or(&h["invalid_sheet_rows"]);
    out.push(format!(
        "- health: {}",
        [
            format!("stale_embeddings {a} {}", mark(a)),
            format!("manifest_drift {b} {}", mark(b)),
            format!("orphaned_entities {orph} {}", mark(orph)),
            format!("cache_consistency {consistent}"),
            format!("invalid_sheet_rows {d} {}", mark(d)),
            format!("pending_proposals {}", as_stored(h.get("pending_proposals"))),
            format!("pending_retirements {}", as_stored(h.get("pending_retirements"))),
        ]
        .join(" | ")
    ));
    let errs = result["errors"].as_object().cloned().unwrap_or_default();
    out.push(if errs.is_empty() {
        "- errors: none".into()
    } else {
        format!("- errors: {}", errs.iter().map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default())).collect::<Vec<_>>().join(", "))
    });
    out.iter().map(|l| format!("{l}\n")).collect()
}

/// Appends a section to Claude/Librarian.md, keeping the last 30 sweeps (08 §7.2).
pub fn append_report(v: &Vault, section: &str) -> Result<()> {
    let old = match v.read(REPORT_NOTE) {
        Ok(t) => t,
        Err(e) if e.is(Kind::VaultError) => REPORT_SEED.to_string(),
        Err(e) => return Err(e),
    };
    let text = format!("{}\n\n{section}", old.trim_end_matches('\n'));
    let mut parts = text.split("\n## Sweep ");
    let header = parts.next().unwrap_or("").to_string();
    let sweeps: Vec<&str> = parts.collect();
    let keep = &sweeps[sweeps.len().saturating_sub(REPORT_MAX_SWEEPS)..];
    let mut out = header;
    for s in keep {
        out.push_str("\n## Sweep ");
        out.push_str(s);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    v.write(REPORT_NOTE, &out, true, false)?;
    Ok(())
}
