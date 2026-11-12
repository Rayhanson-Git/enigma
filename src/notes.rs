//! Agent notes (03 §7) and tasks (03 §8).
use crate::error::{Error, Kind, Result};
use crate::text::{casefold, clock, collapse_ws, day, minute_stamp, quote_str, split_lines, strip};
use crate::vault::{Vault, read_lenient};
use crate::yaml::{Y, dump_block};
use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub fn safe_filename(title: &str) -> String {
    let kept: String = title.chars().filter(|c| !"\\/:*?\"<>|[]#^".contains(*c)).collect();
    let s = collapse_ws(&kept);
    if s.is_empty() { "untitled".into() } else { s }
}

/// Agent frontmatter block (03 §7.2), fences and blank line included.
pub fn frontmatter(project: &str, tags: &[String], created: &NaiveDateTime, agent: &str) -> String {
    let s = |v: &str| Y::Str(v.to_string());
    let yaml = dump_block(&[
        (s("created"), s(&minute_stamp(created))),
        (s("agent"), s(agent)),
        (s("project"), s(project)),
        (s("tags"), Y::Seq(tags.iter().map(|t| s(t)).collect())),
    ]);
    format!("---\n{yaml}---\n\n")
}

pub fn log_session(v: &Vault, title: &str, content: &str, project: &str, tags: &[String], now: &NaiveDateTime, agent: &str) -> Result<String> {
    let base = format!("{} {}", day(now), safe_filename(title));
    let text = format!("{}{content}\n", frontmatter(project, tags, now, agent));
    let (mut stem, mut n) = (base.clone(), 2);
    let rel = loop {
        let rel = format!("Claude/Sessions/{stem}.md");
        match v.write(&rel, &text, false, false) {
            Ok(_) => break rel,
            Err(e) if e.kind == Kind::NoteExists => {
                stem = format!("{base} {n}");
                n += 1;
            }
            Err(e) => return Err(e),
        }
    };
    v.append("Claude/Index.md", &format!("- [[{stem}]] \u{2014} {project}: {title}\n"), false)?;
    Ok(rel)
}

pub fn capture(v: &Vault, content: &str, now: &NaiveDateTime) -> Result<String> {
    let rel = format!("Claude/Inbox/{}.md", day(now));
    v.append(&rel, &format!("- {} {content}\n", clock(now)), false)?;
    Ok(rel)
}

fn concept_lookup(v: &Vault, filename: &str) -> Option<String> {
    if crate::faults::hit("concept_lookup").is_some() {
        return None;
    }
    let dir = v.resolve("Claude/Concepts").ok().filter(|d| d.is_dir())?;
    let want = casefold(filename);
    let mut names: Vec<String> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names.into_iter().find(|n| casefold(n) == want).map(|n| format!("Claude/Concepts/{n}"))
}

pub fn upsert_concept(v: &Vault, name: &str, content: &str, now: &NaiveDateTime, agent: &str) -> Result<String> {
    let filename = format!("{}.md", safe_filename(name));
    let existing = match concept_lookup(v, &filename) {
        Some(e) => e,
        None => {
            let rel = format!("Claude/Concepts/{filename}");
            let text = format!("{}# {name}\n\n{content}\n", frontmatter("", &["concept".into()], now, agent));
            match v.write(&rel, &text, false, false) {
                Ok(_) => return Ok(rel),
                Err(e) if e.kind == Kind::NoteExists => concept_lookup(v, &filename).unwrap_or(rel),
                Err(e) => return Err(e),
            }
        }
    };
    let mut heading = format!("## Update {}", day(now));
    if agent != "claude" {
        heading.push_str(&format!(" ({agent})"));
    }
    v.append(&existing, &format!("\n{heading}\n\n{content}\n"), false)?;
    Ok(existing)
}

// ------------------------------------------------------------------ tasks

pub const TASKS: &str = "Claude/Tasks.md";
pub const SEED: &str = "---\nagent: claude\ntags: [tasks]\n---\n\n# Tasks\n\n## Live view (all open tasks, vault-wide)\n\n\u{60}\u{60}\u{60}tasks\nnot done\ngroup by filename\n\u{60}\u{60}\u{60}\n\n## Open\n\n";

static DUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{4})-(1[0-2]|0[1-9]|[1-9])-(3[01]|[12][0-9]|0[1-9]|[1-9]| [1-9])").expect("static regex"));

/// Due date validation (03 §8).
pub fn validate_due(due: &str) -> Result<()> {
    let c = DUE.captures(due).ok_or_else(|| {
        Error::invalid(format!("{} is not a date in the form YYYY-MM-DD", quote_str(due)))
    })?;
    let end = c.get(0).map_or(0, |m| m.end());
    if end != due.len() {
        return Err(Error::invalid(format!("unexpected text after the date: {}", &due[end..])));
    }
    let n = |i: usize| c[i].trim().parse::<u32>().unwrap_or(0);
    if n(1) == 0 {
        return Err(Error::invalid("the year must be 1 or later"));
    }
    NaiveDate::from_ymd_opt(n(1) as i32, n(2), n(3)).ok_or_else(|| Error::invalid("that day does not exist in that month"))?;
    Ok(())
}

pub fn add_task(v: &Vault, content: &str, due: Option<&str>) -> Result<String> {
    if !v.resolve(TASKS)?.exists() {
        match v.write(TASKS, SEED, false, false) {
            Err(e) if e.kind != Kind::NoteExists => return Err(e),
            _ => {}
        }
    }
    let mut line = format!("- [ ] {}", collapse_ws(content));
    if let Some(d) = due.filter(|d| !d.is_empty()) {
        validate_due(d)?;
        line.push_str(&format!(" \u{1F4C5} {d}"));
    }
    v.append(TASKS, &format!("{line}\n"), false)?;
    Ok(TASKS.into())
}

static TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\s\x1c-\x1f]*[-*] \[([ xX])\] (.+)$").expect("static regex"));

pub fn list_tasks(v: &Vault, include_done: bool, folder: Option<&str>) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for (abs, rel) in crate::search::walk(v, folder)? {
        let text = read_lenient(&abs)?;
        for line in split_lines(&text) {
            let Some(c) = TASK.captures(line) else { continue };
            let done = &c[1] != " ";
            if done && !include_done {
                continue;
            }
            out.push(json!({"path": rel, "text": strip(&c[2]), "done": done}));
        }
    }
    Ok(out)
}
