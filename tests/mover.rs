//! acceptance/mover.yaml
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::faults::{self, Fault};
use enigma_mcp::indexer::Manifest;
use enigma_mcp::mover::*;
use std::collections::BTreeMap;
use std::path::Path;

const RESEARCH: &str = "See [[Signal]] and [[Signal|the signal note]] and [[Signal#Details]].\n";
const TOPIC: &str = "## Mentions\n\n- [[Signal|Signal]] \u{2014} evidence\n";

fn fixture() -> tempfile::TempDir {
    vault_with(&[
        ("Signal.md", "# Signal\n\nBody [[Signal#Top]].\n"),
        ("02 - Orbit/Research.md", RESEARCH),
        ("Signal 2.md", "# Signal 2\n"),
        ("02 - Orbit/Mentions2.md", "See [[Signal 2]] too.\n"),
        ("Claude/Graph/Topics/Signal Topic.md", TOPIC),
    ])
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| (e.path().display().to_string(), std::fs::read(e.path()).unwrap()))
        .collect()
}

fn mv(d: &tempfile::TempDir, st: &Path) -> serde_json::Value {
    move_note(&open(d), st, "Signal.md", "02 - Orbit/Signal.md", None).unwrap()
}

#[test]
fn mover_mv_01() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    let r = mv(&d, st.path());
    assert_eq!((r["from"].as_str(), r["to"].as_str()), (Some("Signal.md"), Some("02 - Orbit/Signal.md")));
    assert!(!d.path().join("Signal.md").exists() && d.path().join("02 - Orbit/Signal.md").is_file());
    let research = read(&d, "02 - Orbit/Research.md");
    for frag in ["[[02 - Orbit/Signal]]", "[[02 - Orbit/Signal|the signal note]]", "[[02 - Orbit/Signal#Details]]"] {
        assert!(research.contains(frag), "{frag}");
    }
    assert!(read(&d, "Claude/Graph/Topics/Signal Topic.md").contains("[[02 - Orbit/Signal|Signal]]"));
    let paths: Vec<&str> = r["rewrites"].as_array().unwrap().iter().map(|w| w["path"].as_str().unwrap()).collect();
    assert!(paths.contains(&"02 - Orbit/Research.md") && paths.contains(&"Claude/Graph/Topics/Signal Topic.md"));
}

#[test]
fn mover_mv_02() {
    let d = fixture();
    mv(&d, tempfile::tempdir().unwrap().path());
    assert_eq!(read(&d, "02 - Orbit/Mentions2.md"), "See [[Signal 2]] too.\n");
}

#[test]
fn mover_mv_03() {
    let d = fixture();
    let st = tempfile::tempdir().unwrap();
    std::fs::write(st.path().join("manifest.json"), r#"{"hashes": {"Signal.md": "h"}, "failures": {"Signal.md": {"error": "e", "attempts": 2}}}"#).unwrap();
    mv(&d, st.path());
    let m = Manifest::load(st.path()).unwrap();
    assert_eq!(m.hashes.get("02 - Orbit/Signal.md").map(String::as_str), Some("h"));
    assert_eq!(m.failures["02 - Orbit/Signal.md"].attempts, 2);
    assert!(!m.hashes.contains_key("Signal.md") && !m.failures.contains_key("Signal.md"));
}

#[test]
fn mover_mv_04() {
    let d = fixture();
    write_file(d.path(), "Clone.md", "a");
    write_file(d.path(), "02 - Orbit/Clone.md", "b");
    let v = open(&d);
    assert!(duplicate_stem_exists(&v, "Clone.md").unwrap());
    assert!(!duplicate_stem_exists(&v, "Signal.md").unwrap());
}

#[test]
fn mover_mv_05() {
    let d = fixture();
    mv(&d, tempfile::tempdir().unwrap().path());
    reverse_rewrites(&open(&d), "Signal.md", "02 - Orbit/Signal.md", &["02 - Orbit/Research.md".into()]).unwrap();
    assert_eq!(read(&d, "02 - Orbit/Research.md"), RESEARCH);
}

#[test]
fn mover_mv_06() {
    faults::reset();
    let d = fixture();
    let before = snapshot(d.path());
    faults::arm("move_rename", Fault::Os(5), 1);
    assert!(move_note(&open(&d), tempfile::tempdir().unwrap().path(), "Signal.md", "02 - Orbit/Signal.md", None).is_err());
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn mover_mv_07() {
    faults::reset();
    let d = fixture();
    let before = snapshot(d.path());
    faults::arm("move_write", Fault::Short(0), 1); // first rewrite goes through
    faults::arm("move_write", Fault::Os(5), 1); // the second one fails
    assert!(move_note(&open(&d), tempfile::tempdir().unwrap().path(), "Signal.md", "02 - Orbit/Signal.md", None).is_err());
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn mover_mv_08() {
    let d = fixture();
    std::fs::write(d.path().join("02 - Orbit/Raw.md"), b"Line\r\n[[Signal]] \xff\xfe end\r\n").unwrap();
    mv(&d, tempfile::tempdir().unwrap().path());
    assert_eq!(std::fs::read(d.path().join("02 - Orbit/Raw.md")).unwrap(), b"Line\r\n[[02 - Orbit/Signal]] \xff\xfe end\r\n");
}

#[test]
fn mover_mv_09() {
    let d = fixture();
    mv(&d, tempfile::tempdir().unwrap().path());
    assert!(read(&d, "02 - Orbit/Signal.md").contains("[[02 - Orbit/Signal#Top]]"));
}

#[test]
fn mover_mv_10() {
    let d = fixture();
    let before = snapshot(d.path());
    let protect = |p: &str| p.starts_with("Claude/Graph/");
    let e = move_note(&open(&d), tempfile::tempdir().unwrap().path(), "Signal.md", "02 - Orbit/Signal.md", Some(&protect)).unwrap_err();
    assert_eq!(e.kind, Kind::ProtectedLink);
    assert_eq!(protected_paths(&e), ["Claude/Graph/Topics/Signal Topic.md"]);
    assert_eq!(snapshot(d.path()), before);
}
