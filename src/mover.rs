//! The link-safe mover, journal and undo (09 §4–§5).
use crate::error::{Error, Kind, Result};
use crate::graph::strip_md;
use crate::indexer::Manifest;
use crate::text::casefold;
use crate::vault::{Vault, retry};
use chrono::NaiveDateTime;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const JOURNAL: &str = "organizer_journal.jsonl";
pub const LOG_NOTE: &str = "Claude/Organizer.md";
pub const LOG_SEED: &str = "# Organizer\n\nAutonomous move log and proposals. See constitution \u{2192} Organizer.\n\n## Log\n";

/// Rewrites links to old into links to new (09 §4.1), on raw bytes.
pub fn rewrite(bytes: &[u8], old: &str, new: &str) -> (Vec<u8>, usize) {
    let pat = format!("[[{}", strip_md(old)).into_bytes();
    let rep = format!("[[{}", strip_md(new)).into_bytes();
    let (mut out, mut count, mut i) = (Vec::with_capacity(bytes.len()), 0, 0);
    while i < bytes.len() {
        if bytes[i..].starts_with(&pat) && matches!(bytes.get(i + pat.len()), Some(b']' | b'|' | b'#')) {
            out.extend_from_slice(&rep);
            i += pat.len();
            count += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    (out, count)
}

fn stem_of(p: &str) -> &str {
    let name = p.rsplit('/').next().unwrap_or(p);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// duplicate_stem_exists(path) (09 §4.2).
pub fn duplicate_stem_exists(v: &Vault, path: &str) -> Result<bool> {
    let want = casefold(stem_of(path));
    Ok(crate::search::walk(v, None)?.iter().filter(|(_, rel)| casefold(stem_of(rel)) == want).count() > 1)
}

/// One planned rewrite: (rel, abs, new bytes, count).
pub type Plan = (String, PathBuf, Vec<u8>, usize);

/// Notes whose links to old would change.
pub fn plan(v: &Vault, old: &str, new: &str) -> Result<Vec<Plan>> {
    let mut out = Vec::new();
    for (abs, rel) in crate::search::walk(v, None)? {
        let bytes = fs::read(&abs).map_err(|e| Error::io(&e, &abs))?;
        let (b, n) = rewrite(&bytes, old, new);
        if n > 0 {
            out.push((rel, abs, b, n));
        }
    }
    Ok(out)
}

/// Atomic replace through "{file name}.tmp-move".
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::faults::os("move_write").map_err(|e| Error::io(&e, path))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp-move"));
    fs::write(&tmp, bytes).map_err(|e| Error::io(&e, &tmp))?;
    retry("move_replace", || fs::rename(&tmp, path)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(&e, path)
    })
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|e| Error::io(&e, dir))?;
    }
    retry("move_rename", || fs::rename(from, to)).map_err(|e| Error::io(&e, to))
}

/// move_note(old, new, protect) (09 §4.3).
pub fn move_note(v: &Vault, state_dir: &Path, old: &str, new: &str, protect: Option<&dyn Fn(&str) -> bool>) -> Result<Value> {
    let (old_abs, new_abs) = (v.resolve(old)?, v.resolve(new)?);
    if !old_abs.is_file() {
        return Err(Error::vault(format!("Cannot move: not a file: {old}")));
    }
    if new_abs.exists() {
        return Err(Error::vault(format!("Cannot move: destination exists: {new}")));
    }
    let plans = plan(v, old, new)?;
    if let Some(p) = protect {
        let hit: Vec<&str> = plans.iter().map(|(rel, ..)| rel.as_str()).filter(|r| p(r)).collect();
        if !hit.is_empty() {
            return Err(Error::new(Kind::ProtectedLink, format!("move would edit protected notes: {}", hit.join(", "))));
        }
    }
    rename(&old_abs, &new_abs)?;
    let mut written: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for (rel, abs, bytes, _) in &plans {
        let target = if rel == old { new_abs.clone() } else { abs.clone() };
        let original = match fs::read(&target) {
            Ok(b) => b,
            Err(e) => {
                rollback(&written, &old_abs, &new_abs);
                return Err(Error::io(&e, &target));
            }
        };
        if let Err(e) = replace(&target, bytes) {
            rollback(&written, &old_abs, &new_abs);
            return Err(e);
        }
        written.push((target, original));
    }
    Manifest::rename_entry(state_dir, old, new)?;
    let rewrites: Vec<Value> = plans.iter().map(|(rel, _, _, n)| json!({"path": rel, "count": n})).collect();
    Ok(json!({"from": old, "to": new, "rewrites": rewrites}))
}

fn rollback(written: &[(PathBuf, Vec<u8>)], old_abs: &Path, new_abs: &Path) {
    for (p, bytes) in written.iter().rev() {
        let _ = fs::write(p, bytes);
    }
    let _ = fs::rename(new_abs, old_abs);
}

/// The ProtectedLink error's path list (09 §3 row 8).
pub fn protected_paths(e: &Error) -> Vec<String> {
    e.msg.strip_prefix("move would edit protected notes: ").map(|s| s.split(", ").map(String::from).collect()).unwrap_or_default()
}

/// reverse_rewrites(old, new, paths) (09 §4.4).
pub fn reverse_rewrites(v: &Vault, old: &str, new: &str, paths: &[String]) -> Result<()> {
    for p in paths {
        let abs = v.resolve(p)?;
        if !abs.is_file() {
            continue;
        }
        let bytes = fs::read(&abs).map_err(|e| Error::io(&e, &abs))?;
        let (b, n) = rewrite(&bytes, new, old);
        if n > 0 {
            replace(&abs, &b)?;
        }
    }
    Ok(())
}

pub fn second_stamp(now: &NaiveDateTime) -> String {
    now.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Makes sure the move log note exists (create-only seed when unreadable).
pub fn ensure_log(v: &Vault) -> Result<()> {
    if v.read(LOG_NOTE).is_ok() {
        return Ok(());
    }
    match v.write(LOG_NOTE, LOG_SEED, false, false) {
        Err(e) if e.kind != Kind::NoteExists => Err(e),
        _ => Ok(()),
    }
}

fn journal_path(state_dir: &Path) -> PathBuf {
    state_dir.join(JOURNAL)
}

/// record_move(record, share, neighbours) (09 §5).
pub fn record_move(v: &Vault, state_dir: &Path, rec: &Value, share: f64, neighbours: &[String], now: &NaiveDateTime) -> Result<()> {
    let ts = second_stamp(now);
    let line = json!({
        "ts": ts, "from": rec["from"], "to": rec["to"], "share": share,
        "neighbors": neighbours.iter().take(3).collect::<Vec<_>>(), "rewrites": rec["rewrites"], "undone": false,
    });
    let jp = journal_path(state_dir);
    let mut f = fs::OpenOptions::new().append(true).create(true).open(&jp).map_err(|e| Error::io(&e, &jp))?;
    writeln!(f, "{}", crate::json::compact(&line)).map_err(|e| Error::io(&e, &jp))?;
    ensure_log(v)?;
    let to = rec["to"].as_str().unwrap_or_default();
    let from = rec["from"].as_str().unwrap_or_default();
    v.append(
        LOG_NOTE,
        &format!("- {ts} \u{2014} moved [[{}|{}]] from \u{60}{from}\u{60} (share {share:.2})\n", strip_md(to), stem_of(to)),
        false,
    )?;
    Ok(())
}

fn read_journal(state_dir: &Path) -> Result<Vec<Value>> {
    let jp = journal_path(state_dir);
    let text = match fs::read_to_string(&jp) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(Error::io(&e, &jp)),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| Error::new(Kind::InvalidData, format!("bad journal line in {}: {e}", jp.display()))))
        .collect()
}

/// undo_move(path) (09 §5).
pub fn undo_move(v: &Vault, state_dir: &Path, path: &str, now: &NaiveDateTime) -> Result<Value> {
    let mut entries = read_journal(state_dir)?;
    let Some(i) = entries.iter().rposition(|e| e["to"].as_str() == Some(path) && e["undone"] != json!(true)) else {
        return Err(Error::vault(format!("No undoable move found for: {path}")));
    };
    let from = entries[i]["from"].as_str().unwrap_or_default().to_string();
    let to = entries[i]["to"].as_str().unwrap_or_default().to_string();
    let (from_abs, to_abs) = (v.resolve(&from)?, v.resolve(&to)?);
    if !to_abs.is_file() {
        return Err(Error::vault(format!("Cannot undo: file no longer at {to}")));
    }
    if from_abs.exists() {
        return Err(Error::vault(format!("Cannot undo: original location occupied: {from}")));
    }
    rename(&to_abs, &from_abs)?;
    let paths: Vec<String> =
        entries[i]["rewrites"].as_array().into_iter().flatten().filter_map(|r| r["path"].as_str().map(String::from)).collect();
    reverse_rewrites(v, &from, &to, &paths)?;
    Manifest::rename_entry(state_dir, &to, &from)?;
    entries[i]["undone"] = json!(true);
    let text: String = entries.iter().map(|e| format!("{}\n", crate::json::compact(e))).collect();
    let jp = journal_path(state_dir);
    fs::write(&jp, text).map_err(|e| Error::io(&e, &jp))?;
    ensure_log(v)?;
    v.append(LOG_NOTE, &format!("- {} \u{2014} UNDID move of \u{60}{to}\u{60} back to \u{60}{from}\u{60}\n", second_stamp(now)), false)?;
    Ok(json!({"restored": from, "was": to}))
}
