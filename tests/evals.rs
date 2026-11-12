//! acceptance/evals.yaml (keyword embedder over [alpha, beta, gamma])
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::evals::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn kw(t: &str) -> Vec<f64> {
    ["alpha", "beta", "gamma"].iter().map(|w| if t.contains(w) { 1.0 } else { 0.0 }).collect()
}

fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn hits(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const GOLDEN: &str = "- id: q1\n  query: alpha beta\n  expect: [Notes/A.md]\n  accept: [Notes/B.md]\n  tags: [x]\n  folder: Notes\n  note: demo\n- id: q2\n  query: gamma\n  expect: [Notes/B.md]\n";

/// A golden file with the given text; the TempDir keeps it alive.
fn golden(name: &str, text: &str) -> (TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    std::fs::write(&p, text).unwrap();
    (d, p)
}

fn load_err(text: &str) -> String {
    let (_d, p) = golden("g.yaml", text);
    let e = load_golden(&p).err().unwrap();
    assert_eq!(e.kind, Kind::EvalConfigError);
    e.msg
}

fn ab_vault() -> TempDir {
    vault_with(&[("Notes/A.md", "alpha alpha content"), ("Notes/B.md", "beta content")])
}

fn queries(text: &str) -> Vec<Query> {
    let (_d, p) = golden("g.yaml", text);
    load_golden(&p).unwrap()
}

fn run_kw(d: &TempDir, st: &Path, text: &str, lenient: bool) -> enigma_mcp::error::Result<Report> {
    run(&open(d), st, &FnEmb(kw), &queries(text), lenient)
}

fn cli(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            cmd.env_remove(k);
        }
    }
    cmd.arg("evals").args(args).envs(env.iter().copied()).output().unwrap()
}

#[test]
fn evals_ev_01() {
    assert_eq!(first_relevant_rank(&hits(&["a", "b", "c"]), &set(&["b"])), Some(2));
    assert_eq!(first_relevant_rank(&hits(&["a", "b", "c"]), &set(&["z"])), None);
    assert_eq!(first_relevant_rank(&[], &set(&["b"])), None);
}

#[test]
fn evals_ev_02() {
    let h = hits(&["a", "b", "c", "d"]);
    assert_eq!(recall_at_k(&h, &set(&["a", "d"]), 2), 0.5);
    assert_eq!(recall_at_k(&h, &set(&["a", "d"]), 4), 1.0);
    assert_eq!(recall_at_k(&h, &set(&[]), 4), 0.0);
}

#[test]
fn evals_ev_03() {
    assert!(success_at_k(&hits(&["a", "b"]), &set(&["b"]), 2));
    assert!(!success_at_k(&hits(&["a", "b"]), &set(&["b"]), 1));
}

#[test]
fn evals_ev_04() {
    let q = queries(GOLDEN);
    assert_eq!(q.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), ["q1", "q2"]);
    assert_eq!(q[0].accept, ["Notes/B.md"]);
    assert_eq!(q[0].tags, Some(vec!["x".to_string()]));
    assert_eq!(q[0].folder.as_deref(), Some("Notes"));
    assert_eq!(q[0].note, "demo");
    assert!(q[1].accept.is_empty());
    assert_eq!(q[1].tags, None);
}

#[test]
fn evals_ev_05() {
    let (_d, p) = golden("Evals.MD", &format!("# Golden\n\n```yaml\n{GOLDEN}```\n"));
    assert_eq!(load_golden(&p).unwrap().len(), 2);
    let (_d, p) = golden("Evals.md", "# Golden\n\nno block here\n");
    let e = load_golden(&p).err().unwrap();
    assert_eq!(e.kind, Kind::EvalConfigError);
    assert_eq!(e.msg, format!("no ```yaml block found in {}", p.display()));
}

#[test]
fn evals_ev_06() {
    assert_eq!(load_err("- {id: a, query: x, expect: [A.md]}\n- {id: a, query: y, expect: [B.md]}\n"), "duplicate golden id: a");
    assert_eq!(load_err("- {id: a, query: x}\n"), "golden a: 'expect' must be non-empty");
    let e = load_golden(Path::new("/nonexistent/golden.yaml")).err().unwrap();
    assert_eq!((e.kind, e.msg.as_str()), (Kind::EvalConfigError, "golden file not found: /nonexistent/golden.yaml"));
    assert!(load_err("{a: 1}\n").starts_with("golden set must be a YAML list: "));
    assert!(load_err("- [unclosed\n").starts_with("invalid YAML in "));
    assert!(load_err("- just text\n").contains("needs 'id' and 'query'"));
}

#[test]
fn evals_ev_07() {
    let q = queries("- id: s\n  query: x\n  expect: Notes/A.md\n  accept: 7\n  tags: x\n");
    assert_eq!(q[0].expect, ["Notes/A.md"]);
    assert_eq!(q[0].accept, ["7"]);
    assert_eq!(q[0].tags, Some(vec!["x".to_string()]));
    assert_eq!(queries("- {id: t, query: x, expect: true}\n")[0].expect, ["true"]);
    let e = load_err("- id: m\n  query: x\n  expect: {a: 1}\n");
    assert_eq!(e, "golden m: 'expect' must be a string or list, got dict");
    assert!(load_err("- id: d\n  query: x\n  expect: 2026-07-09\n").ends_with("got date"));
    assert!(load_err("- {id: '', query: x, expect: [A.md]}\n").contains("non-empty"));
    assert!(load_err("- {id: e, query: '   ', expect: [A.md]}\n").contains("non-empty"));
}

#[test]
fn evals_ev_08() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("vault");
    write_file(&root, "Notes/A.md", "alpha");
    write_file(outer.path(), "outside.md", "x");
    let q = queries("- {id: q1, query: alpha, expect: [Notes/A.md, Notes/GONE.md]}\n");
    let e = validate(&q, &root, true).err().unwrap();
    assert_eq!(e.kind, Kind::EvalConfigError);
    assert_eq!(e.msg, "golden paths missing from vault: q1: Notes/GONE.md");
    assert_eq!(validate(&q, &root, false).unwrap(), vec![("q1".to_string(), vec!["Notes/GONE.md".to_string()])]);
    let abs = outer.path().join("outside.md").display().to_string();
    let q = queries(&format!("- {{id: q2, query: alpha, expect: [Notes/A.md], accept: ['../outside.md', '{abs}']}}\n"));
    assert_eq!(validate(&q, &root, false).unwrap(), vec![("q2".to_string(), vec!["../outside.md".to_string(), abs])]);
}

#[test]
fn evals_ev_09() {
    let q = load_golden(&fixture_golden()).unwrap();
    assert_eq!(q.len(), 17);
    assert_eq!(validate(&q, &enigma_mcp::evals::fixture_vault(), true).unwrap(), vec![]);
}

#[test]
fn evals_ev_10() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let r = run_kw(&d, st.path(), "- {id: q1, query: alpha, expect: [Notes/A.md]}\n", false).unwrap();
    assert_eq!(r.results[0].first_rank, Some(1));
    assert_eq!((r.success5, r.recall10, r.mrr, r.skipped), (1.0, 1.0, 1.0, 0));
    let r = run_kw(&d, st.path(), "- {id: q2, query: zzz-nowhere, expect: [Notes/B.md]}\n", false).unwrap();
    assert_eq!(r.results[0].first_rank, None);
    assert_eq!((r.mrr, r.success10), (0.0, 0.0));
}

#[test]
fn evals_ev_11() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let text = "- {id: stale, query: beta, expect: [Notes/GONE.md]}\n- {id: good, query: alpha, expect: [Notes/A.md]}\n";
    assert_eq!(run_kw(&d, st.path(), text, false).err().unwrap().kind, Kind::EvalConfigError);
    let r = run_kw(&d, st.path(), text, true).unwrap();
    assert_eq!((r.skipped, r.scored), (1, 1));
    assert!(r.results[0].skipped);
    assert_eq!(r.results[0].missing, ["Notes/GONE.md"]);
    assert_eq!((r.mrr, r.success10), (1.0, 1.0));
    assert!(table(&r).contains("skip"));
    // every query skipped: rates are +0.0, never -0.0
    let r = run_kw(&d, st.path(), "- {id: stale, query: beta, expect: [Notes/GONE.md]}\n", true).unwrap();
    assert_eq!((r.scored, r.skipped), (0, 1));
    assert!(r.mrr.is_sign_positive() && r.success10.is_sign_positive());
    assert!(table(&r).ends_with("success@5 0.00  success@10 0.00  recall@5 0.00  recall@10 0.00  MRR 0.00"), "{}", table(&r));
}

#[test]
fn evals_ev_12() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let r = run_kw(&d, st.path(), "- {id: q, query: alpha, expect: [Notes/B.md], accept: [Notes/A.md]}\n", false).unwrap();
    assert_eq!(r.results[0].first_rank, Some(1));
    assert!(r.results[0].success10);
    assert_eq!(r.results[0].recall10, 0.0);
}

#[test]
fn evals_ev_13() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let nested = st.path().join("not/yet");
    run_kw(&d, &nested, "- {id: q1, query: alpha, expect: [Notes/A.md]}\n", false).unwrap();
    assert!(nested.is_dir());
}

#[test]
fn evals_ev_14() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let r = run_kw(&d, st.path(), "- {id: q1, query: alpha, expect: [Notes/A.md]}\n", false).unwrap();
    let t = table(&r);
    assert!(t.contains("MRR") && t.contains("success@10") && t.contains("q1"));
    assert!(!t.ends_with('\n'));
    assert!(t.lines().any(|l| l == "-".repeat(60)));
    let j = to_json(&r);
    assert_eq!(j["mrr"], 1.0);
    assert_eq!(j["queries"][0]["id"], "q1");
    assert_eq!(j.as_object().unwrap().keys().collect::<Vec<_>>(), ["success_at_5", "success_at_10", "recall_at_5", "recall_at_10", "mrr", "skipped", "n_queries", "queries"]);
    let text = enigma_mcp::json::indented(&j);
    assert!(text.contains("\"mrr\": 1.0"), "{text}");
    let now = chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap().and_hms_opt(12, 0, 0).unwrap();
    append_history(st.path(), &d.path().join("./Notes/.."), Path::new("g//golden.yaml"), &j, &now).unwrap();
    let raw = std::fs::read_to_string(st.path().join(HISTORY_FILE)).unwrap();
    assert!(raw.ends_with('\n') && raw.lines().count() == 1);
    let line: Value = serde_json::from_str(raw.trim_end()).unwrap();
    assert_eq!(line["ts"], "2026-07-09T12:00:00");
    assert_eq!(line["mrr"], 1.0);
    assert_eq!(line["vault"], json!(d.path().join("Notes").join("..").display().to_string()));
    assert_eq!(line["golden"], json!(Path::new("g").join("golden.yaml").display().to_string()));
    assert!(line["git"].is_string() || line["git"].is_null());
}

#[test]
fn evals_ev_15() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let (_g, gp) = golden("g.yaml", "- {id: q1, query: alpha, expect: [Notes/A.md]}\n");
    let args = ["--vault", d.path().to_str().unwrap(), "--golden", gp.to_str().unwrap(), "--json"];
    let env = [("ENIGMA_STATE_DIR", st.path().to_str().unwrap())];
    let o = cli(&args, &env);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let j: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["mrr"], 1.0);
    assert!(st.path().join(HISTORY_FILE).is_file());
    // a failing history write warns and keeps exit 0
    let st2 = tempfile::tempdir().unwrap();
    std::fs::create_dir(st2.path().join(HISTORY_FILE)).unwrap();
    let o = cli(&args, &[("ENIGMA_STATE_DIR", st2.path().to_str().unwrap())]);
    assert_eq!(o.status.code(), Some(0));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.starts_with("warning: could not write eval history: "), "{err}");
}

#[test]
fn evals_ev_16() {
    let d = ab_vault();
    let st = tempfile::tempdir().unwrap();
    let env = [("ENIGMA_STATE_DIR", st.path().to_str().unwrap())];
    let o = cli(&["--vault", d.path().to_str().unwrap(), "--golden", "/nonexistent/g.yaml"], &env);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("error: golden file not found"));
    let (_g, gp) = golden("g.yaml", "- {id: q1, query: alpha, expect: [Notes/A.md]}\n");
    let o = cli(&["--vault", "/nonexistent/vault", "--golden", gp.to_str().unwrap()], &env);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("error: "));
    let o = cli(&["--live"], &env);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "error: --live/--init-live require ENIGMA_VAULT_PATH\n");
}

#[test]
fn evals_ev_17() {
    let d = mini_vault();
    assert!(init_live(d.path()).unwrap());
    assert_eq!(read(&d, LIVE_NOTE), LIVE_SEED);
    assert_eq!(load_golden(&d.path().join(LIVE_NOTE)).unwrap()[0].id, "example-constitution");
    std::fs::write(d.path().join(LIVE_NOTE), "my edits").unwrap();
    assert!(!init_live(d.path()).unwrap());
    assert_eq!(read(&d, LIVE_NOTE), "my edits");
    let fresh = mini_vault();
    let o = cli(&["--init-live"], &[("ENIGMA_VAULT_PATH", fresh.path().to_str().unwrap())]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("created: {}\n", fresh.path().join(LIVE_NOTE).display()));
    let o = cli(&["--init-live"], &[("ENIGMA_VAULT_PATH", fresh.path().to_str().unwrap())]);
    assert!(String::from_utf8_lossy(&o.stdout).starts_with("already exists: "));
}

#[test]
fn evals_ev_18() {
    // quality gate only on request (13 §8): any non-empty value runs the paraphrase check, only 1 the floors
    let gate = std::env::var("ENIGMA_RUN_EVALS").unwrap_or_default();
    if gate.is_empty() {
        return;
    }
    let d = common::fixture_vault();
    let st = tempfile::tempdir().unwrap();
    let emb = enigma_mcp::embed::Bge::load().unwrap();
    let r = run(&open(&d), st.path(), &emb, &load_golden(&fixture_golden()).unwrap(), false).unwrap();
    if gate == "1" {
        assert_eq!(r.skipped, 0);
        assert!(r.success10 >= 0.80 && r.mrr >= 0.50, "success@10 {} MRR {}", r.success10, r.mrr);
    }
    let para = r.results.iter().find(|q| q.id == "para-query").unwrap();
    assert!(para.first_rank.is_some_and(|k| k <= 10), "{:?}", para.first_rank);
}
