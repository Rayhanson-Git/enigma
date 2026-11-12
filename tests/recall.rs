//! acceptance/recall.yaml
mod common;
use common::*;
use enigma_mcp::graph;
use enigma_mcp::recall::*;
use enigma_mcp::text;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

fn dig(d: &tempfile::TempDir, st: &Path, since: Option<&str>) -> Value {
    digest(&open(d), st, since, &text::now()).unwrap()
}

fn paths(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|n| n["path"].as_str().unwrap().to_string()).collect()
}

fn texts_of(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|t| t["text"].as_str().unwrap().to_string()).collect()
}

fn session(d: &tempfile::TempDir, name: &str, created: &str, project: &str, body: &str) {
    write_file(d.path(), &format!("Claude/Sessions/{name}.md"), &format!("---\ncreated: {created}\nproject: {project}\n---\n\n{body}\n"));
}

#[test]
fn recall_rc_01() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let b = dig(&d, st.path(), None);
    assert_eq!(
        b.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["mode", "generated", "since", "librarian", "recent_notes", "inbox_captures", "tasks", "proposals", "new_entities"]
    );
    assert_eq!(b["mode"], "digest");
    for k in ["librarian", "recent_notes", "inbox_captures", "tasks", "proposals", "new_entities"] {
        assert_eq!(b[k]["status"], "ok", "{k}: {}", b[k]);
    }
}

#[test]
fn recall_rc_02() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Inbox/2026-07-10.md", "- 10:00 fresh\n");
    let st = tempfile::tempdir().unwrap();
    assert!(paths(&dig(&d, st.path(), None)["inbox_captures"]["notes"]).contains(&"Claude/Inbox/2026-07-10.md".into()));
}

#[test]
fn recall_rc_03() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Inbox/2020-01-02.md", "- old\n");
    let f = std::fs::File::options().write(true).open(d.path().join("Claude/Inbox/2020-01-02.md")).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(1_577_959_200)).unwrap();
    let st = tempfile::tempdir().unwrap();
    let b = dig(&d, st.path(), Some("2026-01-01"));
    assert_eq!(b["since"], "2026-01-01");
    assert!(!paths(&b["inbox_captures"]["notes"]).contains(&"Claude/Inbox/2020-01-02.md".into()));
}

#[test]
fn recall_rc_04() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let e = digest(&open(&d), st.path(), Some("last tuesday"), &text::now()).unwrap_err();
    assert_eq!(e.msg, "since must be YYYY-MM-DD: 'last tuesday'");
    assert!(digest(&open(&d), st.path(), Some("2026-02-30"), &text::now()).is_err());
}

#[test]
fn recall_rc_05() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Tasks.md", "# Tasks\n\n- [ ] open item\n- [x] finished item\n");
    let st = tempfile::tempdir().unwrap();
    let t = &dig(&d, st.path(), None)["tasks"];
    assert_eq!(texts_of(&t["open"]), ["open item"]);
    assert_eq!(texts_of(&t["done_recently"]), ["finished item"]);
}

#[test]
fn recall_rc_06() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    std::fs::create_dir(st.path().join("librarian_state.json")).unwrap(); // reading it fails
    let b = dig(&d, st.path(), None);
    assert_eq!(b["librarian"]["status"], "error");
    assert!(b["librarian"]["error"].as_str().unwrap().starts_with("IoError: "));
    assert_eq!(b["recent_notes"]["status"], "ok");
}

#[test]
fn recall_rc_07() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let p = &dig(&d, st.path(), None)["proposals"];
    assert_eq!((p["pending"].clone(), p["detail_note"].clone()), (json!(0), json!("Claude/Organizer.md")));
    std::fs::write(st.path().join("librarian_state.json"), r#"{"health": {"pending_proposals": 3}}"#).unwrap();
    assert_eq!(dig(&d, st.path(), None)["proposals"]["pending"], 3);
}

fn three_sessions(d: &tempfile::TempDir) {
    session(d, "2026-07-01 Graph work", "2026-07-01 10:00", "enigma-mcp", "Built the graph.");
    session(d, "2026-07-09 Evals", "2026-07-09 10:00", "enigma-mcp", "Wrote the eval harness.");
    session(d, "2026-07-05 Other", "2026-07-05 10:00", "lighthouse", "Unrelated.");
}

#[test]
fn recall_rc_08() {
    let d = mini_vault();
    three_sessions(&d);
    let st = tempfile::tempdir().unwrap();
    let b = resume(&open(&d), st.path(), "enigma", 10);
    assert_eq!((b["mode"].clone(), b["project"].clone()), (json!("resume"), json!("enigma")));
    let notes = &b["sessions"]["notes"];
    assert_eq!(paths(notes), ["Claude/Sessions/2026-07-09 Evals.md", "Claude/Sessions/2026-07-01 Graph work.md"]);
    let ex = notes[0]["excerpt"].as_str().unwrap();
    assert!(ex.contains("Wrote the eval harness.") && !ex.contains("---"));
    assert_eq!(b.as_object().unwrap().keys().collect::<Vec<_>>(), ["mode", "project", "sessions", "decisions", "tasks", "entities"]);
}

#[test]
fn recall_rc_16() {
    let d = mini_vault();
    session(&d, "A", "2026-07-01 10:00", "enigma", "a");
    session(&d, "B", "2026-07-09 10:00", "enigma", "b");
    session(&d, "C", "July 1", "enigma", "c");
    let st = tempfile::tempdir().unwrap();
    let b = resume(&open(&d), st.path(), "enigma", 10);
    assert_eq!(paths(&b["sessions"]["notes"]), ["Claude/Sessions/C.md", "Claude/Sessions/B.md", "Claude/Sessions/A.md"]);
}

#[test]
fn recall_rc_09() {
    let d = mini_vault();
    for i in 1..=4 {
        session(&d, &format!("2026-07-0{i} S"), &format!("2026-07-0{i} 10:00"), "enigma", "b");
    }
    let st = tempfile::tempdir().unwrap();
    assert_eq!(resume(&open(&d), st.path(), "enigma", 2)["sessions"]["notes"].as_array().unwrap().len(), 2);
}

#[test]
fn recall_rc_10() {
    let d = mini_vault();
    let line = "- 2026-07-08 \u{2014} hybrid search ships in enigma ([[x]])";
    write_file(d.path(), "Claude/Decisions.md", &format!("# Decisions\n\n{line}\n- 2026-07-09 \u{2014} lighthouse retired\n"));
    write_file(d.path(), "Claude/Tasks.md", "- [ ] tune enigma eval gate\n- [ ] water plants\n- [x] enigma done thing\n");
    let st = tempfile::tempdir().unwrap();
    let b = resume(&open(&d), st.path(), "Enigma", 10);
    assert_eq!(b["decisions"]["lines"], json!([line]));
    assert_eq!(texts_of(&b["tasks"]["tasks"]), ["tune enigma eval gate"]);
}

#[test]
fn recall_rc_11() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let b = resume(&open(&d), st.path(), "x", 10);
    assert_eq!(b["decisions"], json!({"lines": [], "status": "ok"}));
}

#[test]
fn recall_rc_12() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let b = resume(&open(&d), st.path(), "x", 10);
    assert_eq!((b["entities"]["status"].clone(), b["entities"]["entities"].clone()), (json!("ok"), json!([])));
}

#[test]
fn recall_rc_13() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    write_file(
        d.path(),
        "Claude/Graph/Projects/Enigma.md",
        "---\nentity: project\naliases: [enigma-mcp]\n---\n\n# Enigma\n\nThe mind database project.\n\n## Mentions\n\n## Relations\n",
    );
    graph::rebuild(&open(&d), &st.path().join("graph.db")).unwrap();
    let b = resume(&open(&d), st.path(), "enigma", 10);
    assert_eq!(
        b["entities"]["entities"],
        json!([{"name": "Enigma", "type": "project", "path": "Claude/Graph/Projects/Enigma.md", "summary": "The mind database project."}])
    );
}

#[test]
fn recall_rc_14() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let v = open(&d);
    enigma_mcp::notes::log_session(&v, "Fixed the flux capacitor", "Recalibrated the flux capacitor.", "enigma-mcp", &[], &now(), "claude").unwrap();
    let block = context_block(&v, st.path(), "enigma-mcp", 2000);
    assert!(block.contains("flux capacitor") && block.chars().count() <= 2000 && !block.ends_with('\n'));
    assert!(block.starts_with("Session: 2026-07-05 Fixed the flux capacitor \u{2014} "));
    session(&d, "2026-07-06 Long", "2026-07-06 10:00", "enigma-mcp", &"long words ".repeat(500));
    assert!(context_block(&v, st.path(), "enigma-mcp", 500).chars().count() <= 500);
    assert_eq!(context_block(&v, st.path(), "", 2000), "");
}

#[test]
fn recall_rc_15() {
    let out = Command::new(bin()).args(["recall", "--vault", "/nonexistent", "--context"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}
