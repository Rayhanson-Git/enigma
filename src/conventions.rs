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
pub const DECISIONS_SEED: &str = "---\ncreated: 2026-01-01\nagent: claude\ntags: [decisions]\n---\n\n# Decisions\n\nAppend-only log. One line per decision:\n`- YYYY-MM-DD \u{2014} <decision> ([[session note]])`\n\n";

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(&e, parent))?;
    }
    fs::write(path, text).map_err(|e| Error::io(&e, path))
}

/// The display name of the account running the command, if it has one.
#[cfg(windows)]
pub fn system_owner() -> Option<String> {
    #[link(name = "secur32")]
    unsafe extern "system" {
        fn GetUserNameExW(format: i32, buffer: *mut u16, size: *mut u32) -> u8;
    }
    const NAME_DISPLAY: i32 = 3;
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // on success len is the number of characters written, without the final NUL
    if unsafe { GetUserNameExW(NAME_DISPLAY, buf.as_mut_ptr(), &mut len) } == 0 {
        return None; // a local account often has no display name
    }
    clean(&String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]))
}

/// The full name of the account running the command (the first field of its
/// passwd entry), if it has one.
#[cfg(unix)]
pub fn system_owner() -> Option<String> {
    // getpwuid returns a pointer into static storage; the name is copied out at once
    let gecos = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_gecos.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_gecos).to_string_lossy().into_owned()
    };
    clean(gecos.split(',').next().unwrap_or_default())
}

/// Stripped, with each CR and LF turned into a space; None when nothing is left.
fn clean(raw: &str) -> Option<String> {
    let name = raw.trim().replace(['\r', '\n'], " ");
    (!name.is_empty()).then_some(name)
}

/// The owner wording for the guides (15 §2 step 5): the explicit owner, else
/// ENIGMA_OWNER, else the system account's full name, else a neutral phrase.
pub fn owner_name(owner: Option<&str>) -> String {
    let named = match owner {
        Some(o) => clean(o),
        None => clean(&std::env::var("ENIGMA_OWNER").unwrap_or_default()).or_else(system_owner),
    };
    named.unwrap_or_else(|| "the vault owner".into())
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
    let owner = owner_name(owner);
    let (constitution, guide) = (CONSTITUTION.replace("{OWNER}", &owner), GUIDE.replace("{OWNER}", &owner));
    for (rel, text) in
        [("Claude/README.md", constitution.as_str()), ("Claude/Index.md", INDEX_SEED), ("Claude/Decisions.md", DECISIONS_SEED), ("CLAUDE.md", guide.as_str()), ("AGENTS.md", guide.as_str())]
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
