//! The incremental indexer (06 §2–§4).
use crate::embed::Embedder;
use crate::error::{Error, Kind, Result};
use crate::extractor::Extract;
use crate::graph::{self, Graph};
use crate::vault::Vault;
use chrono::NaiveDateTime;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const DEFAULT_BATCH: usize = 25;
const MAX_ATTEMPTS: i64 = 3;
pub const CARETAKER_NOTES: [&str; 2] = ["Claude/Organizer.md", "Claude/Librarian.md"];
const DEFAULT_IGNORE: [&str; 1] = ["copilot"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Failure {
    pub error: String,
    pub attempts: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Manifest {
    pub hashes: IndexMap<String, String>,
    pub failures: IndexMap<String, Failure>,
}

impl Manifest {
    pub fn path(state_dir: &Path) -> std::path::PathBuf {
        state_dir.join("manifest.json")
    }

    /// Load (06 §2).
    pub fn load(state_dir: &Path) -> Result<Manifest> {
        let p = Self::path(state_dir);
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
            Err(e) => return Err(Error::io(&e, &p)),
        };
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::new(Kind::InvalidData, format!("unreadable manifest {}: {e}", p.display())))?;
        let mut m = Manifest::default();
        if let Some(h) = v.get("hashes").and_then(Value::as_object) {
            for (k, val) in h {
                m.hashes.insert(k.clone(), val.as_str().map_or_else(|| val.to_string(), String::from));
            }
        }
        if let Some(f) = v.get("failures").and_then(Value::as_object) {
            for (k, val) in f {
                let fail = match val {
                    Value::String(s) => Failure { error: s.clone(), attempts: 1 },
                    other => Failure {
                        error: other.get("error").and_then(Value::as_str).unwrap_or_default().to_string(),
                        attempts: other.get("attempts").and_then(Value::as_i64).unwrap_or(1),
                    },
                };
                m.failures.insert(k.clone(), fail);
            }
        }
        for c in CARETAKER_NOTES {
            m.hashes.shift_remove(c);
            m.failures.shift_remove(c);
        }
        Ok(m)
    }

    /// Save: plain overwrite, indented File JSON without a trailing newline.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let p = Self::path(state_dir);
        std::fs::write(&p, crate::json::indented(self)).map_err(|e| Error::io(&e, &p))
    }

    /// rename_manifest_entry(old, new) (06 §2).
    pub fn rename_entry(state_dir: &Path, old: &str, new: &str) -> Result<()> {
        let mut m = Self::load(state_dir)?;
        let mut moved = false;
        if let Some(h) = m.hashes.shift_remove(old) {
            m.hashes.insert(new.into(), h);
            moved = true;
        }
        if let Some(f) = m.failures.shift_remove(old) {
            m.failures.insert(new.into(), f);
            moved = true;
        }
        if moved { m.save(state_dir) } else { Ok(()) }
    }
}

/// scan_notes(vault) (06 §3): path → SHA-256 of the raw bytes.
pub fn scan_notes(v: &Vault) -> Result<IndexMap<String, String>> {
    let mut out = IndexMap::new();
    for (abs, rel) in crate::search::walk(v, None)? {
        let first = rel.split('/').next().unwrap_or("");
        if rel.starts_with("Claude/Graph/") || CARETAKER_NOTES.contains(&rel.as_str()) || DEFAULT_IGNORE.contains(&first) {
            continue;
        }
        // unreadable: the empty hash keeps it pending, so run records the read error per note
        let hash = std::fs::read(&abs).map(|bytes| Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()).unwrap_or_default();
        out.insert(rel, hash);
    }
    Ok(out)
}

/// pending_notes(manifest, scan) (06 §3) → (pending, benched).
pub fn pending_notes(m: &Manifest, scan: &IndexMap<String, String>) -> (Vec<String>, i64) {
    let (mut pending, mut benched) = (Vec::new(), 0);
    for (path, hash) in scan {
        match m.failures.get(path) {
            Some(f) if f.attempts >= MAX_ATTEMPTS => benched += 1,
            Some(_) => pending.push(path.clone()),
            None if m.hashes.get(path) != Some(hash) => pending.push(path.clone()),
            None => {}
        }
    }
    (pending, benched)
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Counts {
    pub processed: i64,
    pub entities_created: i64,
    pub entities_merged: i64,
    pub mentions_added: i64,
    pub relations_added: i64,
    pub mentions_retracted: i64,
    pub failed: i64,
    pub skipped: i64,
    pub remaining: i64,
}

pub struct Options {
    pub batch: usize,
    pub force: bool,
    pub retry_failures: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { batch: DEFAULT_BATCH, force: false, retry_failures: false }
    }
}

/// The per-note work of run (06 §4): read, extract, retract stale mentions, apply.
fn index_note(v: &Vault, path: &str, ex: &dyn Extract, cache: Option<&Graph>, now: &NaiveDateTime) -> Result<(graph::ApplyCounts, i64)> {
    let text = v.read(path)?;
    let x = ex.extract(path, &text)?;
    let mut retracted = 0;
    for ent in cache.map(|g| g.note_entity_paths(path)).unwrap_or_default() {
        match graph::remove_mention(v, &format!("{ent}.md"), path) {
            Ok(true) => retracted += 1,
            Ok(false) => {}
            Err(e) if e.is(Kind::VaultError) => {}
            Err(e) => return Err(e),
        }
    }
    Ok((graph::apply(v, path, &x, now)?, retracted))
}

/// run (06 §4); emb None skips the embedding precompute.
pub fn run(v: &Vault, state_dir: &Path, ex: &dyn Extract, emb: Option<&dyn Embedder>, o: &Options, now: &NaiveDateTime) -> Result<Counts> {
    let mut m = Manifest::load(state_dir)?;
    if o.retry_failures {
        m.failures.clear();
    }
    let scan = scan_notes(v)?;
    let (pending, skipped) = if o.force { (scan.keys().cloned().collect(), 0) } else { pending_notes(&m, &scan) };
    let mut c = Counts { skipped, remaining: pending.len().saturating_sub(o.batch) as i64, ..Default::default() };
    let db = state_dir.join("graph.db");
    let cache = match db.exists() {
        false => None,
        true => match Graph::load(&db) {
            Ok(g) => Some(g),
            // an unusable cache is rebuilt from the markdown, not a reason to stop
            Err(_) => {
                graph::rebuild(v, &db)?;
                Some(Graph::load(&db)?)
            }
        },
    };
    for path in pending.iter().take(o.batch) {
        match index_note(v, path, ex, cache.as_ref(), now) {
            Ok((a, retracted)) => {
                c.entities_created += a.entities_created;
                c.entities_merged += a.entities_merged;
                c.mentions_added += a.mentions_added;
                c.relations_added += a.relations_added;
                c.mentions_retracted += retracted;
                m.hashes.insert(path.clone(), scan[path].clone());
                m.failures.shift_remove(path);
                c.processed += 1;
            }
            // errors never stop the run: record them and retry on later runs (06 §4)
            Err(e) => {
                let msg = if e.kind == Kind::ExtractorError { e.msg } else { e.to_string() };
                let attempts = m.failures.get(path).map_or(0, |f| f.attempts) + 1;
                m.failures.insert(path.clone(), Failure { error: msg.chars().take(300).collect(), attempts });
                c.failed += 1;
            }
        }
    }
    m.save(state_dir)?;
    if c.processed > 0 || !db.exists() {
        graph::rebuild(v, &db)?;
    }
    if let Some(emb) = emb {
        // notes that cannot be read are left out; their failures are in the manifest
        let texts: Vec<(String, String)> =
            crate::search::walk(v, None)?.into_iter().filter_map(|(abs, rel)| crate::vault::read_lenient(&abs).ok().map(|t| (rel, t))).collect();
        crate::search::note_vectors(v, state_dir, emb, Some(&texts), true)?;
    }
    Ok(c)
}
