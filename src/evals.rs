//! Search evaluation harness (13).
use crate::embed::Embedder;
use crate::error::{Error, Kind, Result};
use crate::extractor::{RunOutcome, run_process_for};
use crate::graph::truthy;
use crate::text::display;
use crate::vault::Vault;
use crate::yaml::{Y, parse_doc, syntax_error};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub const LIVE_NOTE: &str = "Claude/Evals.md";
pub const HISTORY_FILE: &str = "eval_history.jsonl";
pub const LIMIT: i64 = 20;

pub const LIVE_SEED: &str = "# Search eval golden set (live vault)\n\nQueries for `enigma-mcp evals --live`. Paths are vault-relative. A query\nwhose expected notes have all gone is skipped and reported, never fatal,\nbecause a live vault drifts. Add a query whenever a search disappoints you.\n\n```yaml\n- id: example-constitution\n  query: what are the rules for agents writing to the vault\n  expect:\n    - Claude/README.md\n  note: seed example -- replace or extend freely\n```\n";

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```yaml\s*\n(.*?)```").expect("static regex"));

/// The source checkout the binary was built from (13 §1).
pub fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture_vault() -> PathBuf {
    source_root().join("spec/fixtures/vault")
}

pub fn fixture_golden() -> PathBuf {
    source_root().join("spec/fixtures/vault-golden.yaml")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub id: String,
    pub query: String,
    pub expect: Vec<String>,
    pub accept: Vec<String>,
    pub tags: Option<Vec<String>>,
    pub folder: Option<String>,
    pub note: String,
}

fn config(msg: String) -> Error {
    Error::new(Kind::EvalConfigError, msg)
}

// ---------------------------------------------------------------- metrics (§2)

pub fn first_relevant_rank(hits: &[String], relevant: &HashSet<String>) -> Option<usize> {
    hits.iter().position(|h| relevant.contains(h)).map(|i| i + 1)
}

pub fn recall_at_k(hits: &[String], expect: &HashSet<String>, k: usize) -> f64 {
    if expect.is_empty() {
        return 0.0;
    }
    hits.iter().take(k).filter(|h| expect.contains(*h)).count() as f64 / expect.len() as f64
}

pub fn success_at_k(hits: &[String], relevant: &HashSet<String>, k: usize) -> bool {
    hits.iter().take(k).any(|h| relevant.contains(h))
}

// ------------------------------------------------------------ golden sets (§3)

fn as_list(id: &str, field: &str, v: Option<&Y>) -> Result<Vec<String>> {
    match v {
        None | Some(Y::Null) => Ok(vec![]),
        Some(s @ (Y::Str(_) | Y::Int(_) | Y::Float(_) | Y::Bool(_))) => Ok(vec![display(s)]),
        Some(Y::Seq(items)) => Ok(items.iter().map(display).collect()),
        Some(other) => {
            let ty = match other {
                Y::Map(_) => "dict",
                Y::Date(_) => "date",
                Y::DateTime(..) => "datetime",
                // shortcut: omap/pairs also land here as "bytes"; upgrade if a golden file ever uses them
                Y::Other(s) if s.starts_with('{') => "set",
                _ => "bytes",
            };
            Err(config(format!("golden {id}: '{field}' must be a string or list, got {ty}")))
        }
    }
}

/// load_golden(path) (13 §3).
pub fn load_golden(path: &Path) -> Result<Vec<Query>> {
    let p = path.display();
    if !path.is_file() {
        return Err(config(format!("golden file not found: {p}")));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::io(&e, path))?;
    let mut text = String::from_utf8(bytes).map_err(|_| Error::new(Kind::DecodeError, format!("{p} is not valid UTF-8")))?;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
        text = FENCE.captures(&text).map(|c| c[1].to_string()).ok_or_else(|| config(format!("no ```yaml block found in {p}")))?;
    }
    if let Some(detail) = syntax_error(&text) {
        return Err(config(format!("invalid YAML in {p}: {detail}")));
    }
    let Some(Y::Seq(entries)) = parse_doc(&text).map_err(|e| config(format!("invalid YAML in {p}: {}", e.msg)))? else {
        return Err(config(format!("golden set must be a YAML list: {p}")));
    };
    let mut out = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Y::Map(m) = entry else { return Err(config(format!("entry {i} in {p} needs 'id' and 'query'"))) };
        let get = |k: &str| m.iter().find(|(key, _)| matches!(key, Y::Str(s) if s == k)).map(|(_, v)| v);
        let (Some(id), Some(query)) = (get("id"), get("query")) else {
            return Err(config(format!("entry {i} in {p} needs 'id' and 'query'")));
        };
        let (id, query) = (display(id), display(query));
        if id.trim().is_empty() || query.trim().is_empty() {
            return Err(config(format!("entry {i} in {p}: 'id' and 'query' must be non-empty strings")));
        }
        out.push(Query {
            expect: as_list(&id, "expect", get("expect"))?,
            accept: as_list(&id, "accept", get("accept"))?,
            tags: if truthy(get("tags")) { Some(as_list(&id, "tags", get("tags"))?) } else { None },
            folder: truthy(get("folder")).then(|| display(get("folder").unwrap_or(&Y::Null))),
            note: if truthy(get("note")) { display(get("note").unwrap_or(&Y::Null)) } else { String::new() },
            id,
            query,
        });
    }
    let mut seen = HashSet::new();
    for q in &out {
        if !seen.insert(q.id.as_str()) {
            return Err(config(format!("duplicate golden id: {}", q.id)));
        }
        if q.expect.is_empty() {
            return Err(config(format!("golden {}: 'expect' must be non-empty", q.id)));
        }
    }
    Ok(out)
}

// ------------------------------------------------------------ validation (§4)

fn missing_path(root: &Path, rel: &str) -> bool {
    let p = Path::new(rel);
    p.is_absolute()
        || p.components().any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        || rel.split(['/', '\\']).any(|s| s == "..")
        || !root.join(rel).is_file()
}

/// validate(queries, root, strict) → (id, missing paths) in query order (13 §4).
pub fn validate(queries: &[Query], root: &Path, strict: bool) -> Result<Vec<(String, Vec<String>)>> {
    let mut out = Vec::new();
    for q in queries {
        let missing: Vec<String> = q.expect.iter().chain(&q.accept).filter(|r| missing_path(root, r)).cloned().collect();
        if !missing.is_empty() {
            out.push((q.id.clone(), missing));
        }
    }
    if strict && !out.is_empty() {
        let detail: Vec<String> = out.iter().map(|(id, m)| format!("{id}: {}", m.join(", "))).collect();
        return Err(config(format!("golden paths missing from vault: {}", detail.join("; "))));
    }
    Ok(out)
}

// --------------------------------------------------------------- running (§5)

#[derive(Debug, Clone)]
pub struct Scored {
    pub id: String,
    pub hits: Vec<String>,
    pub first_rank: Option<usize>,
    pub recall5: f64,
    pub recall10: f64,
    pub success5: bool,
    pub success10: bool,
    pub skipped: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub results: Vec<Scored>,
    pub success5: f64,
    pub success10: f64,
    pub recall5: f64,
    pub recall10: f64,
    pub mrr: f64,
    pub skipped: usize,
    pub scored: usize,
}

/// run(vault, state dir, embedder, queries, lenient) (13 §5).
pub fn run(v: &Vault, state_dir: &Path, emb: &dyn Embedder, queries: &[Query], lenient: bool) -> Result<Report> {
    std::fs::create_dir_all(state_dir).map_err(|e| Error::io(&e, state_dir))?;
    let missing = validate(queries, &v.root, !lenient)?;
    let mut results = Vec::new();
    for q in queries {
        let miss: Vec<String> = missing.iter().find(|(id, _)| *id == q.id).map(|(_, m)| m.clone()).unwrap_or_default();
        let gone: HashSet<&String> = miss.iter().collect();
        if lenient && q.expect.iter().all(|e| gone.contains(e)) {
            results.push(Scored {
                id: q.id.clone(),
                hits: vec![],
                first_rank: None,
                recall5: 0.0,
                recall10: 0.0,
                success5: false,
                success10: false,
                skipped: true,
                missing: miss,
            });
            continue;
        }
        let hits: Vec<String> = crate::search::hybrid(v, state_dir, emb, &q.query, q.tags.as_deref(), q.folder.as_deref(), LIMIT)?
            .iter()
            .filter_map(|h| h["path"].as_str().map(String::from))
            .collect();
        let expect: HashSet<String> = q.expect.iter().filter(|e| !gone.contains(e)).cloned().collect();
        let relevant: HashSet<String> = q.expect.iter().chain(&q.accept).filter(|e| !gone.contains(e)).cloned().collect();
        results.push(Scored {
            id: q.id.clone(),
            first_rank: first_relevant_rank(&hits, &relevant),
            recall5: recall_at_k(&hits, &expect, 5),
            recall10: recall_at_k(&hits, &expect, 10),
            success5: success_at_k(&hits, &relevant, 5),
            success10: success_at_k(&hits, &relevant, 10),
            hits,
            skipped: false,
            missing: miss,
        });
    }
    let scored: Vec<&Scored> = results.iter().filter(|r| !r.skipped).collect();
    let n = scored.len().max(1) as f64;
    // fold from +0.0: an empty f64 sum() is -0.0, which would print as -0.00
    let rate = |f: &dyn Fn(&Scored) -> f64| scored.iter().fold(0.0, |acc, r| acc + f(r)) / n;
    Ok(Report {
        success5: rate(&|r| f64::from(u8::from(r.success5))),
        success10: rate(&|r| f64::from(u8::from(r.success10))),
        recall5: rate(&|r| r.recall5),
        recall10: rate(&|r| r.recall10),
        mrr: rate(&|r| r.first_rank.map_or(0.0, |k| 1.0 / k as f64)),
        skipped: results.len() - scored.len(),
        scored: scored.len(),
        results,
    })
}

// ---------------------------------------------------------------- output (§6)

/// The table (13 §6.1), without a trailing LF.
pub fn table(r: &Report) -> String {
    let mut lines = vec![format!("{:<28} {:>4} {:>5}  miss", "id", "rank", "r@10")];
    for q in &r.results {
        let rank = if q.skipped { "skip".into() } else { q.first_rank.map_or("-".into(), |k| k.to_string()) };
        let recall = if q.skipped { "-".into() } else { format!("{:.2}", q.recall10) };
        lines.push(format!("{:<28} {:>4} {:>5}  {}", q.id, rank, recall, q.missing.join(", ")));
    }
    lines.push("-".repeat(60));
    lines.push(format!(
        "queries {}  skipped {}  success@5 {:.2}  success@10 {:.2}  recall@5 {:.2}  recall@10 {:.2}  MRR {:.2}",
        r.scored, r.skipped, r.success5, r.success10, r.recall5, r.recall10, r.mrr
    ));
    lines.join("\n")
}

/// The JSON form (13 §6.2).
pub fn to_json(r: &Report) -> Value {
    json!({
        "success_at_5": r.success5, "success_at_10": r.success10, "recall_at_5": r.recall5, "recall_at_10": r.recall10,
        "mrr": r.mrr, "skipped": r.skipped, "n_queries": r.scored,
        "queries": r.results.iter().map(|q| json!({
            "id": q.id, "first_rank": q.first_rank, "skipped": q.skipped, "recall": q.recall10, "missing": q.missing,
        })).collect::<Vec<_>>(),
    })
}

/// A path as given, with redundant separators and `.` segments removed.
pub fn tidy(p: &Path) -> String {
    let out: PathBuf = p.components().filter(|c| !matches!(c, Component::CurDir)).collect();
    if out.as_os_str().is_empty() { ".".into() } else { out.display().to_string() }
}

fn git_head() -> Option<String> {
    let argv = ["git".into(), "-C".into(), source_root().display().to_string(), "rev-parse".into(), "--short".into(), "HEAD".into()];
    match run_process_for(&argv, "", 5) {
        RunOutcome::Exit { code: 0, stdout, .. } => Some(stdout.trim().to_string()).filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// Appends one history line (13 §6.3).
pub fn append_history(state_dir: &Path, vault: &Path, golden: &Path, result: &Value, now: &chrono::NaiveDateTime) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let mut line = json!({
        "ts": now.format("%Y-%m-%dT%H:%M:%S").to_string(), "git": git_head(), "vault": tidy(vault), "golden": tidy(golden),
    });
    if let (Some(m), Some(r)) = (line.as_object_mut(), result.as_object()) {
        m.extend(r.clone());
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(state_dir.join(HISTORY_FILE))?;
    f.write_all(format!("{}\n", crate::json::compact(&line)).as_bytes())
}

/// --init-live (13 §7): Ok(true) when created.
pub fn init_live(vault_root: &Path) -> Result<bool> {
    let target = vault_root.join(LIVE_NOTE);
    if target.symlink_metadata().is_ok() {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(&e, parent))?;
    }
    std::fs::write(&target, LIVE_SEED).map_err(|e| Error::io(&e, &target))?;
    Ok(true)
}
