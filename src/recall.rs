//! Recall bundles and the context block (11).
use crate::error::{Error, Result};
use crate::graph::{Graph, truthy};
use crate::text::{casefold, collapse_ws, display, minute_stamp, split_lines};
use crate::vault::{Vault, body, get, parse, read_lenient};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

static SINCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})$").expect("static regex"));

/// A section: the producer's mapping plus status "ok", or the error form (11 §1).
fn section(f: impl FnOnce() -> Result<Map<String, Value>>) -> Value {
    match f() {
        Ok(mut m) => {
            m.insert("status".into(), json!("ok"));
            Value::Object(m)
        }
        Err(e) => json!({"status": "error", "error": e.to_string()}),
    }
}

fn mtime_local(path: &Path) -> Result<NaiveDateTime> {
    let m = std::fs::metadata(path).and_then(|m| m.modified()).map_err(|e| Error::io(&e, path))?;
    let secs = m.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Ok(chrono::DateTime::from_timestamp(secs, 0).map(|d| d.with_timezone(&Local).naive_local()).unwrap_or_default())
}

/// notes since(cutoff, folder) (11 §1).
pub fn notes_since(v: &Vault, cutoff: &NaiveDateTime, folder: Option<&str>) -> Result<Vec<Value>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (abs, rel) in crate::search::walk(v, folder)? {
        let t = mtime_local(&abs)?;
        if t >= *cutoff {
            out.push((minute_stamp(&t), rel));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(out.into_iter().map(|(m, p)| json!({"path": p, "modified": m})).collect())
}

fn one(key: &str, v: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(key.into(), v);
    m
}

/// digest(since, now) (11 §2).
pub fn digest(v: &Vault, state_dir: &Path, since: Option<&str>, now: &NaiveDateTime) -> Result<Value> {
    let cutoff = match since.filter(|s| !s.is_empty()) {
        Some(s) => {
            let bad = || Error::vault(format!("since must be YYYY-MM-DD: {}", crate::text::quote_str(s)));
            let c = SINCE.captures(s).ok_or_else(bad)?;
            let n = |i: usize| c[i].parse::<u32>().unwrap_or(0);
            NaiveDate::from_ymd_opt(n(1) as i32, n(2), n(3)).ok_or_else(bad)?.and_hms_opt(0, 0, 0).unwrap_or_default()
        }
        None => *now - Duration::days(7),
    };
    let recent = || notes_since(v, &cutoff, None);
    let status = || crate::librarian::status(state_dir);
    Ok(json!({
        "mode": "digest",
        "generated": minute_stamp(now),
        "since": crate::text::day(&cutoff),
        "librarian": section(|| Ok(one("report", status()?))),
        "recent_notes": section(|| Ok(one("notes", Value::Array(recent()?)))),
        "inbox_captures": section(|| Ok(one("notes", Value::Array(notes_since(v, &cutoff, Some("Claude/Inbox"))?)))),
        "tasks": section(|| {
            let paths: BTreeSet<String> = recent()?.iter().filter_map(|n| n["path"].as_str().map(String::from)).collect();
            let all = crate::notes::list_tasks(v, true, None)?;
            let (done, open): (Vec<Value>, Vec<Value>) = all.into_iter().partition(|t| t["done"] == json!(true));
            let recently: Vec<Value> = done.into_iter().filter(|t| t["path"].as_str().is_some_and(|p| paths.contains(p))).collect();
            let mut m = one("open", Value::Array(open));
            m.insert("done_recently".into(), Value::Array(recently));
            Ok(m)
        }),
        "proposals": section(|| {
            let pending = status()?.get("health").and_then(|h| h.get("pending_proposals")).cloned().unwrap_or(json!(0));
            let mut m = one("pending", pending);
            m.insert("detail_note".into(), json!(crate::mover::LOG_NOTE));
            Ok(m)
        }),
        "new_entities": section(|| Ok(one("notes", Value::Array(notes_since(v, &cutoff, Some("Claude/Graph"))?)))),
    }))
}

/// resume(project, limit) (11 §3).
pub fn resume(v: &Vault, state_dir: &Path, project: &str, limit: usize) -> Value {
    let q = casefold(project);
    json!({
        "mode": "resume",
        "project": project,
        "sessions": section(|| {
            let mut found: Vec<(String, Value)> = Vec::new();
            if v.resolve("Claude/Sessions")?.is_dir() {
                for (abs, rel) in crate::search::walk(v, Some("Claude/Sessions"))? {
                    let text = read_lenient(&abs)?;
                    let meta = parse(&text)?;
                    let proj = get(&meta, "project").map(display).unwrap_or_default();
                    if !casefold(&format!("{proj} {}", crate::graph::stem(&rel))).contains(&q) {
                        continue;
                    }
                    let created = match get(&meta, "created") {
                        c if truthy(c) => display(c.unwrap_or(&crate::yaml::Y::Null)),
                        _ => minute_stamp(&mtime_local(&abs)?),
                    };
                    let excerpt: String = collapse_ws(body(&text)).chars().take(400).collect();
                    found.push((created.clone(), json!({"path": rel, "created": created, "excerpt": excerpt})));
                }
            }
            found.sort_by(|a, b| b.0.cmp(&a.0));
            Ok(one("notes", found.into_iter().take(limit).map(|(_, n)| n).collect()))
        }),
        "decisions": section(|| {
            let rel = "Claude/Decisions.md";
            let lines: Vec<Value> = if v.resolve(rel)?.is_file() {
                split_lines(&v.read(rel)?).into_iter().filter(|l| l.starts_with("- ") && casefold(l).contains(&q)).map(|l| json!(l)).collect()
            } else {
                vec![]
            };
            Ok(one("lines", Value::Array(lines)))
        }),
        "tasks": section(|| {
            let open = crate::notes::list_tasks(v, false, None)?;
            Ok(one("tasks", open.into_iter().filter(|t| casefold(t["text"].as_str().unwrap_or_default()).contains(&q)).collect()))
        }),
        "entities": section(|| {
            let db = state_dir.join("graph.db");
            if !db.exists() {
                let mut m = one("entities", json!([]));
                m.insert("note".into(), json!("graph cache not built"));
                return Ok(m);
            }
            let found = Graph::load(&db)?.find(project, None);
            Ok(one("entities", found.iter().map(|e| json!({"name": e["name"], "type": e["type"], "path": e["path"], "summary": e["summary"]})).collect()))
        }),
    })
}

/// context_block(project, budget) (11 §4).
pub fn context_block(v: &Vault, state_dir: &Path, project: &str, budget: i64) -> String {
    if project.is_empty() {
        return String::new();
    }
    let b = resume(v, state_dir, project, 10);
    let ok = |k: &str| b[k]["status"] == "ok";
    let mut parts: Vec<String> = Vec::new();
    if ok("sessions") {
        for n in b["sessions"]["notes"].as_array().into_iter().flatten().take(3) {
            let path = n["path"].as_str().unwrap_or_default();
            let last = path.rsplit('/').next().unwrap_or(path);
            let title = last.strip_suffix(".md").unwrap_or(last);
            let ex: String = n["excerpt"].as_str().unwrap_or_default().chars().take(300).collect();
            parts.push(format!("Session: {title} \u{2014} {ex}"));
        }
    }
    if ok("tasks") {
        for t in b["tasks"]["tasks"].as_array().into_iter().flatten().take(5) {
            parts.push(format!("Open task: {}", t["text"].as_str().unwrap_or_default()));
        }
    }
    if ok("decisions") {
        parts.extend(b["decisions"]["lines"].as_array().into_iter().flatten().take(3).filter_map(|l| l.as_str().map(String::from)));
    }
    let text = parts.join("\n");
    let budget = budget.max(0) as usize;
    if text.chars().count() <= budget {
        return text;
    }
    let cut: String = text.chars().take(budget).collect();
    match cut.rfind('\n') {
        Some(i) if i > 0 => cut[..i].to_string(),
        _ => cut,
    }
}
