//! acceptance/consolidate.yaml (fake embedder [len(text), 1.0], fake backend)
mod common;
use common::*;
use enigma_mcp::consolidate::*;
use enigma_mcp::error::{Error, Kind, Result};
use enigma_mcp::extractor::Complete;
use enigma_mcp::graph::{self, Entity, Extraction, Graph, Relation, entity_summary};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::Mutex;

fn emb() -> FnEmb {
    FnEmb(|t| vec![t.chars().count() as f64, 1.0])
}

/// Replies in order (repeating the last), Err for None; records prompts.
struct Backend {
    replies: Mutex<VecDeque<Option<Value>>>,
    prompts: Mutex<Vec<String>>,
}

impl Backend {
    fn new(replies: Vec<Option<Value>>) -> Self {
        Backend { replies: Mutex::new(replies.into()), prompts: Mutex::default() }
    }
}

impl Complete for Backend {
    fn complete_json(&self, prompt: &str) -> Result<Value> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        let mut q = self.replies.lock().unwrap();
        let r = if q.len() > 1 { q.pop_front().unwrap() } else { q.front().cloned().unwrap_or(None) };
        r.ok_or_else(|| Error::new(Kind::ExtractorError, "backend down"))
    }
}

const CANON: &str = "Claude/Graph/Organizations/Cedar Cloud.md";
const DUP: &str = "Claude/Graph/Organizations/Cedar Cloud deploy.md";

fn merge_reply() -> Value {
    json!({"merges": [{"type": "organization", "canonical": "Cedar Cloud", "duplicates": ["Cedar Cloud deploy"]}]})
}

fn seed() -> tempfile::TempDir {
    let d = mini_vault();
    let v = open(&d);
    write_file(d.path(), "A.md", "Cedar Cloud runs our VM.\n");
    write_file(d.path(), "B.md", "Deploying Cedar Cloud.\n");
    let e = |name: &str, summary: &str| Entity { name: name.into(), ty: "organization".into(), aliases: vec![], summary: summary.into() };
    graph::apply(&v, "A.md", &Extraction { entities: vec![e("Cedar Cloud", "Cloud VM.")], relations: vec![] }, &now()).unwrap();
    let rel = Relation {
        from: "Cedar Cloud deploy".into(),
        from_type: "organization".into(),
        rel: "related_to".into(),
        to: "DEPLOY guide".into(),
        to_type: "source".into(),
        evidence: String::new(),
    };
    graph::apply(&v, "B.md", &Extraction { entities: vec![e("Cedar Cloud deploy", "Deploying it.")], relations: vec![rel] }, &now()).unwrap();
    d
}

fn run_(d: &tempfile::TempDir, st: &std::path::Path, b: &Backend, apply: bool) -> Value {
    run(&open(d), st, b, apply, &emb(), &now()).unwrap()
}

#[test]
fn consolidate_co_01() {
    let d = seed();
    let g = gather(&open(&d)).unwrap();
    // walk order: "Cedar Cloud deploy.md" sorts before "Cedar Cloud.md" (' ' < '.')
    let org: Vec<(&str, &str, &str)> =
        g.iter().filter(|e| e.ty == "organization").map(|e| (e.ty.as_str(), e.name.as_str(), e.summary.as_str())).collect();
    assert_eq!(org, [("organization", "Cedar Cloud deploy", "Deploying it."), ("organization", "Cedar Cloud", "Cloud VM.")]);
}

#[test]
fn consolidate_co_02() {
    let d = seed();
    let st = tempfile::tempdir().unwrap();
    let mut reply = merge_reply();
    reply["merges"].as_array_mut().unwrap().extend([
        json!({"type": "organization", "canonical": "Nonexistent", "duplicates": ["Cedar Cloud"]}),
        json!({"type": "person", "canonical": "Cedar Cloud", "duplicates": ["Cedar Cloud deploy"]}),
    ]);
    let r = run_(&d, st.path(), &Backend::new(vec![Some(reply)]), false);
    assert_eq!(r["proposed"], merge_reply()["merges"]);
    assert_eq!(r["skipped_batches"], 0);
}

#[test]
fn consolidate_co_03() {
    let d = seed();
    let st = tempfile::tempdir().unwrap();
    let before = read(&d, DUP);
    let b = Backend::new(vec![Some(merge_reply())]);
    let r = run_(&d, st.path(), &b, false);
    assert!(!r["proposed"].as_array().unwrap().is_empty());
    assert_eq!((r["applied"].clone(), r["skipped_batches"].clone()), (json!(false), json!(0)));
    assert_eq!(read(&d, DUP), before);
    assert!(!vectors_path(st.path()).exists());
    assert_eq!(r.as_object().unwrap().keys().collect::<Vec<_>>(), ["entities", "proposed", "applied", "merged_entities", "skipped_batches"]);
    assert!(b.prompts.lock().unwrap()[0].ends_with("Entities:\norganization | Cedar Cloud | -\norganization | Cedar Cloud deploy | -"));
}

#[test]
fn consolidate_co_04() {
    let d = seed();
    let st = tempfile::tempdir().unwrap();
    let r = run_(&d, st.path(), &Backend::new(vec![Some(merge_reply())]), true);
    assert_eq!((r["applied"].clone(), r["merged_entities"].clone()), (json!(true), json!(1)));
    let canon = read(&d, CANON);
    assert!(canon.contains("[[B|") && canon.contains("related_to [[") && canon.contains("Cedar Cloud deploy"));
    assert!(canon.contains("aliases: [Cedar Cloud deploy]"));
    let dup = read(&d, DUP);
    assert!(dup.contains("merged_into:") && dup.contains("Merged into [[Cedar Cloud]]"));
    assert!(dup.contains("merged_into: Claude/Graph/Organizations/Cedar Cloud\n"));
}

#[test]
fn consolidate_co_05() {
    let d = seed();
    let st = tempfile::tempdir().unwrap();
    graph::merge_aliases(&open(&d), CANON, &["Cedar Cloud deploy".into()]).unwrap();
    run_(&d, st.path(), &Backend::new(vec![Some(merge_reply())]), true);
    assert!(read(&d, DUP).contains("merged_into:"));
}

#[test]
fn consolidate_co_06() {
    let d = seed();
    let st = tempfile::tempdir().unwrap();
    run_(&d, st.path(), &Backend::new(vec![Some(merge_reply())]), true);
    let found = Graph::load(&st.path().join("graph.db")).unwrap().find("cedar", None);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0]["name"].clone(), found[0]["mention_count"].clone()), (json!("Cedar Cloud"), json!(2)));
}

#[test]
fn consolidate_co_07() {
    let t = "# Foo\n\nLine one.\n\n---\n\nLine two.\n\n## Mentions\n\n## Relations\n";
    assert_eq!(entity_summary(t), "Line one.\n\n---\n\nLine two.");
    assert_eq!(entity_summary("# Foo\n\n## Mentions\n\n## Relations\n"), "");
}

fn ents(names: &[&str]) -> Vec<Ent> {
    names
        .iter()
        .map(|n| Ent { name: n.to_string(), ty: "organization".into(), path: format!("Claude/Graph/Organizations/{n}"), aliases: vec![], summary: String::new() })
        .collect()
}

#[test]
fn consolidate_co_08() {
    let all = ents(&["Cedar Cloud", "Cedar Cloud deploy", "Acme", "Acme Inc"]);
    let batches = vec![all[2..].to_vec(), all[..2].to_vec()];
    let b = Backend::new(vec![None, Some(merge_reply())]);
    let (merges, skipped) = adjudicate(&b, &batches, &all);
    assert_eq!(skipped, 1);
    assert_eq!(merges, [Merge { ty: "organization".into(), canonical: "Cedar Cloud".into(), duplicates: vec!["Cedar Cloud deploy".into()] }]);
}

#[test]
fn consolidate_co_09() {
    let all = ents(&["Cedar Cloud", "Cedar Cloud deploy"]);
    let same = json!({"merges": [{"type": "organization", "canonical": "cedar cloud", "duplicates": ["CEDAR CLOUD DEPLOY"]}]});
    let b = Backend::new(vec![Some(merge_reply()), Some(same)]);
    let (merges, skipped) = adjudicate(&b, &[all.clone(), all.clone()], &all);
    assert_eq!((merges.len(), skipped), (1, 0));
}

#[test]
fn consolidate_co_10() {
    let d = seed();
    let retired = read(&d, DUP).replacen("---\n", "---\nretired: true\n", 1);
    write_file(d.path(), DUP, &retired);
    let m = Merge { ty: "organization".into(), canonical: "Cedar Cloud".into(), duplicates: vec!["Cedar Cloud deploy".into()] };
    apply_merge(&open(&d), &m, &now()).unwrap();
    assert_eq!(read(&d, DUP), retired);
    assert!(!read(&d, CANON).contains("[[B|"));
}

#[test]
fn consolidate_co_11() {
    let spec = spec_block("07-consolidate.md", "CO-11 checks the whole text");
    assert_eq!(PROMPT, spec);
    let ent = |ty: &str, name: &str, aliases: &[&str]| Ent {
        name: name.into(),
        ty: ty.into(),
        path: String::new(),
        aliases: aliases.iter().map(|a| a.to_string()).collect(),
        summary: String::new(),
    };
    let batch = [ent("organization", "Oracle VM", &[]), ent("organization", "Oracle VM deploy", &["OVM", "oracle"])];
    let want = spec.replace("{listing}", "organization | Oracle VM | -\norganization | Oracle VM deploy | OVM, oracle");
    assert_eq!(prompt(&batch), want);
}
