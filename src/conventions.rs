//! Bundled assets, the conventions install and skill-sync (15 §1, §2, §8).
use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub const CONSTITUTION: &str = include_str!("../spec/constitution/README.md");
pub const GUIDE: &str = include_str!("../spec/constitution/vault-guide.md");

macro_rules! skills {
    ($($name:literal),*) => {
        /// The bundled skills, sorted by name: (name, SKILL.md bytes).
        pub const SKILLS: &[(&str, &[u8])] = &[$(($name, include_bytes!(concat!("../skills/", $name, "/SKILL.md")))),*];
    };
}
skills!("clip", "connections", "decide", "digest", "distill", "dossier", "recall", "resume", "rounds");

pub const FOLDERS: [&str; 5] = ["Claude/Inbox", "Claude/Sessions", "Claude/Concepts", "Claude/Answers", "Claude/Digests"];
pub const INDEX_SEED: &str = "# Index\n\nMap of agent-written notes. Session entries append below.\n\n";
pub const DECISIONS_SEED: &str = "---\ncreated: 2026-07-06\nagent: claude\ntags: [decisions]\n---\n\n# Decisions\n\nAppend-only log. One line per decision:\n`- YYYY-MM-DD \u{2014} <decision> ([[session note]])`\n\n";

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(&e, parent))?;
    }
    fs::write(path, text).map_err(|e| Error::io(&e, path))
}

/// The owner wording for the guides (15 §2 step 5).
pub fn owner_name(owner: Option<&str>) -> String {
    let raw = match owner {
        Some(o) => o.to_string(),
        None => std::env::var("ENIGMA_OWNER").unwrap_or_default(),
    };
    let name = raw.trim().replace(['\r', '\n'], " ");
    if name.is_empty() { "the vault owner".into() } else { name }
}

/// install(vault, owner) → created vault-relative paths (15 §2).
pub fn install(vault: &Path, owner: Option<&str>) -> Result<Vec<String>> {
    let mut created = Vec::new();
    for f in FOLDERS {
        let p = vault.join(f);
        if !p.is_dir() {
            fs::create_dir_all(&p).map_err(|e| Error::io(&e, &p))?;
            created.push(f.to_string());
        }
    }
    let guide = GUIDE.replace("{OWNER}", &owner_name(owner));
    for (rel, text) in
        [("Claude/README.md", CONSTITUTION), ("Claude/Index.md", INDEX_SEED), ("Claude/Decisions.md", DECISIONS_SEED), ("CLAUDE.md", &guide), ("AGENTS.md", &guide)]
    {
        let p = vault.join(rel);
        if p.symlink_metadata().is_err() {
            write(&p, text)?;
            created.push(rel.to_string());
        }
    }
    Ok(created)
}

// ---------------------------------------------------------------- skill-sync (§8)

/// Every file under a folder as (relative path, bytes), sorted.
type Tree = Vec<(String, Vec<u8>)>;

/// The tree of dir; None when dir is absent.
fn tree(dir: &Path) -> Result<Option<Tree>> {
    if !dir.exists() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let e = e.map_err(|e| Error::new(crate::error::Kind::IoError, e.to_string()))?;
        if e.file_type().is_file() {
            let rel = e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
            out.push((rel, fs::read(e.path()).map_err(|err| Error::io(&err, e.path()))?));
        }
    }
    Ok(Some(out))
}

fn copy_skill(dest: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(dest).map_err(|e| Error::io(&e, dest))?;
    let p = dest.join("SKILL.md");
    fs::write(&p, bytes).map_err(|e| Error::io(&e, &p))
}

/// skill_sync(dest, check, force) → {installed, updated, up_to_date, drift}.
pub fn skill_sync(dest: &Path, check: bool, force: bool) -> Result<Value> {
    let (mut installed, mut updated, mut up_to_date, mut drift) = (vec![], vec![], vec![], vec![]);
    for (name, bytes) in SKILLS {
        let d = dest.join(name);
        match tree(&d)? {
            None => {
                if !check {
                    copy_skill(&d, bytes)?;
                }
                installed.push(*name);
            }
            Some(files) if files == [("SKILL.md".to_string(), bytes.to_vec())] => up_to_date.push(*name),
            Some(_) if force && !check => {
                fs::remove_dir_all(&d).map_err(|e| Error::io(&e, &d))?;
                copy_skill(&d, bytes)?;
                updated.push(*name);
            }
            Some(_) => drift.push(*name),
        }
    }
    Ok(json!({"installed": installed, "updated": updated, "up_to_date": up_to_date, "drift": drift}))
}
