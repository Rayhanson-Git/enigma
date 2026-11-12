//! acceptance/cleanup.yaml (NOW = 2026-07-13 12:00)
mod common;
use common::*;
use chrono::{NaiveDate, NaiveDateTime};
use enigma_mcp::cleanup::*;
use enigma_mcp::graph::{self, Entity, Extraction, Graph, resolve_redirect};
use enigma_mcp::indexer::{Failure, Manifest};
use enigma_mcp::vault::{get, parse, str_list};
use enigma_mcp::yaml::Y;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn now13() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 13).unwrap().and_hms_opt(12, 0, 0).unwrap()
}

fn org(name: &str) -> Entity {
    Entity { name: name.into(), ty: "organization".into(), aliases: vec![], summary: "S.".into() }
}

const ORGS: &str = "Claude/Graph/Organizations";

fn ent(name: &str) -> String {
    format!("{ORGS}/{name}")
}

/// A raw organization note with optional extra frontmatter and section lines.
fn note(d: &tempfile::TempDir, name: &str, extra_fm: &str, mentions: &str, relations: &str) {
    write_file(
        d.path(),
        &format!("{ORGS}/{name}.md"),
        &format!("---\nentity: organization\n{extra_fm}---\n\n# {name}\n\nS.\n\n## Mentions\n{mentions}\n## Relations\n{relations}"),
    );
}

fn stub(d: &tempfile::TempDir, name: &str, target: &str) {
    note(d, name, &format!("merged_into: {ORGS}/{target}\n"), "", "");
}

fn track(st: &Path, hashes: &[&str], failures: &[&str]) {
    let mut m = Manifest::default();
    for h in hashes {
        m.hashes.insert(h.to_string(), "h".into());
    }
    for f in failures {
        m.failures.insert(f.to_string(), Failure { error: "e".into(), attempts: 1 });
    }
    m.save(st).unwrap();
}

/// Indexes note with entity Acme, rebuilds the cache, then deletes the note.
fn doomed(d: &tempfile::TempDir, st: &Path, notes: &[&str], with_db: bool) {
    let v = open(d);
    for n in notes {
        write_file(d.path(), n, "doomed");
        graph::apply(&v, n, &Extraction { entities: vec![org("Acme")], relations: vec![] }, &now13()).unwrap();
    }
    track(st, notes, &[]);
    if with_db {
        graph::rebuild(&v, &st.join("graph.db")).unwrap();
    }
    for n in notes {
        std::fs::remove_file(d.path().join(n)).unwrap();
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

#[test]
fn cleanup_cl_01() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    write_file(d.path(), "Projects/Kept.md", "kept");
    track(st.path(), &["Projects/Kept.md", "Projects/Gone.md"], &["Projects/Benched.md"]);
    assert_eq!(deleted_notes(&open(&d), st.path()).unwrap().0, ["Projects/Benched.md", "Projects/Gone.md"]);
    assert!(suspect(25, 100) && !suspect(25, 1000) && !suspect(19, 19));
}

#[test]
fn cleanup_cl_02() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    doomed(&d, st.path(), &["Projects/Doomed.md"], true);
    let r = retract_deleted(&open(&d), st.path(), 100, false).unwrap();
    assert_eq!(r, json!({"retracted_notes": 1, "removed_mentions": 1, "remaining": 0}));
    assert!(!read(&d, &format!("{ORGS}/Acme.md")).contains("[[Projects/Doomed|"));
    assert!(!Manifest::load(st.path()).unwrap().hashes.contains_key("Projects/Doomed.md"));
}

#[test]
fn cleanup_cl_03() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    doomed(&d, st.path(), &["Projects/D1.md", "Projects/D2.md", "Projects/D3.md"], true);
    let r = retract_deleted(&open(&d), st.path(), 2, false).unwrap();
    assert_eq!((r["retracted_notes"].clone(), r["remaining"].clone()), (json!(2), json!(1)));
}

#[test]
fn cleanup_cl_04() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    doomed(&d, st.path(), &["Projects/Doomed.md"], false);
    assert!(!st.path().join("graph.db").exists());
    assert_eq!(retract_deleted(&open(&d), st.path(), 100, false).unwrap()["removed_mentions"], 1);
}

#[test]
fn cleanup_cl_05() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    doomed(&d, st.path(), &["Projects/Doomed.md"], true);
    std::fs::remove_file(d.path().join(format!("{ORGS}/Acme.md"))).unwrap();
    let r = retract_deleted(&open(&d), st.path(), 100, false).unwrap();
    assert_eq!(r, json!({"retracted_notes": 1, "removed_mentions": 0, "remaining": 0}));
    assert!(Manifest::load(st.path()).unwrap().hashes.is_empty());
}

#[test]
fn cleanup_cl_06() {
    let d = mini_vault();
    let v = open(&d);
    graph::upsert_entity(&v, &Entity { name: "Acme".into(), ty: "organization".into(), aliases: vec!["ACME Inc".into()], summary: "Maker of anvils.".into() }, &now13()).unwrap();
    let rel = format!("{ORGS}/Acme.md");
    retire_note(&v, &rel, &now13(), RETIRE_ORPHAN).unwrap();
    let t = read(&d, &rel);
    let meta = parse(&t).unwrap();
    assert_eq!(get(&meta, "retired"), Some(&Y::Str("2026-07-13 12:00".into())));
    assert_eq!(str_list(get(&meta, "aliases")), ["ACME Inc"]);
    assert!(t.contains("Maker of anvils.") && t.contains("Retired: orphaned \u{2014} no mentions or relations.\n"));
}

#[test]
fn cleanup_cl_07() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let v = open(&d);
    graph::upsert_entity(&v, &org("Acme"), &now13()).unwrap();
    retire_note(&v, &format!("{ORGS}/Acme.md"), &now13(), RETIRE_ORPHAN).unwrap();
    assert!(enigma_mcp::consolidate::gather(&v).unwrap().iter().all(|e| e.name != "Acme"));
    graph::rebuild(&v, &st.path().join("graph.db")).unwrap();
    assert!(Graph::load(&st.path().join("graph.db")).unwrap().find("Acme", None).is_empty());
}

#[test]
fn cleanup_cl_08() {
    let d = mini_vault();
    let v = open(&d);
    note(&d, "Canonical", "", "", "");
    stub(&d, "Mid", "Canonical");
    stub(&d, "Old", "Mid");
    assert_eq!(resolve_redirect(&v, &ent("Old")), Some(ent("Canonical")));
    stub(&d, "A", "B");
    stub(&d, "B", "A");
    assert_eq!(resolve_redirect(&v, &ent("A")), None);
    assert_eq!(resolve_redirect(&v, &ent("Missing")), None);
    note(&d, "N6", "", "", "");
    for i in 0..6 {
        stub(&d, &format!("N{i}"), &format!("N{}", i + 1));
    }
    assert_eq!(resolve_redirect(&v, &ent("N0")), None);
    assert_eq!(resolve_redirect(&v, &ent("N4")), Some(ent("N6")));
}

fn uses(target: &str) -> String {
    format!("- uses [[{ORGS}/{target}|{target}]]\n")
}

#[test]
fn cleanup_cl_09() {
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    stub(&d, "Dup", "Canonical");
    note(&d, "Source", "", "", &uses("Dup"));
    assert_eq!(repair_relations(&open(&d), 200, false).unwrap(), (1, 0));
    assert!(read(&d, &format!("{ORGS}/Source.md")).contains("- uses [[Claude/Graph/Organizations/Canonical|Canonical]]\n"));
    let d = mini_vault();
    note(&d, "Source", "", "", &uses("Ghost"));
    assert_eq!(repair_relations(&open(&d), 200, false).unwrap(), (0, 1));
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    stub(&d, "Dup", "Canonical");
    note(&d, "Source", "", "", &format!("{}{}", uses("Canonical"), uses("Dup")));
    assert_eq!(repair_relations(&open(&d), 200, false).unwrap(), (0, 1));
    assert_eq!(read(&d, &format!("{ORGS}/Source.md")).matches("[[Claude/Graph/Organizations/Canonical|").count(), 1);
    let d = mini_vault();
    note(&d, "Source", "", "", &format!("{}{}", uses("Ghost1"), uses("Ghost2")));
    let (f, r) = repair_relations(&open(&d), 1, false).unwrap();
    assert_eq!(f + r, 1);
}

#[test]
fn cleanup_cl_10() {
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    note(&d, "Stubby", &format!("merged_into: {ORGS}/Canonical\n"), "", &uses("Ghost"));
    note(&d, "Tomb", "retired: 2026-07-01 10:00\n", "", &uses("Ghost"));
    assert_eq!(repair_relations(&open(&d), 200, false).unwrap(), (0, 0));
}

#[test]
fn cleanup_cl_11() {
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    stub(&d, "Mid", "Canonical");
    stub(&d, "Old", "Mid");
    assert_eq!(flatten_stubs(&open(&d), &now13(), false).unwrap(), (1, 0));
    let t = read(&d, &format!("{ORGS}/Old.md"));
    assert!(t.contains(&format!("merged_into: {ORGS}/Canonical")) && t.contains("Merged into [[Canonical]]."));
    let d = mini_vault();
    stub(&d, "Lost", "Ghost");
    assert_eq!(flatten_stubs(&open(&d), &now13(), false).unwrap(), (0, 1));
    assert_eq!(get(&parse(&read(&d, &format!("{ORGS}/Lost.md"))).unwrap(), "retired"), Some(&Y::Str("2026-07-13 12:00".into())));
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    stub(&d, "Direct", "Canonical");
    assert_eq!(flatten_stubs(&open(&d), &now13(), false).unwrap(), (0, 0));
}

fn orphan_fixture() -> tempfile::TempDir {
    let d = mini_vault();
    note(&d, "Lonely", "", "", "");
    note(&d, "Mentioned", "", "- [[Daily|Daily]]\n", "");
    note(&d, "Source", "", "- [[Daily|Daily]]\n", &uses("Endpoint"));
    note(&d, "Endpoint", "", "", "");
    stub(&d, "Stubbed", "Source");
    note(&d, "Tomb", "retired: 2026-07-01 10:00\n", "", "");
    d
}

fn orphan_paths(v: &[Value]) -> Vec<String> {
    v.iter().map(|o| o["path"].as_str().unwrap().to_string()).collect()
}

#[test]
fn cleanup_cl_12() {
    let d = orphan_fixture();
    let o = find_orphans(&open(&d)).unwrap();
    assert_eq!(orphan_paths(&o), [ent("Lonely")]);
    assert_eq!(o[0], json!({"path": ent("Lonely"), "name": "Lonely", "type": "organization", "reason": "orphaned: no mentions or relations"}));
}

#[test]
fn cleanup_cl_13() {
    let mut block = Map::new();
    block.insert("pending_retirements".into(), json!([{"path": "gone-now"}]));
    let orphans: Vec<Value> = (0..3).map(|i| json!({"path": format!("o{i}")})).collect();
    let pending = update_retirement_proposals(&mut block, &orphans, 2);
    assert_eq!(orphan_paths(&pending), ["o0", "o1"]);
    assert_eq!(block["pending_retirements"], Value::Array(pending));
}

#[test]
fn cleanup_cl_14() {
    let d = orphan_fixture();
    let st = tempfile::tempdir().unwrap();
    let r = apply_retirements(&open(&d), st.path(), None, &now13()).unwrap();
    assert_eq!(r, json!({"retired": [ent("Lonely")]}));
    assert!(get(&parse(&read(&d, &format!("{ORGS}/Lonely.md"))).unwrap(), "retired").is_some());
    assert!(Graph::load(&st.path().join("graph.db")).unwrap().find("Lonely", None).is_empty());
    let d = mini_vault();
    note(&d, "LonelyA", "", "", "");
    note(&d, "LonelyB", "", "", "");
    let r = apply_retirements(&open(&d), st.path(), Some(&[ent("LonelyA")]), &now13()).unwrap();
    assert_eq!(r, json!({"retired": [ent("LonelyA")]}));
    assert!(get(&parse(&read(&d, &format!("{ORGS}/LonelyB.md"))).unwrap(), "retired").is_none());
    assert_eq!(apply_retirements(&open(&d), st.path(), Some(&[]), &now13()).unwrap(), json!({"retired": []}));
}

#[test]
fn cleanup_cl_15() {
    let mut block = Map::new();
    block.insert("checked_hash".into(), json!({"live": "h1", "gone": "h2"}));
    let live: BTreeSet<String> = ["live".to_string()].into();
    assert_eq!(prune_checked_hash(&mut block, &live), 1);
    assert_eq!(block["checked_hash"], json!({"live": "h1"}));
    assert_eq!(prune_checked_hash(&mut Map::new(), &live), 0);
}

#[test]
fn cleanup_cl_16() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let notes: Vec<String> = (0..20).map(|i| format!("Projects/Gone{i}.md")).collect();
    let refs: Vec<&str> = notes.iter().map(String::as_str).collect();
    track(st.path(), &refs, &[]);
    let before = std::fs::read(st.path().join("manifest.json")).unwrap();
    let r = retract_deleted(&open(&d), st.path(), 100, false).unwrap();
    assert_eq!((r["suspect_mass_deletion"].clone(), r["retracted_notes"].clone()), (json!(true), json!(0)));
    assert_eq!(std::fs::read(st.path().join("manifest.json")).unwrap(), before);
    assert_eq!(retract_deleted(&open(&d), st.path(), 100, true).unwrap()["retracted_notes"], 20);
}

#[test]
fn cleanup_cl_17() {
    let d = mini_vault();
    note(&d, "Canonical", "", "", "");
    stub(&d, "Mid", "Canonical");
    stub(&d, "Old", "Mid");
    note(&d, "Source", "", "", &uses("Ghost"));
    let before = snapshot(d.path());
    assert_eq!(repair_relations(&open(&d), 200, true).unwrap(), (0, 1));
    assert_eq!(flatten_stubs(&open(&d), &now13(), true).unwrap(), (1, 0));
    assert_eq!(snapshot(d.path()), before);
}
