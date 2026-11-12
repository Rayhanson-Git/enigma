//! acceptance/search.yaml
mod common;
use common::*;
use enigma_mcp::embed::Embedder;
use enigma_mcp::error::Kind;
use enigma_mcp::faults::{self, Fault};
use enigma_mcp::search::*;
use enigma_mcp::vault::{body, str_list};
use enigma_mcp::yaml::Y;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

fn corpus(docs: &[(&str, &str)]) -> Vec<(String, String)> {
    docs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

fn ranked(docs: &[(&str, &str)], q: &str, limit: usize) -> Vec<String> {
    rank(&corpus(docs), q, limit).into_iter().map(|(r, _)| r).collect()
}

fn paths(hits: &[Value]) -> Vec<String> {
    hits.iter().map(|h| h["path"].as_str().unwrap().to_string()).collect()
}

fn hy(d: &tempfile::TempDir, emb: &dyn Embedder, q: &str, tags: Option<&[String]>, limit: i64) -> Vec<Value> {
    let state = tempfile::tempdir().unwrap();
    hybrid(&open(d), state.path(), emb, q, tags, None, limit).unwrap()
}

#[test]
fn search_bm_01() {
    assert_eq!(tokenize("Lighthouse-Grid Pipeline!"), ["lighthouse", "grid", "pipeline"]);
}

#[test]
fn search_bm_02() {
    let docs = [("a.md", "the lighthouse grid pipeline ingests outage data"), ("b.md", "an unrelated note about weather patterns")];
    assert_eq!(ranked(&docs, "lighthouse pipeline", 10), ["a.md"]);
}

#[test]
fn search_bm_03() {
    let docs = [("common.md", "the the the the the project project"), ("rare.md", "zephyr appears exactly once here")];
    assert_eq!(ranked(&docs, "zephyr", 10)[0], "rare.md");
}

#[test]
fn search_bm_04() {
    assert!(ranked(&[], "x", 10).is_empty());
    assert!(ranked(&[("a.md", "some words")], "unknown", 10).is_empty());
}

#[test]
fn search_bm_05() {
    let docs: Vec<(&str, &str)> = (0..10).map(|_| ("same.md", "identical shared words")).collect();
    assert_eq!(ranked(&docs, "shared", 3).len(), 3);
}

#[test]
fn search_se_01() {
    let d = mini_vault();
    write_file(d.path(), ".obsidian/notes.md", "hidden");
    let v = open(&d);
    assert!(walk(&v, None).unwrap().iter().all(|(_, rel)| !rel.contains(".obsidian")));
    let claude = walk(&v, Some("Claude")).unwrap();
    assert!(!claude.is_empty() && claude.iter().all(|(_, rel)| rel.starts_with("Claude/")));
}

#[test]
fn search_se_02() {
    let d = mini_vault();
    let rels: Vec<String> = candidates(&open(&d), None, None).unwrap().into_iter().map(|(r, _)| r).collect();
    assert!(rels.contains(&"Projects/Lighthouse Grid.md".into()) && rels.contains(&"Daily.md".into()));
}

fn tagged(d: &tempfile::TempDir, tags: &[&str]) -> Vec<String> {
    let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    candidates(&open(d), Some(&tags), None).unwrap().into_iter().map(|(r, _)| r).collect()
}

#[test]
fn search_se_03() {
    let d = mini_vault();
    assert_eq!(tagged(&d, &["grid"]), ["Projects/Lighthouse Grid.md"]);
    assert_eq!(tagged(&d, &["GRID"]), ["Projects/Lighthouse Grid.md"]);
    assert!(!tagged(&d, &["grid", "project"]).is_empty());
    assert!(tagged(&d, &["grid", "nope"]).is_empty());
}

#[test]
fn search_se_04() {
    let d = mini_vault();
    write_file(d.path(), "Scalar.md", "---\ntags: solo\n---\n\nBody\n");
    assert_eq!(tagged(&d, &["solo"]), ["Scalar.md"]);
}

#[test]
fn search_se_05() {
    assert!(str_list(Some(&Y::Null)).is_empty());
    assert_eq!(str_list(Some(&Y::Str("one".into()))), ["one"]);
    assert_eq!(str_list(Some(&Y::Seq(vec![Y::Str("a".into()), Y::Int(2)]))), ["a", "2"]);
}

#[test]
fn search_se_06() {
    assert_eq!(body("---\ntags: [x]\n---\n\n# T\n\nBody.\n"), "\n\n# T\n\nBody.\n");
    assert_eq!(body("# T\n\nBody.\n"), "# T\n\nBody.\n");
    assert_eq!(body("---\ntags: [x]\n\nBody.\n"), "---\ntags: [x]\n\nBody.\n");
}

#[test]
fn search_hy_01() {
    let d = mini_vault();
    write_file(d.path(), "Contractors.md", "Outstanding invoices from contractors need review.\n");
    let hits = paths(&hy(&d, &FnEmb(semantic_vec), "who do I owe money to", None, 20));
    assert!(hits.contains(&"Contractors.md".into()), "{hits:?}");
}

#[test]
fn search_hy_02() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let fused = rrf(&[s(&["x", "y", "z"]), s(&["y", "x", "z"])]);
    assert!(fused[0] == "x" || fused[0] == "y");
    assert_eq!(fused[2], "z");
    assert_eq!(rrf(&[s(&["x"]), vec![]]), ["x"]);
    assert!(rrf(&[vec![], vec![]]).is_empty());
}

#[test]
fn search_hy_03() {
    let d = mini_vault();
    write_file(d.path(), "Freight.md", "This note is about supply chain freight operations.");
    assert!(paths(&hy(&d, &FnEmb(keyword_vec), "freight", None, 20)).contains(&"Freight.md".into()));
}

#[test]
fn search_hy_04() {
    let d = mini_vault();
    let tags = vec!["grid".to_string()];
    assert_eq!(paths(&hy(&d, &FnEmb(keyword_vec), "e", Some(&tags), 20)), ["Projects/Lighthouse Grid.md"]);
}

#[test]
fn search_hy_05() {
    let notes: Vec<(String, &str)> = (0..5).map(|i| (format!("n{i}.md"), "shared keyword here")).collect();
    let refs: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    let d = vault_with(&refs);
    assert_eq!(hy(&d, &FnEmb(keyword_vec), "keyword", None, 2).len(), 2);
}

#[test]
fn search_hy_06() {
    let d = mini_vault();
    assert!(hy(&d, &FnEmb(keyword_vec), "zzzznomatch", None, 20).is_empty());
}

#[test]
fn search_hy_07() {
    let d = vault_with(&[("aaa.md", "an important announcement was made"), ("zzz.md", "the port of hamburg is busy")]);
    let hits = paths(&hy(&d, &FnEmb(keyword_vec), "port", None, 20));
    assert!(hits.contains(&"zzz.md".into()) && !hits.contains(&"aaa.md".into()));
}

#[test]
fn search_hy_08() {
    assert_eq!(excerpt("---\ntags: [x]\n---\n\n# Title Line\n\nReal content here.\n", "n.md", "unrelated"), "# Title Line");
}

#[test]
fn search_hy_09() {
    let t = "---\ntags: [billing]\n---\n\nInvoice for the billing cycle.\n";
    assert_eq!(excerpt(t, "n.md", "billing"), "Invoice for the billing cycle.");
}

#[test]
fn search_hy_10() {
    assert_eq!(excerpt("---\ntags: [x]\n---\n", "n.md", "query"), "");
}

#[test]
fn search_hy_11() {
    assert_eq!(excerpt("anything", "Claude/Sessions/Weekly Review.md", "weekly"), "(title match)");
}

#[test]
fn search_hy_12() {
    let d = mini_vault();
    write_file(d.path(), "Bills.md", "---\ntags: [money]\n---\n\nPay the outstanding invoice soon.\nSecond line.\n");
    let hits = hy(&d, &FnEmb(semantic_vec), "who do I owe", None, 20);
    let bills = hits.iter().find(|h| h["path"] == "Bills.md").unwrap();
    assert_eq!(bills["excerpt"], "Pay the outstanding invoice soon.");
}

// ------------------------------------------------------------- vectors

fn sc_line(rel: &str, vec: &str, at: f64) -> String {
    format!(r#""smart_sources:{rel}": {{"path": "{rel}", "embeddings": {{"TaylorAI/bge-micro-v2": {{"vec": {vec}}}}}, "last_embed": {{"at": {at}}}}},"#)
}

fn write_sc(root: &Path, lines: &[String]) {
    write_file(root, ".smart-env/multi/notes.ajson", &(lines.join("\n") + "\n"));
}

const FUTURE: f64 = 4_000_000_000_000.0;

fn vec_of(vs: &[(String, Vec<f64>)], rel: &str) -> Vec<f64> {
    vs.iter().find(|(r, _)| r == rel).unwrap().1.clone()
}

#[test]
fn search_em_01() {
    let d = mini_vault();
    let lines: Vec<String> = walk(&open(&d), None)
        .unwrap()
        .iter()
        .map(|(_, rel)| sc_line(rel, if rel == "Daily.md" { "[1, 2]" } else { "[3, 4]" }, FUTURE))
        .collect();
    write_sc(d.path(), &lines);
    let (state, emb) = (tempfile::tempdir().unwrap(), LenEmb::default());
    let vs = note_vectors(&open(&d), state.path(), &emb, None, true).unwrap();
    assert_eq!(vec_of(&vs, "Daily.md"), [1.0, 2.0]);
    assert_eq!(emb.texts.load(Ordering::SeqCst), 0);
}

#[test]
fn search_em_02() {
    let d = mini_vault();
    write_sc(d.path(), &[sc_line("Daily.md", "[1, 2]", 0.0)]);
    let (state, emb) = (tempfile::tempdir().unwrap(), LenEmb::default());
    let vs = note_vectors(&open(&d), state.path(), &emb, None, true).unwrap();
    let daily = read(&d, "Daily.md");
    assert_eq!(vec_of(&vs, "Daily.md"), [daily.chars().count() as f64, 0.0]);
    assert!(emb.texts.load(Ordering::SeqCst) > 0);
}

#[test]
fn search_em_03() {
    let d = mini_vault();
    let (state, emb) = (tempfile::tempdir().unwrap(), LenEmb::default());
    let n = walk(&open(&d), None).unwrap().len();
    note_vectors(&open(&d), state.path(), &emb, None, true).unwrap();
    assert_eq!(emb.texts.load(Ordering::SeqCst), n);
    note_vectors(&open(&d), state.path(), &emb, None, true).unwrap();
    assert_eq!(emb.texts.load(Ordering::SeqCst), n);
}

#[test]
fn search_em_04() {
    let d = mini_vault();
    let cold = tempfile::tempdir().unwrap();
    assert!(!stale_notes(&open(&d), cold.path()).unwrap().is_empty());
    assert!(!fallback_path(cold.path()).exists());
    let state = tempfile::tempdir().unwrap();
    note_vectors(&open(&d), state.path(), &LenEmb::default(), None, true).unwrap();
    assert!(stale_notes(&open(&d), state.path()).unwrap().is_empty());
    write_file(d.path(), "Daily.md", "# Daily\n\nEdited.\n");
    assert_eq!(stale_notes(&open(&d), state.path()).unwrap(), ["Daily.md"]);
}

fn entry(v: f64) -> Entry {
    Entry { hash: "h".into(), vec: vec![v, 0.5] }
}

#[test]
fn search_em_05() {
    let state = tempfile::tempdir().unwrap();
    let path = fallback_path(state.path());
    let mut a = Cache::new();
    a.insert("a.md".into(), entry(1.0));
    save_cache(&path, &a).unwrap();
    assert_eq!(load_cache(&path), a);
    assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"a.md": {"hash": "h", "vec": [1.0, 0.5]}}"#);
    let mut b = Cache::new();
    b.insert("b.md".into(), entry(2.0));
    b.insert("c.md".into(), entry(3.0));
    fs::write(&path, serde_json::to_string(&b).unwrap()).unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(2_000_000_000)).unwrap();
    assert_eq!(load_cache(&path), b);
    let mut loaded = load_cache(&path);
    loaded.insert("x.md".into(), entry(9.0));
    assert_eq!(load_cache(&path), b);
    fs::write(&path, "{corrupt").unwrap();
    assert!(load_cache(&path).is_empty());
}

#[test]
fn search_em_06() {
    faults::reset();
    let state = tempfile::tempdir().unwrap();
    let path = fallback_path(state.path());
    let mut old = Cache::new();
    old.insert("a.md".into(), entry(1.0));
    save_cache(&path, &old).unwrap();
    let mut new = old.clone();
    new.insert("b.md".into(), entry(2.0));
    faults::arm("cache_rename", Fault::Os(13), 1);
    assert_eq!(save_cache(&path, &new).unwrap_err().kind, Kind::IoError);
    let names: Vec<String> = fs::read_dir(state.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
    assert_eq!(names, ["fallback_embeddings.json"]);
    assert_eq!(load_cache(&path), old);
}

#[test]
fn search_em_07() {
    let d = mini_vault();
    let state = tempfile::tempdir().unwrap();
    let vs = note_vectors(&open(&d), state.path(), &LenEmb::default(), None, false).unwrap();
    assert!(!vs.is_empty());
    assert!(!fallback_path(state.path()).exists());
}

// ------------------------------------------------------ Smart Connections

fn set_mtime(path: &Path, secs: u64) {
    fs::File::options().write(true).open(path).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
}

#[test]
fn search_sc_01() {
    let d = mini_vault();
    set_mtime(&d.path().join("Daily.md"), 1_700_000_000);
    set_mtime(&d.path().join("Claude/Index.md"), 1_700_000_000);
    write_sc(
        d.path(),
        &[sc_line("Daily.md", "[0.1, 0.2, 0.3]", 1_700_000_001_000.0), sc_line("Claude/Index.md", "[1]", 1_699_999_999_000.0)],
    );
    let sc = smart_vectors(d.path());
    assert_eq!(sc["Daily.md"], ScVec { vec: vec![0.1, 0.2, 0.3], fresh: true });
    assert!(!sc["Claude/Index.md"].fresh);
}

#[test]
fn search_sc_02() {
    let d = mini_vault();
    write_sc(d.path(), &[sc_line("Daily.md", "[1]", FUTURE), sc_line("Daily.md", "[2]", FUTURE)]);
    assert_eq!(smart_vectors(d.path())["Daily.md"].vec, [2.0]);
}

#[test]
fn search_sc_03() {
    let d = mini_vault();
    assert!(smart_vectors(d.path()).is_empty());
}

#[test]
fn search_sc_04() {
    let d = mini_vault();
    let block = r#""smart_blocks:Daily.md#Daily": {"path": "Daily.md", "embeddings": {"TaylorAI/bge-micro-v2": {"vec": [9]}}, "last_embed": {"at": 4000000000000}},"#;
    write_sc(d.path(), &[block.to_string(), sc_line("Daily.md", "[1]", FUTURE)]);
    let sc = smart_vectors(d.path());
    assert_eq!(sc.keys().collect::<Vec<_>>(), ["Daily.md"]);
    assert_eq!(sc["Daily.md"].vec, [1.0]);
}

// ------------------------------------------------ review fixes (Step A)

#[test]
fn walk_skips_symlinks_that_leave_the_vault() {
    let d = mini_vault();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "- [ ] SECRET-TOKEN-123 secretword\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.md"), d.path().join("leak.md")).unwrap();
    std::os::unix::fs::symlink(outside.path(), d.path().join("Outside")).unwrap();
    std::os::unix::fs::symlink("Daily.md", d.path().join("alias.md")).unwrap();
    let v = open(&d);
    let rels: Vec<String> = walk(&v, None).unwrap().into_iter().map(|(_, r)| r).collect();
    assert!(rels.contains(&"alias.md".into()), "in-vault symlinks are still walked");
    assert!(!rels.iter().any(|r| r == "leak.md" || r.starts_with("Outside")));
    let tasks = enigma_mcp::notes::list_tasks(&v, true, None).unwrap();
    assert!(!serde_json::to_string(&tasks).unwrap().contains("SECRET"));
    let hits = hy(&d, &FnEmb(keyword_vec), "secretword", None, 20);
    assert!(paths(&hits).iter().all(|p| p != "leak.md"));
    assert!(!serde_json::to_string(&hits).unwrap().contains("SECRET"));
}

#[test]
fn embedding_order_and_batching_do_not_change_vectors() {
    let bge = enigma_mcp::embed::Bge::load().unwrap();
    let texts: Vec<String> = (0..40).map(|i| format!("note {i} {}", "word ".repeat((i * 37) % 300))).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let mut rev = refs.clone();
    rev.reverse();
    let a = bge.embed(&refs).unwrap();
    let mut b = bge.embed(&rev).unwrap();
    b.reverse();
    let close = |x: &[f64], y: &[f64]| x.iter().zip(y).all(|(p, q)| (p - q).abs() < 1e-5);
    assert!(a.iter().zip(&b).all(|(x, y)| close(x, y)));
    for i in [0, 7, 39] {
        let single = bge.embed(&[refs[i]]).unwrap();
        assert!(close(&single[0], &a[i]), "text {i} differs from its unbatched vector");
    }
}
