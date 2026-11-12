//! acceptance/librarian.yaml (NOW = 2026-07-09 12:00:00)
mod common;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use common::*;
use enigma_mcp::consolidate::{MAX_ENTITIES_PER_CALL, SLICE_SIZE};
use enigma_mcp::error::{Kind, Result};
use enigma_mcp::extractor::{Complete, Extract};
use enigma_mcp::faults::{self, Fault};
use enigma_mcp::graph::{self, Entity, Extraction};
use enigma_mcp::indexer::Manifest;
use enigma_mcp::librarian::*;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Mutex;
use tempfile::TempDir;

fn now9() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 9).unwrap().and_hms_opt(12, 0, 0).unwrap()
}

/// Extracts no entities and records every path it is asked about.
#[derive(Default)]
struct NoEntities {
    calls: Mutex<Vec<String>>,
}

impl Extract for NoEntities {
    fn extract(&self, path: &str, _content: &str) -> Result<Extraction> {
        self.calls.lock().unwrap().push(path.to_string());
        Ok(Extraction::default())
    }
}

/// Rewrites every note it reads, so the indexer never drains.
struct Restless(std::path::PathBuf);

impl Extract for Restless {
    fn extract(&self, path: &str, content: &str) -> Result<Extraction> {
        std::fs::write(self.0.join(path), format!("{content}x")).unwrap();
        Ok(Extraction::default())
    }
}

/// Proposes Acme <- Acme Corp and counts calls.
#[derive(Default)]
struct Merger {
    calls: Mutex<usize>,
}

impl Complete for Merger {
    fn complete_json(&self, _prompt: &str) -> Result<Value> {
        *self.calls.lock().unwrap() += 1;
        Ok(json!({"merges": [{"type": "organization", "canonical": "Acme", "duplicates": ["Acme Corp"]}]}))
    }
}

impl Merger {
    fn n(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

struct Rig {
    d: TempDir,
    st: TempDir,
    ex: NoEntities,
    con: Merger,
    emb: LenEmb,
}

impl Rig {
    fn new(d: TempDir) -> Rig {
        Rig { d, st: tempfile::tempdir().unwrap(), ex: NoEntities::default(), con: Merger::default(), emb: LenEmb::default() }
    }

    fn mv() -> Rig {
        Rig::new(mini_vault())
    }

    fn st(&self) -> &Path {
        self.st.path()
    }

    fn sweep_at(&self, apply: bool, now: &NaiveDateTime, auto: bool) -> Value {
        let deps = Deps { extractor: Some(&self.ex), consolidator: Some(&self.con), emb: &self.emb };
        run_sweep(&open(&self.d), self.st(), &deps, apply, None, now, auto).unwrap()
    }

    fn sweep(&self, apply: bool) -> Value {
        self.sweep_at(apply, &now9(), false)
    }

    fn state(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(state_path(self.st())).unwrap()).unwrap()
    }

    fn org(&self, name: &str, summary: &str) {
        let e = Entity { name: name.into(), ty: "organization".into(), aliases: vec![], summary: summary.into() };
        graph::upsert_entity(&open(&self.d), &e, &now9()).unwrap();
    }

    fn acme_pair(&self) {
        self.org("Acme", "First.");
        self.org("Acme Corp", "Second.");
    }
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| (e.path().display().to_string(), std::fs::read(e.path()).unwrap()))
        .collect()
}

fn health_of(r: &Rig, state: &Map<String, Value>, steps: Value) -> Map<String, Value> {
    health(&open(&r.d), r.st(), state, steps.as_object().unwrap(), &Map::new())
}

const ACME: &str = "Claude/Graph/Organizations/Acme";

/// Entity Acme mentions Projects/Doomed (tracked, file absent); cache built.
fn doomed(r: &Rig) {
    let v = open(&r.d);
    write_file(r.d.path(), "Projects/Doomed.md", "doomed");
    let x = Extraction { entities: vec![Entity { name: "Acme".into(), ty: "organization".into(), aliases: vec![], summary: "S.".into() }], relations: vec![] };
    graph::apply(&v, "Projects/Doomed.md", &x, &now9()).unwrap();
    let mut m = Manifest::default();
    m.hashes.insert("Projects/Doomed.md".into(), "h".into());
    m.save(r.st()).unwrap();
    graph::rebuild(&v, &r.st().join("graph.db")).unwrap();
    std::fs::remove_file(r.d.path().join("Projects/Doomed.md")).unwrap();
}

fn cli(vault: &Path, st: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            cmd.env_remove(k);
        }
    }
    cmd.arg("librarian").arg(vault).args(args).env("ENIGMA_STATE_DIR", st).envs(env.iter().copied());
    cmd.output().unwrap()
}

/// The JSON result printed after any report lines.
fn printed_json(o: &Output) -> Value {
    let s = String::from_utf8(o.stdout.clone()).unwrap();
    let at = s.find("\n{").map_or(0, |i| i + 1);
    serde_json::from_str(&s[at..]).unwrap()
}

#[test]
fn librarian_lb_01() {
    assert_eq!(BACKSTOP_MIN_INTERVAL_DAYS, 14);
    assert_eq!(SLICE_SIZE, 200);
    assert_eq!(MAX_ENTITIES_PER_CALL, 40);
    assert_eq!(REPORT_MAX_SWEEPS, 30);
    assert_eq!(MAX_INDEX_ROUNDS, 40);
}

#[test]
fn librarian_lb_02() {
    let n = now9();
    let marker = json!("2026-07-09 12:00:00");
    assert!(!backstop_due(None, &n).unwrap());
    assert!(!backstop_due(Some(&marker), &(n + Duration::days(13))).unwrap());
    assert!(backstop_due(Some(&marker), &(n + Duration::days(14))).unwrap());
}

#[test]
fn librarian_lb_03() {
    let st = tempfile::tempdir().unwrap();
    let mut s = load_state(st.path()).unwrap();
    assert_eq!(Value::Object(s.clone()), json!({"last_sweep": null, "steps": {}, "health": {}, "errors": {}, "consolidation": {}}));
    assert_eq!(s.keys().collect::<Vec<_>>(), ["last_sweep", "steps", "health", "errors", "consolidation"]);
    s.insert("last_sweep".into(), json!("2026-07-09 12:00:00"));
    save_state(st.path(), &s).unwrap();
    assert_eq!(load_state(st.path()).unwrap(), s);
}

#[test]
fn librarian_lb_04() {
    let st = tempfile::tempdir().unwrap();
    let mut s = load_state(st.path()).unwrap();
    s.insert("last_sweep".into(), json!("2026-07-09 12:00:00"));
    save_state(st.path(), &s).unwrap();
    let before = std::fs::read(state_path(st.path())).unwrap();
    let mut later = s.clone();
    later.insert("last_sweep".into(), json!("2026-07-10 12:00:00"));
    faults::reset();
    faults::arm("state_rename", Fault::Os(5), 1);
    assert!(save_state(st.path(), &later).is_err());
    assert_eq!(std::fs::read(state_path(st.path())).unwrap(), before);
    assert_eq!(load_state(st.path()).unwrap(), s);
}

#[test]
fn librarian_lb_05() {
    let r = Rig::mv();
    std::fs::write(state_path(r.st()), "{not json").unwrap();
    let s = status(r.st()).unwrap();
    assert_eq!(s["status"], "state file unreadable");
    assert!(!s["error"].as_str().unwrap().is_empty());
    assert_eq!(s["path"], json!(state_path(r.st()).to_string_lossy()));
    // the sweep fails before any step runs
    let deps = Deps { extractor: Some(&r.ex), consolidator: Some(&r.con), emb: &r.emb };
    let e = run_sweep(&open(&r.d), r.st(), &deps, false, None, &now9(), false).err().unwrap();
    assert_eq!(e.kind, Kind::InvalidData);
    assert!(r.ex.calls.lock().unwrap().is_empty());
}

#[test]
fn librarian_lb_06() {
    let r = Rig::mv();
    let mut m = Manifest::default();
    m.hashes.insert("Ghost.md".into(), "h".into());
    m.save(r.st()).unwrap();
    let h = health_of(&r, &load_state(r.st()).unwrap(), json!({}));
    assert!(h["manifest_drift"]["deleted_but_tracked"].as_array().unwrap().contains(&json!("Ghost.md")));
    assert!(h["manifest_drift"]["present_but_untracked"].as_array().unwrap().contains(&json!("Daily.md")));
}

#[test]
fn librarian_lb_07() {
    let r = Rig::mv();
    let v = open(&r.d);
    let db = r.st().join("graph.db");
    let acme = Extraction { entities: vec![Entity { name: "Acme".into(), ty: "organization".into(), aliases: vec![], summary: "S.".into() }], relations: vec![] };
    let state = load_state(r.st()).unwrap();
    graph::apply(&v, "Projects/Gone.md", &acme, &now9()).unwrap();
    assert_eq!(health_of(&r, &state, json!({}))["orphaned_entities"], json!([]), "no cache db");
    graph::rebuild(&v, &db).unwrap();
    assert_eq!(health_of(&r, &state, json!({}))["orphaned_entities"], json!([{"entity": ACME, "missing_note": "Projects/Gone"}]));
    // a mention of an existing note only
    std::fs::remove_file(r.d.path().join(format!("{ACME}.md"))).unwrap();
    graph::apply(&v, "Projects/Lighthouse Grid.md", &acme, &now9()).unwrap();
    graph::rebuild(&v, &db).unwrap();
    assert_eq!(health_of(&r, &state, json!({}))["orphaned_entities"], json!([]));
}

#[test]
fn librarian_lb_08() {
    let r = Rig::mv();
    r.org("Acme", "S.");
    graph::rebuild(&open(&r.d), &r.st().join("graph.db")).unwrap();
    let state = load_state(r.st()).unwrap();
    assert_eq!(health_of(&r, &state, json!({}))["cache_consistency"]["consistent"], true);
    r.org("Beta", "S.");
    assert_eq!(health_of(&r, &state, json!({}))["cache_consistency"], json!({"db_entities": 1, "md_entities": 2, "consistent": false}));
}

#[test]
fn librarian_lb_09() {
    let r = Rig::mv();
    let mut state = load_state(r.st()).unwrap();
    state.insert("consolidation".into(), json!({"pending_proposals": [{"x": 1}]}));
    let organize = json!({"moved": [], "proposals": [{}, {}], "skipped": []});
    let h = health_of(&r, &state, json!({"organize": organize, "consolidate": {"ran": false, "proposed": []}}));
    assert_eq!(h["pending_proposals"], 3);
    let h = health_of(&r, &state, json!({"organize": organize, "consolidate": {"ran": true, "proposed": [{}]}}));
    assert_eq!(h["pending_proposals"], 3);
}

#[test]
fn librarian_lb_10() {
    let r = Rig::mv();
    std::fs::write(r.st().join("manifest.json"), "kaput").unwrap();
    let h = health_of(&r, &load_state(r.st()).unwrap(), json!({}));
    assert!(h["manifest_drift"]["error"].as_str().unwrap().starts_with("InvalidData: "), "{}", h["manifest_drift"]);
    let keys: Vec<&String> = h.keys().collect();
    assert_eq!(
        keys,
        ["stale_embeddings", "manifest_drift", "orphaned_entities", "cache_consistency", "pending_proposals", "invalid_sheet_rows", "pending_retirements", "sweep_errors"]
    );
    assert!(h["stale_embeddings"].as_i64().unwrap() >= 0);
}

#[test]
fn librarian_lb_11() {
    let r = Rig::mv();
    let res = r.sweep(true);
    // Daily.md was indexed under its old path, so index ran before organize moved it
    assert_eq!(res["steps"]["organize"]["moved"][0]["from"], "Daily.md");
    let calls = r.ex.calls.lock().unwrap();
    assert!(calls.contains(&"Daily.md".to_string()));
    assert!(!calls.iter().any(|c| c == "Projects/Daily.md"));
}

#[test]
fn librarian_lb_12() {
    let notes: Vec<(String, String)> = (0..30).map(|i| (format!("N{i:02}.md"), format!("note {i}"))).collect();
    let refs: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let r = Rig::new(vault_with(&refs));
    let res = r.sweep(true);
    assert_eq!(res["steps"]["index"]["processed"], 30);
    assert_eq!(res["steps"]["index"]["remaining"], 0);
    // a round that never drains
    let r = Rig::new(vault_with(&refs[..26]));
    let ex = Restless(r.d.path().to_path_buf());
    let deps = Deps { extractor: Some(&ex), consolidator: Some(&r.con), emb: &r.emb };
    let res = run_sweep(&open(&r.d), r.st(), &deps, true, None, &now9(), false).unwrap();
    assert!(res["steps"]["index"].is_null());
    let e = res["errors"]["index"].as_str().unwrap();
    assert!(e.contains("did not drain"), "{e}");
    assert_eq!(e, "Internal: index did not drain after 40 rounds; 1 notes still pending");
}

#[test]
fn librarian_lb_13() {
    let r = Rig::mv();
    std::fs::write(r.d.path().join("Broken.md"), b"\xff\xfe broken").unwrap();
    r.org("Acme", "S.");
    let res = r.sweep(true);
    let e = res["errors"]["organize"].as_str().unwrap();
    assert!(regex::Regex::new(r"^[A-Za-z]+: .+").unwrap().is_match(e), "{e}");
    assert!(res["steps"]["organize"].is_null());
    assert!(!res["steps"]["consolidate"].is_null());
    assert!(!res["health"]["sweep_errors"]["organize"].as_str().unwrap().is_empty());
}

#[test]
fn librarian_lb_14() {
    let r = Rig::mv();
    r.acme_pair();
    let res = r.sweep(true);
    let c = &res["steps"]["consolidate"];
    assert_eq!(c["ran"], true);
    assert_eq!(c["proposed"], json!([{"type": "organization", "canonical": "Acme", "duplicates": ["Acme Corp"]}]));
    let st = r.state();
    let checked = st["consolidation"]["checked_hash"].as_object().unwrap();
    assert!(checked.contains_key(ACME) && checked.contains_key("Claude/Graph/Organizations/Acme Corp"));
    assert_eq!(st["consolidation"]["pending_proposals"], c["proposed"]);
    assert_eq!(r.con.n(), 1);
}

#[test]
fn librarian_lb_15() {
    let r = Rig::mv();
    r.acme_pair();
    r.sweep(true);
    assert_eq!(r.con.n(), 1);
    let res = r.sweep_at(true, &(now9() + Duration::days(1)), false);
    assert_eq!(r.con.n(), 1);
    assert_eq!(res["steps"]["consolidate"]["reason"], "nothing to check");
}

#[test]
fn librarian_lb_16() {
    let r = Rig::mv();
    r.acme_pair();
    r.sweep(true);
    let p = r.d.path().join(format!("{ACME}.md"));
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("First."));
    std::fs::write(&p, text.replace("First.", "Changed.")).unwrap();
    r.sweep_at(true, &(now9() + Duration::days(1)), false);
    assert_eq!(r.con.n(), 2);
    r.sweep_at(true, &(now9() + Duration::days(2)), false);
    assert_eq!(r.con.n(), 2);
}

#[test]
fn librarian_lb_17() {
    let r = Rig::mv();
    r.sweep(true);
    let st = r.state();
    assert_eq!(st["last_sweep"], "2026-07-09 12:00:00");
    assert!(st["steps"].get("index").is_some());
    assert!(st["health"].get("stale_embeddings").is_some());
    assert!(read(&r.d, REPORT_NOTE).contains("## Sweep 2026-07-09 12:00\n"));
}

#[test]
fn librarian_lb_18() {
    let r = Rig::mv();
    r.acme_pair();
    r.sweep(true);
    let (vault, state) = (snapshot(r.d.path()), snapshot(r.st()));
    let res = r.sweep_at(false, &(now9() + Duration::days(1)), false);
    assert_eq!(res["applied"], false);
    assert_eq!(res["steps"]["index"], json!({"pending": 0}));
    assert_eq!(snapshot(r.d.path()), vault);
    assert_eq!(snapshot(r.st()), state);
}

#[test]
fn librarian_lb_19() {
    let r = Rig::mv();
    r.acme_pair();
    let vault = snapshot(r.d.path());
    let res = r.sweep(false);
    assert_eq!(res["steps"]["consolidate"]["ran"], true);
    assert_eq!(snapshot(r.d.path()), vault);
    assert!(snapshot(r.st()).is_empty(), "{:?}", snapshot(r.st()).keys());
}

#[test]
fn librarian_lb_20() {
    let r = Rig::mv();
    r.sweep(true);
    r.sweep_at(true, &(now9() + Duration::minutes(5)), false);
    assert_eq!(r.sweep(false)["steps"]["index"], json!({"pending": 0}));
}

#[test]
fn librarian_lb_21() {
    let r = Rig::mv();
    r.sweep(true);
    r.sweep_at(true, &(now9() + Duration::minutes(5)), false);
    assert!(r.d.path().join("Claude/Organizer.md").is_file());
    let raw = std::fs::read_to_string(r.st().join("manifest.json")).unwrap();
    assert!(!raw.contains("Claude/Librarian.md") && !raw.contains("Claude/Organizer.md"), "{raw}");
}

#[test]
fn librarian_lb_22() {
    let r = Rig::mv();
    let res = r.sweep(true);
    assert_eq!(res["health"]["manifest_drift"], json!({"deleted_but_tracked": [], "present_but_untracked": []}));
}

#[test]
fn librarian_lb_23() {
    let r = Rig::mv();
    r.sweep(true);
    enigma_mcp::mover::undo_move(&open(&r.d), r.st(), "Projects/Daily.md", &now9()).unwrap();
    assert!(read(&r.d, "Claude/Organizer.md").contains("UNDID"));
    r.ex.calls.lock().unwrap().clear();
    r.sweep_at(true, &(now9() + Duration::minutes(5)), false);
    assert!(!r.ex.calls.lock().unwrap().iter().any(|c| c == "Claude/Organizer.md"));
}

#[test]
fn librarian_lb_24() {
    let result = json!({
        "applied": true,
        "steps": {
            "index": {"processed": 3, "failed": 1, "remaining": 0},
            "organize": {"moved": [{}], "proposals": [{}, {}], "skipped": [{}]},
            "cache": {"rebuilt": true, "by": "index"},
            "consolidate": {"ran": false, "reason": "2 new entities since last pass; threshold 15", "proposed": []},
        },
        "health": {
            "stale_embeddings": 0,
            "manifest_drift": {"deleted_but_tracked": [], "present_but_untracked": []},
            "orphaned_entities": [{"entity": "E", "missing_note": "N"}],
            "cache_consistency": {"db_entities": 1, "md_entities": 1, "consistent": true},
            "pending_proposals": 2,
            "sweep_errors": {},
        },
        "errors": {},
    });
    let s = report_section(&result, &now9());
    assert!(s.starts_with("## Sweep 2026-07-09 12:00\n"));
    for line in [
        "- index: processed 3, failed 1, remaining 0\n",
        "- organize: moved 1, proposals 2, skipped 1\n",
        "- cache: rebuilt (index)\n",
        "- cleanup: FAILED\n",
        "- consolidate: skipped (2 new entities since last pass; threshold 15)\n",
        "- errors: none\n",
    ] {
        assert!(s.contains(line), "missing {line:?} in\n{s}");
    }
    let health = s.lines().find(|l| l.starts_with("- health: ")).unwrap();
    for part in ["stale_embeddings 0 ✓", "orphaned_entities 1 ⚠", "invalid_sheet_rows -1 ⚠"] {
        assert!(health.contains(part), "{health}");
    }
}

#[test]
fn librarian_lb_25() {
    let result = json!({
        "applied": true,
        "steps": {
            "index": null,
            "organize": {"moved": [], "proposals": [], "skipped": []},
            "cache": {"rebuilt": false, "by": "none"},
            "consolidate": {"ran": true, "reason": "first pass", "proposed": [{}]},
        },
        "health": {
            "stale_embeddings": 0, "manifest_drift": {}, "orphaned_entities": [],
            "cache_consistency": {"consistent": true}, "pending_proposals": 1, "sweep_errors": {"index": "Internal: x"},
        },
        "errors": {"index": "Internal: x"},
    });
    let s = report_section(&result, &now9());
    for line in ["- index: FAILED\n", "- consolidate: ran (first pass) — 1 merge proposals\n", "- errors: index: Internal: x\n"] {
        assert!(s.contains(line), "missing {line:?} in\n{s}");
    }
}

#[test]
fn librarian_lb_26() {
    let r = Rig::mv();
    r.acme_pair();
    r.sweep(true);
    let st = r.state();
    assert!(st["steps"]["consolidate"].get("skipped_batches").is_some());
    assert!(st["steps"]["consolidate"]["proposed"].is_i64());
    let c = st["steps"]["cleanup"].as_object().unwrap();
    for k in ["retracted_notes", "fixed_relations", "removed_relations", "flattened_stubs", "retired_stubs", "pruned_cache_keys", "pending_retirements", "auto_repair", "pending_repairs"] {
        assert!(c.contains_key(k), "{k}");
    }
    assert_eq!(st["steps"]["organize"], json!({"moved": 1, "proposals": 0, "skipped": 1}));
    // missing keys take their defaults, never an error
    let steps = json!({"cleanup": {"retracted_notes": 2}, "consolidate": {"ran": false, "reason": "no entities", "proposed": []}});
    let s = summary(steps.as_object().unwrap());
    assert_eq!(s["cleanup"], json!({"retracted_notes": 2, "auto_repair": false, "pending_repairs": null}));
    assert_eq!(s["consolidate"], json!({"ran": false, "reason": "no entities", "proposed": 0, "skipped_batches": 0}));
}

#[test]
fn librarian_lb_27() {
    let d = mini_vault();
    let v = open(&d);
    for i in 0..33 {
        append_report(&v, &format!("## Sweep 2026-07-09 12:{i:02}\n- index: FAILED\n")).unwrap();
    }
    let text = read(&d, REPORT_NOTE);
    assert!(text.starts_with(REPORT_SEED));
    assert_eq!(text.matches("## Sweep ").count(), 30);
    for gone in ["12:00\n", "12:01\n", "12:02\n"] {
        assert!(!text.contains(&format!("## Sweep 2026-07-09 {gone}")));
    }
    assert!(text.contains("## Sweep 2026-07-09 12:03\n") && text.contains("## Sweep 2026-07-09 12:32\n"));
    assert!(text.ends_with('\n'));
}

#[test]
fn librarian_lb_28() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let o = cli(d.path(), st.path(), &["--dry-run"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8(o.stdout.clone()).expect("UTF-8 output");
    assert!(out.contains("## Sweep"));
    let (section, json) = out.split_once("\n\n{\n").expect("a blank line before the JSON");
    assert!(section.contains("## Sweep") && !section.contains('{'));
    assert!(json.contains("\"applied\": false") && json.ends_with("}\n"));
    assert!(!d.path().join(REPORT_NOTE).exists());
}

#[test]
fn librarian_lb_29() {
    let d = mini_vault();
    std::fs::write(d.path().join("Broken.md"), b"\xff\xfe broken").unwrap();
    let st = tempfile::tempdir().unwrap();
    let o = cli(d.path(), st.path(), &["--dry-run"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!printed_json(&o)["errors"].as_object().unwrap().is_empty());
}

#[test]
fn librarian_lb_30() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let fake = fake_codex_dir();
    let path = path_with(fake.path());
    let env = [("PATH", path.as_str()), ("ENIGMA_EXTRACTOR", "codex")];
    let o = cli(d.path(), st.path(), &[], &env);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--confirm-first-run"));
    assert!(!d.path().join(REPORT_NOTE).exists());
    assert!(!state_path(st.path()).exists());
    let o = cli(d.path(), st.path(), &["--confirm-first-run"], &env);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(printed_json(&o)["applied"], true);
    assert!(d.path().join(REPORT_NOTE).is_file());
    let o = cli(d.path(), st.path(), &[], &env);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(read(&d, REPORT_NOTE).matches("## Sweep ").count(), 2);
}

#[test]
fn librarian_lb_31() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let o = cli(d.path(), st.path(), &["--dry-run"], &[]);
    assert_eq!(printed_json(&o)["steps"]["cleanup"]["auto_repair"], false);
    let o = cli(d.path(), st.path(), &["--dry-run", "--auto-cleanup"], &[]);
    assert_eq!(printed_json(&o)["steps"]["cleanup"]["auto_repair"], true);
}

#[test]
fn librarian_lb_32() {
    let r = Rig::mv();
    r.acme_pair();
    r.sweep(true);
    assert_eq!(r.state()["consolidation"]["backstop_last_advance"], "2026-07-09 12:00:00");
    assert_eq!(r.con.n(), 1);
    r.sweep_at(true, &(now9() + Duration::days(13)), false);
    assert_eq!(r.con.n(), 1);
    let res = r.sweep_at(true, &(now9() + Duration::days(14)), false);
    assert!(r.con.n() > 1);
    assert!(res["steps"]["consolidate"]["reason"].as_str().unwrap().starts_with("backstop ("));
    // a carried-over state without the marker gets stamped on an empty-slice sweep
    let mut st = load_state(r.st()).unwrap();
    st["consolidation"].as_object_mut().unwrap().remove("backstop_last_advance");
    save_state(r.st(), &st).unwrap();
    let res = r.sweep_at(true, &(now9() + Duration::days(15)), false);
    assert_eq!(res["steps"]["consolidate"]["reason"], "nothing to check");
    assert_eq!(r.state()["consolidation"]["backstop_last_advance"], "2026-07-24 12:00:00");
}

#[test]
fn librarian_lb_33() {
    let r = Rig::mv();
    doomed(&r);
    let res = r.sweep_at(true, &now9(), true);
    let c = &res["steps"]["cleanup"];
    assert_eq!(c["retracted_notes"], 1);
    assert_eq!(c["removed_mentions"], 1);
    assert!(!read(&r.d, &format!("{ACME}.md")).contains("Doomed"));
    assert_eq!(c["pending_retirements"], 1);
    assert_eq!(r.state()["cleanup"]["pending_retirements"][0]["path"], ACME);
}

#[test]
fn librarian_lb_34() {
    let r = Rig::mv();
    doomed(&r);
    let before = read(&r.d, &format!("{ACME}.md"));
    let res = r.sweep_at(false, &now9(), true);
    assert_eq!(res["steps"]["cleanup"]["applied"], false);
    assert_eq!(res["steps"]["cleanup"]["deleted_pending"], 1);
    assert_eq!(read(&r.d, &format!("{ACME}.md")), before);
}

#[test]
fn librarian_lb_35() {
    let r = Rig::mv();
    r.org("Acme", "S.");
    r.sweep(true);
    assert_eq!(r.state()["consolidation"]["checked_hash"].as_object().unwrap().len(), 1);
    std::fs::remove_file(r.d.path().join(format!("{ACME}.md"))).unwrap();
    let res = r.sweep_at(true, &(now9() + Duration::days(1)), false);
    assert_eq!(res["steps"]["cleanup"]["pruned_cache_keys"], 2);
    assert_eq!(r.state()["consolidation"]["checked_hash"], json!({}));
}

#[test]
fn librarian_lb_36() {
    let r = Rig::mv();
    let mut state = load_state(r.st()).unwrap();
    state.insert("cleanup".into(), json!({"pending_retirements": [{"path": ACME}]}));
    assert_eq!(health_of(&r, &state, json!({}))["pending_retirements"], 1);
}

#[test]
fn librarian_lb_37() {
    let r = Rig::mv();
    doomed(&r);
    let before = read(&r.d, &format!("{ACME}.md"));
    let res = r.sweep_at(true, &now9(), false);
    let c = &res["steps"]["cleanup"];
    assert_eq!(c["auto_repair"], false);
    assert_eq!(c["retracted_notes"], 0);
    assert_eq!(c["pending_repairs"]["deleted_notes"], 1);
    assert_eq!(read(&r.d, &format!("{ACME}.md")), before);
    assert!(Manifest::load(r.st()).unwrap().hashes.contains_key("Projects/Doomed.md"));
    let report = read(&r.d, REPORT_NOTE);
    assert!(report.contains("cleanup: report-only") && report.contains("--apply-repairs"), "{report}");
}

#[test]
fn librarian_lb_38() {
    let r = Rig::mv();
    doomed(&r);
    save_state(r.st(), &load_state(r.st()).unwrap()).unwrap(); // not a first sweep
    let fake = fake_codex_dir();
    let path = path_with(fake.path());
    let env = [("PATH", path.as_str()), ("ENIGMA_EXTRACTOR", "codex"), ("ENIGMA_AUTO_CLEANUP", "1")];
    let o = cli(r.d.path(), r.st(), &[], &env);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(printed_json(&o)["steps"]["cleanup"]["retracted_notes"], 1);
}

#[test]
fn librarian_lb_39() {
    let r = Rig::mv();
    // folder notes pinned so the only organizer outcome is the Daily.md proposal
    write_file(r.d.path(), "Areas/Garden.md", "---\norganize: false\n---\n\ntomatoes and beans\n");
    write_file(r.d.path(), "Projects/Lighthouse Grid.md", "---\norganize: false\n---\n\nGrid outage ingestion pipeline.\n");
    for minutes in [0, 5] {
        let res = r.sweep_at(true, &(now9() + Duration::minutes(minutes)), false);
        assert!(!res["steps"]["organize"]["proposals"].as_array().unwrap().is_empty());
        assert!(res["steps"]["organize"]["moved"].as_array().unwrap().is_empty());
        assert_eq!(res["health"]["stale_embeddings"], json!(0), "sweep at +{minutes} min");
    }
    assert!(read(&r.d, "Claude/Organizer.md").contains("Daily.md"));
    assert_eq!(r.sweep(false)["health"]["stale_embeddings"], json!(0));
}

#[test]
fn librarian_lb_40() {
    let r = Rig::mv();
    r.sweep(true);
    let daily = if r.d.path().join("Daily.md").exists() { "Daily.md" } else { "Projects/Daily.md" };
    std::fs::write(r.d.path().join(daily), "# Daily\n\nEdited after indexing.\n").unwrap();
    assert_eq!(r.sweep(false)["health"]["stale_embeddings"], json!(1));
}
