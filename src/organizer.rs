//! The organizer: neighbour-vote filing sweep (09 §1–§3).
use crate::embed::Embedder;
use crate::error::{Error, Kind, Result};
use crate::mover;
use crate::search::{cosine, note_vectors};
use crate::vault::{Vault, get, parse};
use crate::yaml::Y;
use chrono::NaiveDateTime;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

pub const EXCLUDED_DIRS: [&str; 7] = ["Claude", "00 - Maps of Content", ".obsidian", ".smart-env", ".trash", ".space", "copilot"];
pub const EXCLUDED_ROOT_FILES: [&str; 3] = ["CLAUDE.md", "AGENTS.md", "README.md"];
const VOTE_K: usize = 10;
const VOTE_THRESHOLD: f64 = 0.7;
/// Days a suggestion may stay unticked before it is dropped (09 §3.1).
pub const SUGGESTION_TTL_DAYS: i64 = 15;
pub const SUGGESTIONS_FILE: &str = "organizer_suggestions.json";
pub const BLOCK_START: &str = "<!-- enigma:suggestions:start -->";
pub const BLOCK_END: &str = "<!-- enigma:suggestions:end -->";

/// A path whose parent folder is a sheet folder (09 §1).
pub fn in_sheet_folder(v: &Vault, path: &str) -> bool {
    match path.rsplit_once('/') {
        Some((parent, _)) => crate::sheets::is_sheet_folder(v, parent),
        None => false,
    }
}

pub fn taxonomy(v: &Vault) -> Result<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(&v.root)
        .map_err(|e| crate::error::Error::io(&e, &v.root))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !EXCLUDED_DIRS.contains(&n.as_str()) && !n.starts_with('.') && !crate::sheets::is_sheet_folder(v, n))
        .collect();
    out.sort();
    Ok(out)
}

pub fn organized(v: &Vault, tax: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for f in tax {
        out.extend(crate::search::walk(v, Some(f))?.into_iter().map(|(_, rel)| rel));
    }
    Ok(out)
}

pub fn candidates(v: &Vault, labelled: &[String]) -> Result<Vec<String>> {
    let mut root: Vec<String> = std::fs::read_dir(&v.root)
        .map_err(|e| crate::error::Error::io(&e, &v.root))?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md") && !EXCLUDED_ROOT_FILES.contains(&n.as_str()))
        .collect();
    root.sort();
    let mut out = Vec::new();
    for p in root.into_iter().chain(labelled.iter().cloned()) {
        if in_sheet_folder(v, &p) {
            continue;
        }
        if matches!(get(&parse(&v.read(&p)?)?, "organize"), Some(Y::Bool(false))) {
            continue;
        }
        out.push(p);
    }
    Ok(out)
}

/// classify(path, vectors, labelled) (09 §2).
pub fn classify(path: &str, vectors: &HashMap<String, Vec<f64>>, labelled: &[String]) -> (Option<String>, f64, Vec<String>) {
    let Some(me) = vectors.get(path) else { return (None, 0.0, vec![]) };
    let mut scored: Vec<(&str, f64)> = labelled
        .iter()
        .filter(|l| l.as_str() != path)
        .filter_map(|l| vectors.get(l).map(|vec| (l.as_str(), cosine(me, vec))))
        .filter(|(_, s)| *s > 0.0)
        .collect();
    if scored.is_empty() {
        return (None, 0.0, vec![]);
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(VOTE_K);
    let mut votes: Vec<(&str, f64)> = Vec::new();
    for (p, s) in &scored {
        let folder = p.split('/').next().unwrap_or("");
        match votes.iter_mut().find(|(f, _)| *f == folder) {
            Some(v) => v.1 += s,
            None => votes.push((folder, *s)),
        }
    }
    let total: f64 = votes.iter().map(|(_, s)| s).sum();
    let best = votes.iter().map(|(_, s)| *s).fold(f64::MIN, f64::max);
    let winner = votes.iter().find(|(_, s)| *s == best).map(|(f, _)| f.to_string());
    (winner, best / total, scored.into_iter().map(|(p, _)| p.to_string()).collect())
}

/// Rounded to 3 decimals, half-to-even on the exact binary value.
fn round3(x: f64) -> f64 {
    format!("{x:.3}").parse().unwrap_or(x)
}

/// run(vault, embedder, apply, max_moves) (09 §3).
pub fn run(v: &Vault, state_dir: &Path, emb: &dyn Embedder, apply: bool, max_moves: Option<i64>, now: &NaiveDateTime) -> Result<Value> {
    let vectors: HashMap<String, Vec<f64>> = note_vectors(v, state_dir, emb, None, apply)?.into_iter().collect();
    let mut sugg = load_suggestions(state_dir)?;
    let note = v.read(mover::LOG_NOTE).ok();
    let ticked = note.as_deref().map(ticked_pairs).unwrap_or_default();
    let acked = |path: &str, folder: &str| {
        sugg.acknowledged.iter().map(|a| (a.path.as_str(), a.suggested.as_str())).chain(ticked.iter().map(|(p, f)| (p.as_str(), f.as_str()))).any(|pair| pair == (path, folder))
    };
    let tax = taxonomy(v)?;
    let labelled = organized(v, &tax)?;
    let cands = candidates(v, &labelled)?;
    let (mut moved, mut proposals, mut skipped) = (Vec::<Value>::new(), Vec::<Value>::new(), Vec::<Value>::new());
    let skip = |p: &str, r: &str| json!({"path": p, "reason": r});
    for c in &cands {
        let current = c.split_once('/').map(|(f, _)| f.to_string()).filter(|f| tax.contains(f));
        let (folder, share, neigh) = classify(c, &vectors, &labelled);
        let three: Vec<&String> = neigh.iter().take(3).collect();
        let propose = |sug: &str, reason: &str| json!({"path": c, "suggested": sug, "share": round3(share), "neighbors": three, "reason": reason});
        let Some(folder) = folder else {
            skipped.push(skip(c, "no vector or no labeled neighbors"));
            continue;
        };
        if Some(&folder) == current.as_ref() {
            skipped.push(skip(c, "already correctly filed"));
            continue;
        }
        if share < VOTE_THRESHOLD {
            match current {
                None => proposals.push(propose(&folder, "low confidence")),
                Some(_) => skipped.push(skip(c, "low-confidence disagreement")),
            }
            continue;
        }
        if mover::duplicate_stem_exists(v, c)? {
            proposals.push(propose(&folder, "duplicate stem \u{2014} bare links would become ambiguous"));
            continue;
        }
        if max_moves.is_some_and(|m| moved.len() as i64 >= m) {
            skipped.push(skip(c, "move cap reached"));
            continue;
        }
        let target = format!("{folder}/{}", c.rsplit('/').next().unwrap_or(c));
        let sheet_reason = |paths: Vec<String>| format!("linked from sheet rows: {}", paths.into_iter().take(3).collect::<Vec<_>>().join(", "));
        if !apply {
            let hit: Vec<String> = mover::plan(v, c, &target)?.into_iter().map(|(rel, ..)| rel).filter(|r| in_sheet_folder(v, r)).collect();
            if !hit.is_empty() {
                proposals.push(propose(&folder, &sheet_reason(hit)));
                continue;
            }
        } else {
            let protect = |p: &str| in_sheet_folder(v, p);
            mover::journal_ready(state_dir)?;
            match mover::move_note(v, state_dir, c, &target, Some(&protect)) {
                Ok(rec) => mover::record_move(v, state_dir, &rec, share, &neigh, now)?,
                Err(e) if e.kind == Kind::ProtectedLink => {
                    proposals.push(propose(&folder, &sheet_reason(mover::protected_paths(&e))));
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        moved.push(json!({"from": c, "to_folder": folder, "share": round3(share), "neighbors": three}));
    }
    // acknowledged suggestions are never proposed again (09 §3)
    let (acknowledged, proposals): (Vec<Value>, Vec<Value>) =
        proposals.into_iter().partition(|p| acked(p["path"].as_str().unwrap_or_default(), p["suggested"].as_str().unwrap_or_default()));
    skipped.extend(acknowledged.iter().map(|p| skip(p["path"].as_str().unwrap_or_default(), "suggestion acknowledged")));
    let cache_rebuilt = apply && !moved.is_empty();
    if cache_rebuilt {
        crate::graph::rebuild(v, &state_dir.join("graph.db"))?;
        // moved notes have new paths and rewritten notes new text: embed them now, not next sweep
        note_vectors(v, state_dir, emb, None, true)?;
    }
    if apply {
        update_block(v, state_dir, &mut sugg, &ticked, &proposals, now)?;
    }
    Ok(json!({"moved": moved, "proposals": proposals, "skipped": skipped, "cache_rebuilt": cache_rebuilt}))
}

// ---------------------------------------------------------------- suggestions block (09 §3.1)

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Open {
    pub path: String,
    pub suggested: String,
    pub reason: String,
    pub proposed: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Acknowledged {
    pub path: String,
    pub suggested: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Suggestions {
    #[serde(default)]
    pub open: Vec<Open>,
    #[serde(default)]
    pub acknowledged: Vec<Acknowledged>,
}

pub fn load_suggestions(state_dir: &Path) -> Result<Suggestions> {
    let p = state_dir.join(SUGGESTIONS_FILE);
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| Error::new(Kind::InvalidData, format!("unreadable suggestions state {}: {e}", p.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Suggestions::default()),
        Err(e) => Err(Error::io(&e, &p)),
    }
}

/// (start, end) byte range of the block, end marker included.
fn block_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find(BLOCK_START)?;
    let end = start + text[start..].find(BLOCK_END)? + BLOCK_END.len();
    Some((start, end))
}

static TICK: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?m)^- \[[xX]\] `([^`]*)` \u{2192} \*\*(.*?)\*\*").expect("static regex"));

/// The (path, folder) pairs ticked inside the block.
pub fn ticked_pairs(note: &str) -> Vec<(String, String)> {
    let Some((a, b)) = block_range(note) else { return vec![] };
    TICK.captures_iter(&note[a..b]).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

pub fn render_block(open: &[Open]) -> String {
    let mut s = format!("{BLOCK_START}\n## Suggestions\n\nTick a box to dismiss a suggestion; it will not be proposed again. Enigma never moves a note because of this list.\n\n");
    if open.is_empty() {
        s.push_str("_No open suggestions._\n");
    }
    for o in open {
        s.push_str(&format!("- [ ] \u{60}{}\u{60} \u{2192} **{}** ({}) \u{2014} proposed {}\n", o.path, o.suggested, o.reason, o.proposed));
    }
    s.push_str(BLOCK_END);
    s
}

/// Steps 1–7 of 09 §3.1 for an applying run.
fn update_block(v: &Vault, state_dir: &Path, sugg: &mut Suggestions, ticked: &[(String, String)], proposals: &[Value], now: &NaiveDateTime) -> Result<()> {
    let before = sugg.clone();
    let stamp = mover::second_stamp(now);
    let today = now.date();
    let mut history = Vec::new();
    let mut log = |what: &str, path: &str, folder: &str| history.push(format!("- {stamp} \u{2014} {what}: \u{60}{path}\u{60} \u{2192} **{folder}**\n"));
    for (path, folder) in ticked {
        if !sugg.acknowledged.iter().any(|a| (&a.path, &a.suggested) == (path, folder)) {
            sugg.acknowledged.push(Acknowledged { path: path.clone(), suggested: folder.clone(), at: stamp.clone() });
            log("suggestion acknowledged", path, folder);
        }
    }
    let current: Vec<(&str, &str, &str)> =
        proposals.iter().map(|p| (p["path"].as_str().unwrap_or_default(), p["suggested"].as_str().unwrap_or_default(), p["reason"].as_str().unwrap_or_default())).collect();
    let (mut open, mut expired) = (Vec::new(), Vec::new());
    for mut o in std::mem::take(&mut sugg.open) {
        if ticked.iter().any(|(p, f)| (p, f) == (&o.path, &o.suggested)) {
            continue;
        }
        match current.iter().find(|(p, f, _)| (*p, *f) == (o.path.as_str(), o.suggested.as_str())) {
            None if !v.root.join(&o.path).is_file() => log("suggestion dropped (note moved or deleted)", &o.path, &o.suggested),
            None => log("suggestion dropped (no longer applies)", &o.path, &o.suggested),
            Some((_, _, reason)) => {
                let age = chrono::NaiveDate::parse_from_str(&o.proposed, "%Y-%m-%d").map_or(SUGGESTION_TTL_DAYS, |d| (today - d).num_days());
                if age >= SUGGESTION_TTL_DAYS {
                    log("suggestion expired after 15 days", &o.path, &o.suggested);
                    expired.push((o.path.clone(), o.suggested.clone()));
                } else {
                    o.reason = reason.to_string();
                    open.push(o);
                }
            }
        }
    }
    for (path, folder, reason) in current {
        let known = open.iter().any(|o| (o.path.as_str(), o.suggested.as_str()) == (path, folder)) || expired.iter().any(|(p, f)| (p.as_str(), f.as_str()) == (path, folder));
        if !known {
            open.push(Open { path: path.into(), suggested: folder.into(), reason: reason.into(), proposed: crate::text::day(now) });
        }
    }
    sugg.open = open;
    let note = v.read(mover::LOG_NOTE).ok();
    let has_block = note.as_deref().and_then(block_range).is_some();
    if has_block || !sugg.open.is_empty() || !history.is_empty() {
        if note.is_none() {
            mover::ensure_log(v)?;
        }
        let text = v.read(mover::LOG_NOTE)?;
        let block = render_block(&sugg.open);
        let mut new = match block_range(&text) {
            Some((a, b)) => format!("{}{block}{}", &text[..a], &text[b..]),
            None => match text.find("\n## Log\n").map(|i| i + 1).or_else(|| text.starts_with("## Log\n").then_some(0)) {
                Some(i) => format!("{}{block}\n\n{}", &text[..i], &text[i..]),
                None => format!("{}\n{block}\n", if text.ends_with('\n') || text.is_empty() { text.clone() } else { format!("{text}\n") }),
            },
        };
        if !new.ends_with('\n') {
            new.push('\n');
        }
        new.extend(history);
        if new != text {
            v.write(mover::LOG_NOTE, &new, true, false)?;
        }
    }
    if *sugg != before {
        let p = state_dir.join(SUGGESTIONS_FILE);
        std::fs::write(&p, crate::json::indented(&*sugg)).map_err(|e| Error::io(&e, &p))?;
    }
    Ok(())
}
