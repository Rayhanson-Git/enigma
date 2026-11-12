//! acceptance/organizer.yaml (cluster embedder)
mod common;
use common::*;
use enigma_mcp::indexer::Manifest;
use enigma_mcp::mover::{JOURNAL, LOG_NOTE, move_note, record_move, undo_move};
use enigma_mcp::organizer::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

/// "orbit" only → [1,0]; "recipe" only → [0,1]; both or neither → [0.7,0.7].
fn cluster(t: &str) -> Vec<f64> {
    match (t.contains("orbit"), t.contains("recipe")) {
        (true, false) => vec![1.0, 0.0],
        (false, true) => vec![0.0, 1.0],
        _ => vec![0.7, 0.7],
    }
}

fn fixture() -> tempfile::TempDir {
    vault_with(&[
        ("02 - Orbit/Probe Ops.md", "orbit telemetry anomaly research"),
        ("02 - Orbit/CubeSat.md", "orbit conference cubesat"),
        ("02 - Orbit/Signals.md", "orbit lstm telemetry"),
        ("05 - Kitchen/Levain.md", "recipe starter flour"),
        ("05 - Kitchen/Pho.md", "recipe broth noodles"),
        ("00 - Maps of Content/Home.md", "home map"),
        ("Claude/Inbox/capture.md", "a capture"),
        ("Loose Orbit Note.md", "orbit transfer telemetry note"),
        ("Pinned.md", "---\norganize: false\n---\n\norbit pinned note\n"),
        (".obsidian/app.md", "x"),
        (".smart-env/x.md", "x"),
        (".trash/old.md", "x"),
        ("CLAUDE.md", "orbit guide"),
        ("AGENTS.md", "orbit agents"),
        (".claude/commands/day.md", "orbit day"),
    ])
}

fn sweep(d: &tempfile::TempDir, st: &Path, apply: bool, max_moves: Option<i64>) -> Value {
    run(&open(d), st, &FnEmb(cluster), apply, max_moves, &now()).unwrap()
}

fn froms(r: &Value) -> Vec<String> {
    r["moved"].as_array().unwrap().iter().map(|m| m["from"].as_str().unwrap().to_string()).collect()
}

fn proposal<'a>(r: &'a Value, path: &str) -> Option<&'a Value> {
    r["proposals"].as_array().unwrap().iter().find(|p| p["path"] == path)
}

#[test]
fn organizer_or_01() {
    // the constants are private; their effect is pinned by OR-05/OR-06 and OR-15
    let labelled: Vec<String> = (0..12).map(|i| format!("A/{i}.md")).collect();
    let mut vecs: HashMap<String, Vec<f64>> = labelled.iter().map(|p| (p.clone(), vec![1.0, 0.0])).collect();
    vecs.insert("c.md".into(), vec![1.0, 0.0]);
    let (_, share, neigh) = classify("c.md", &vecs, &labelled);
    assert_eq!(neigh.len(), 10, "VOTE_K is 10");
    assert_eq!(share, 1.0);
}

#[test]
fn organizer_or_02() {
    let d = fixture();
    assert_eq!(taxonomy(&open(&d)).unwrap(), ["02 - Orbit", "05 - Kitchen"]);
    std::fs::create_dir_all(d.path().join("07 - Money")).unwrap();
    write_file(d.path(), "Records/_schema.md", "---\nsheet: r\nfilename: x\nkey: [a]\ncolumns: {}\n---\n");
    assert_eq!(taxonomy(&open(&d)).unwrap(), ["02 - Orbit", "05 - Kitchen", "07 - Money"]);
}

#[test]
fn organizer_or_03() {
    let d = fixture();
    let v = open(&d);
    let o = organized(&v, &taxonomy(&v).unwrap()).unwrap();
    assert!(o.contains(&"02 - Orbit/Probe Ops.md".into()) && o.contains(&"05 - Kitchen/Pho.md".into()));
    assert!(o.iter().all(|p| !p.starts_with("Claude/") && !p.starts_with("00 - Maps of Content")));
}

#[test]
fn organizer_or_04() {
    let d = fixture();
    write_file(d.path(), "Records/_schema.md", "---\nsheet: r\nfilename: x\nkey: [a]\ncolumns: {}\n---\n");
    write_file(d.path(), "Records/Row.md", "orbit row");
    let v = open(&d);
    let c = candidates(&v, &organized(&v, &taxonomy(&v).unwrap()).unwrap()).unwrap();
    assert!(c.contains(&"Loose Orbit Note.md".into()) && c.contains(&"02 - Orbit/Probe Ops.md".into()));
    for never in ["Pinned.md", "Claude/Inbox/capture.md", "CLAUDE.md", "AGENTS.md", ".claude/commands/day.md", "Records/Row.md"] {
        assert!(!c.contains(&never.to_string()), "{never}");
    }
}

fn split(cand: Vec<f64>) -> (Option<String>, f64, Vec<String>) {
    let labelled: Vec<String> = ["O/a.md", "O/b.md", "O/c.md", "K/a.md", "K/b.md"].iter().map(|s| s.to_string()).collect();
    let mut vecs: HashMap<String, Vec<f64>> = HashMap::new();
    for p in &labelled {
        vecs.insert(p.clone(), if p.starts_with('O') { vec![1.0, 0.0] } else { vec![0.0, 1.0] });
    }
    vecs.insert("c.md".into(), cand);
    classify("c.md", &vecs, &labelled)
}

#[test]
fn organizer_or_05() {
    let (f, share, neigh) = split(vec![0.9, 0.1]);
    assert_eq!(f.as_deref(), Some("O"));
    assert!(share >= 0.7);
    assert!(neigh.contains(&"O/a.md".into()));
}

#[test]
fn organizer_or_06() {
    assert!(split(vec![0.7, 0.7]).1 < 0.7);
}

#[test]
fn organizer_or_07() {
    let labelled = vec!["O/a.md".to_string(), "O/b.md".to_string()];
    let vecs: HashMap<String, Vec<f64>> = labelled.iter().map(|p| (p.clone(), vec![1.0, 0.0])).collect();
    assert!(!classify("O/a.md", &vecs, &labelled).2.contains(&"O/a.md".into()));
}

#[test]
fn organizer_or_08() {
    assert_eq!(classify("x.md", &HashMap::new(), &["O/a.md".into()]), (None, 0.0, vec![]));
    let vecs = HashMap::from([("x.md".to_string(), vec![1.0, 0.0])]);
    assert_eq!(classify("x.md", &vecs, &[]), (None, 0.0, vec![]));
}

fn moved_once(d: &tempfile::TempDir, st: &Path) -> Value {
    let v = open(d);
    let rec = move_note(&v, st, "Loose Orbit Note.md", "02 - Orbit/Loose Orbit Note.md", None).unwrap();
    record_move(&v, st, &rec, 0.85, &["02 - Orbit/Probe Ops.md".into()], &now()).unwrap();
    rec
}

#[test]
fn organizer_or_09() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    moved_once(&d, st.path());
    let journal = std::fs::read_to_string(st.path().join(JOURNAL)).unwrap();
    let last: Value = serde_json::from_str(journal.lines().last().unwrap()).unwrap();
    assert_eq!((last["from"].clone(), last["to"].clone(), last["share"].clone()), (json!("Loose Orbit Note.md"), json!("02 - Orbit/Loose Orbit Note.md"), json!(0.85)));
    let log = read(&d, LOG_NOTE);
    assert!(log.contains("Loose Orbit Note") && log.contains("0.85"));
    assert!(log.contains("moved [[02 - Orbit/Loose Orbit Note|Loose Orbit Note]] from \u{60}Loose Orbit Note.md\u{60} (share 0.85)"));
}

#[test]
fn organizer_or_10() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    moved_once(&d, st.path());
    std::fs::write(
        st.path().join("manifest.json"),
        r#"{"hashes": {}, "failures": {"02 - Orbit/Loose Orbit Note.md": {"error": "e", "attempts": 1}}}"#,
    )
    .unwrap();
    let v = open(&d);
    let r = undo_move(&v, st.path(), "02 - Orbit/Loose Orbit Note.md", &now()).unwrap();
    assert_eq!(r, json!({"restored": "Loose Orbit Note.md", "was": "02 - Orbit/Loose Orbit Note.md"}));
    assert!(d.path().join("Loose Orbit Note.md").is_file() && !d.path().join("02 - Orbit/Loose Orbit Note.md").exists());
    assert!(Manifest::load(st.path()).unwrap().failures.contains_key("Loose Orbit Note.md"));
    for p in ["02 - Orbit/Loose Orbit Note.md", "Nope.md"] {
        assert!(undo_move(&v, st.path(), p, &now()).unwrap_err().msg.contains("No undoable move"));
    }
    assert!(read(&d, LOG_NOTE).contains("UNDID move of \u{60}02 - Orbit/Loose Orbit Note.md\u{60} back to \u{60}Loose Orbit Note.md\u{60}"));
}

#[test]
fn organizer_or_11() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    let r = sweep(&d, st.path(), false, None);
    assert!(r["moved"].as_array().unwrap().iter().any(|m| m["from"] == "Loose Orbit Note.md" && m["to_folder"] == "02 - Orbit"));
    assert!(d.path().join("Loose Orbit Note.md").is_file());
    assert!(!st.path().join(JOURNAL).exists() && !st.path().join("fallback_embeddings.json").exists());
    assert_eq!(r["cache_rebuilt"], false);
}

#[test]
fn organizer_or_12() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    let r = sweep(&d, st.path(), true, None);
    assert!(d.path().join("02 - Orbit/Loose Orbit Note.md").is_file());
    assert!(st.path().join(JOURNAL).exists());
    assert_eq!(r["cache_rebuilt"], true);
    assert!(froms(&r).iter().all(|f| !f.starts_with("02 - Orbit/") && !f.starts_with("05 - Kitchen/")));
    assert_eq!(r.as_object().unwrap().keys().collect::<Vec<_>>(), ["moved", "proposals", "skipped", "cache_rebuilt"]);
}

#[test]
fn organizer_or_13() {
    let d = fixture();
    write_file(d.path(), "Fusion Orbit Kitchen.md", "orbit recipe fusion");
    let st = tempfile::tempdir().unwrap();
    let r = sweep(&d, st.path(), true, None);
    let p = proposal(&r, "Fusion Orbit Kitchen.md").unwrap();
    assert_eq!(p["reason"], "low confidence");
    assert!(d.path().join("Fusion Orbit Kitchen.md").is_file());
    let log = read(&d, LOG_NOTE);
    assert!(log.contains("- [ ] \u{60}Fusion Orbit Kitchen.md\u{60} \u{2192} **"), "{log}");
}

#[test]
fn organizer_or_14() {
    let d = fixture();
    write_file(d.path(), "05 - Kitchen/Loose Orbit Note.md", "recipe copy");
    let st = tempfile::tempdir().unwrap();
    let r = sweep(&d, st.path(), true, None);
    assert!(proposal(&r, "Loose Orbit Note.md").unwrap()["reason"].as_str().unwrap().contains("duplicate"));
    assert!(d.path().join("Loose Orbit Note.md").is_file());
}

#[test]
fn organizer_or_15() {
    let d = fixture();
    write_file(d.path(), "Second Orbit.md", "orbit second confident note");
    let st = tempfile::tempdir().unwrap();
    let r = sweep(&d, st.path(), true, Some(1));
    assert_eq!(r["moved"].as_array().unwrap().len(), 1);
    let capped: Vec<&Value> = r["skipped"].as_array().unwrap().iter().filter(|s| s["reason"] == "move cap reached").collect();
    assert_eq!(capped.len(), 1);
    assert!(d.path().join(capped[0]["path"].as_str().unwrap()).is_file());
    let d2 = fixture();
    let st2 = tempfile::tempdir().unwrap();
    assert!(sweep(&d2, st2.path(), true, Some(0))["moved"].as_array().unwrap().is_empty());
}

#[test]
fn organizer_or_16() {
    for apply in [false, true] {
        let d = fixture();
        write_file(d.path(), "Records/_schema.md", "---\nsheet: r\nfilename: x\nkey: [a]\ncolumns: {}\n---\n");
        write_file(d.path(), "Records/Row.md", "See [[Loose Orbit Note]].\n");
        let st = tempfile::tempdir().unwrap();
        let r = sweep(&d, st.path(), apply, None);
        assert!(!froms(&r).contains(&"Loose Orbit Note.md".to_string()), "apply={apply}");
        let reason = proposal(&r, "Loose Orbit Note.md").unwrap()["reason"].as_str().unwrap().to_string();
        assert!(reason.starts_with("linked from sheet rows"), "{reason}");
        assert_eq!(read(&d, "Records/Row.md"), "See [[Loose Orbit Note]].\n");
        assert!(d.path().join("Loose Orbit Note.md").is_file());
    }
}

// ---------------------------------------------------------------- suggestions block (09 §3.1)

const FUSION: &str = "Fusion Orbit Kitchen.md";

fn day_at(d: u32) -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 7, d).unwrap().and_hms_opt(12, 0, 0).unwrap()
}

fn sweep_on(d: &tempfile::TempDir, st: &Path, apply: bool, day: u32) -> Value {
    run(&open(d), st, &FnEmb(cluster), apply, None, &day_at(day)).unwrap()
}

fn fusion() -> tempfile::TempDir {
    let d = fixture();
    write_file(d.path(), FUSION, "orbit recipe fusion");
    d
}

fn state_text(st: &Path) -> String {
    std::fs::read_to_string(st.join(SUGGESTIONS_FILE)).unwrap()
}

fn line(path: &str, day: &str) -> String {
    format!("- [ ] \u{60}{path}\u{60} \u{2192} **02 - Orbit** (low confidence) \u{2014} proposed {day}\n")
}

#[test]
fn organizer_or_17() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    let log = read(&d, LOG_NOTE);
    let block = format!(
        "{BLOCK_START}\n## Suggestions\n\nTick a box to dismiss a suggestion; it will not be proposed again. Enigma never moves a note because of this list.\n\n{}{BLOCK_END}\n",
        line(FUSION, "2026-07-05")
    );
    assert_eq!(log.matches(BLOCK_START).count(), 1);
    assert!(log.contains(&format!("{block}\n## Log\n")), "{log}");
    assert!(!log.contains("### Proposals"));
    let s = load_suggestions(st.path()).unwrap();
    assert_eq!((s.open.len(), s.open[0].path.as_str(), s.open[0].proposed.as_str()), (1, FUSION, "2026-07-05"));
}

#[test]
fn organizer_or_18() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    let (log, state) = (read(&d, LOG_NOTE), state_text(st.path()));
    sweep_on(&d, st.path(), true, 6);
    assert_eq!(read(&d, LOG_NOTE), log);
    assert_eq!(state_text(st.path()), state);
}

#[test]
fn organizer_or_19() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    write_file(d.path(), "Fusion Two Orbit Kitchen.md", "orbit recipe two");
    sweep_on(&d, st.path(), true, 6);
    let log = read(&d, LOG_NOTE);
    let (first, second) = (log.find(&line(FUSION, "2026-07-05")).unwrap(), log.find(&line("Fusion Two Orbit Kitchen.md", "2026-07-06")).unwrap());
    assert!(first < second);
    std::fs::remove_file(d.path().join(FUSION)).unwrap();
    sweep_on(&d, st.path(), true, 7);
    let log = read(&d, LOG_NOTE);
    assert!(!log.contains(&line(FUSION, "2026-07-05")));
    assert!(log.ends_with(&format!("- 2026-07-07 12:00:00 \u{2014} suggestion dropped (note moved or deleted): \u{60}{FUSION}\u{60} \u{2192} **02 - Orbit**\n")), "{log}");
    write_file(d.path(), "Fusion Two Orbit Kitchen.md", "---\norganize: false\n---\n\norbit recipe two\n");
    sweep_on(&d, st.path(), true, 8);
    let log = read(&d, LOG_NOTE);
    assert!(log.contains("suggestion dropped (no longer applies): \u{60}Fusion Two Orbit Kitchen.md\u{60}"));
    assert!(log.contains("_No open suggestions._\n"));
}

#[test]
fn organizer_or_20() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    let ticked = read(&d, LOG_NOTE).replace(&format!("- [ ] \u{60}{FUSION}"), &format!("- [x] \u{60}{FUSION}"));
    std::fs::write(d.path().join(LOG_NOTE), &ticked).unwrap();
    // a dry run already honours the tick, and writes nothing
    let r = sweep_on(&d, st.path(), false, 6);
    assert!(proposal(&r, FUSION).is_none());
    assert_eq!(read(&d, LOG_NOTE), ticked);
    let r = sweep_on(&d, st.path(), true, 6);
    assert!(proposal(&r, FUSION).is_none());
    assert!(r["skipped"].as_array().unwrap().contains(&json!({"path": FUSION, "reason": "suggestion acknowledged"})));
    let log = read(&d, LOG_NOTE);
    assert!(!log.contains(&format!("\u{60}{FUSION}\u{60} \u{2192} **02 - Orbit** (low")), "{log}");
    assert!(log.ends_with(&format!("- 2026-07-06 12:00:00 \u{2014} suggestion acknowledged: \u{60}{FUSION}\u{60} \u{2192} **02 - Orbit**\n")));
    let s = load_suggestions(st.path()).unwrap();
    assert_eq!(s.acknowledged, vec![Acknowledged { path: FUSION.into(), suggested: "02 - Orbit".into(), at: "2026-07-06 12:00:00".into() }]);
    assert!(d.path().join(FUSION).is_file(), "never moved");
    let before = read(&d, LOG_NOTE);
    let r = sweep_on(&d, st.path(), true, 9);
    assert!(proposal(&r, FUSION).is_none());
    assert_eq!(read(&d, LOG_NOTE), before);
}

#[test]
fn organizer_or_21() {
    assert_eq!(SUGGESTION_TTL_DAYS, 15);
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    sweep_on(&d, st.path(), true, 19);
    assert!(read(&d, LOG_NOTE).contains(&line(FUSION, "2026-07-05")), "14 days: still there");
    sweep_on(&d, st.path(), true, 20);
    let log = read(&d, LOG_NOTE);
    assert!(!log.contains(&format!("\u{60}{FUSION}\u{60} \u{2192} **02 - Orbit** (low")), "{log}");
    assert!(log.ends_with(&format!("- 2026-07-20 12:00:00 \u{2014} suggestion expired after 15 days: \u{60}{FUSION}\u{60} \u{2192} **02 - Orbit**\n")));
    sweep_on(&d, st.path(), true, 21);
    assert!(read(&d, LOG_NOTE).contains(&line(FUSION, "2026-07-21")), "re-proposed with a fresh date");
}

#[test]
fn organizer_or_22() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    let log = read(&d, LOG_NOTE);
    let (a, b) = (log.find(BLOCK_START).unwrap(), log.find(BLOCK_END).unwrap() + BLOCK_END.len());
    let edited = format!(
        "{}Harvey above the block.\n{}junk typed inside\n{}\nHarvey between.\n{}Harvey after the log.\n",
        &log[..a],
        &log[a..b - BLOCK_END.len()],
        &log[b - BLOCK_END.len()..b],
        &log[b..]
    );
    std::fs::write(d.path().join(LOG_NOTE), &edited).unwrap();
    write_file(d.path(), "Fusion Two Orbit Kitchen.md", "orbit recipe two");
    sweep_on(&d, st.path(), true, 6);
    let log = read(&d, LOG_NOTE);
    for kept in ["Harvey above the block.\n", "\nHarvey between.\n", "Harvey after the log.\n", "## Log\n"] {
        assert!(log.contains(kept), "{kept:?} in {log}");
    }
    assert!(!log.contains("junk typed inside"));
    assert!(log.contains(&line("Fusion Two Orbit Kitchen.md", "2026-07-06")));
}

#[test]
fn organizer_or_23() {
    let d = vault_with(&[("02 - Orbit/A.md", "orbit a"), ("05 - Kitchen/B.md", "recipe b")]);
    let st = tempfile::tempdir().unwrap();
    sweep_on(&d, st.path(), true, 5);
    assert!(!d.path().join(LOG_NOTE).exists());
    assert!(!st.path().join(SUGGESTIONS_FILE).exists());
}

#[test]
fn organizer_or_24() {
    let d = fusion();
    let st = tempfile::tempdir().unwrap();
    let r = sweep_on(&d, st.path(), false, 5);
    assert!(proposal(&r, FUSION).is_some());
    assert!(!d.path().join(LOG_NOTE).exists());
    assert!(!st.path().join(SUGGESTIONS_FILE).exists());
}
