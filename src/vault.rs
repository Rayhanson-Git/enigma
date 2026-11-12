//! Vault access layer (03): path resolution, quarantine, read, atomic write,
//! append, frontmatter.
use crate::error::{Error, Kind, Result};
use crate::faults::{self, Fault};
use crate::text::{decode_lenient, decode_strict, display, normalize_newlines};
use crate::yaml::{Y, parse_doc};
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

// POSIX errno values (identical on Linux and macOS).
const EPERM: i32 = 1;
const EACCES: i32 = 13;

/// Resolves an absolute path like a non-strict realpath: "." and ".." are
/// collapsed, symlinks of existing components followed, a missing tail kept.
pub fn resolve_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut todo: VecDeque<PathBuf> = p.components().map(|c| PathBuf::from(c.as_os_str())).collect();
    let mut hops = 0;
    while let Some(part) = todo.pop_front() {
        match part.components().next() {
            Some(Component::RootDir) | Some(Component::Prefix(_)) => out = part,
            Some(Component::CurDir) | None => {}
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::Normal(_)) => {
                let cand = out.join(&part);
                match fs::read_link(&cand) {
                    Ok(target) if hops < 40 => {
                        hops += 1;
                        for c in target.components().rev() {
                            todo.push_front(PathBuf::from(c.as_os_str()));
                        }
                    }
                    _ => out = cand,
                }
            }
        }
    }
    out
}

fn is_lock_err(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(EACCES) | Some(EPERM))
}

fn backoff(k: u64) {
    std::thread::sleep(Duration::from_millis(200 * k));
}

/// The lock retry rule (03 §5.1): up to 5 attempts, lock errors only.
pub(crate) fn retry<T>(op: &'static str, mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut k = 1;
    loop {
        match faults::os(op).and_then(|_| f()) {
            Err(e) if is_lock_err(&e) && k < 5 => {
                backoff(k);
                k += 1;
            }
            r => return r,
        }
    }
}

pub struct Vault {
    pub root: PathBuf,
}

fn dir_error(rel: &str) -> Error {
    Error::vault(format!("'{rel}' is a directory, not a note."))
}

fn exists_error(rel: &str) -> Error {
    Error::new(Kind::NoteExists, format!("'{rel}' already exists. Pass overwrite=True to replace it."))
}

impl Vault {
    pub fn open(p: &Path) -> Result<Vault> {
        Ok(Vault { root: crate::config::open_root(p)? })
    }

    /// The server's vault from ENIGMA_VAULT_PATH.
    pub fn from_env() -> Result<Vault> {
        Ok(Vault { root: crate::config::vault_root()? })
    }

    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let p = resolve_path(&self.root.join(rel));
        if p.starts_with(&self.root) { Ok(p) } else { Err(Error::vault(format!("Path escapes the vault: {rel}"))) }
    }

    pub fn in_claude(&self, resolved: &Path) -> bool {
        resolved.starts_with(self.root.join("Claude"))
    }

    /// rel path of an absolute path inside the vault, "/"-joined.
    pub fn rel_of(&self, abs: &Path) -> String {
        let r = abs.strip_prefix(&self.root).unwrap_or(abs);
        r.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
    }

    fn checked_target(&self, rel: &str, confirm: bool) -> Result<PathBuf> {
        let target = self.resolve(rel)?;
        if !confirm && !self.in_claude(&target) {
            return Err(Error::vault(format!(
                "'{rel}' is outside Claude/. Pass confirm_outside_claude=True only when the user explicitly asked for this write."
            )));
        }
        if target.is_dir() {
            return Err(dir_error(rel));
        }
        Ok(target)
    }

    /// Strict read (03 §4).
    pub fn read(&self, rel: &str) -> Result<String> {
        let p = self.resolve(rel)?;
        if p.is_dir() {
            return Err(dir_error(rel));
        }
        if !p.is_file() {
            return Err(Error::vault(format!("Note not found: {rel}")));
        }
        let bytes = retry("read", || fs::read(&p)).map_err(|e| Error::io(&e, &p))?;
        let text = decode_strict(&bytes).map_err(|m| Error::new(Kind::DecodeError, m))?;
        Ok(normalize_newlines(text))
    }

    /// Atomic write (03 §5.2); returns the resolved path.
    pub fn write(&self, rel: &str, content: &str, overwrite: bool, confirm: bool) -> Result<PathBuf> {
        let target = self.checked_target(rel, confirm)?;
        if !overwrite && target.exists() {
            return Err(exists_error(rel));
        }
        let dir = target.parent().unwrap_or(&self.root).to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| Error::io(&e, &dir))?;
        let tmp = dir.join(format!(".{:016x}.tmp-write", fastrand::u64(..)));
        let res = self.publish(rel, &tmp, &target, content.as_bytes(), overwrite);
        let _ = retry("temp_delete", || fs::remove_file(&tmp));
        res.map(|_| target)
    }

    fn publish(&self, rel: &str, tmp: &Path, target: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(tmp).map_err(|e| Error::io(&e, tmp))?;
        f.write_all(bytes).map_err(|e| Error::io(&e, tmp))?;
        f.sync_all().map_err(|e| Error::io(&e, tmp))?; // durable before it is published
        drop(f);
        if overwrite {
            return retry("publish", || fs::rename(tmp, target)).map_err(|e| Error::io(&e, target));
        }
        let mut k = 1;
        loop {
            match faults::os("publish").and_then(|_| fs::hard_link(tmp, target)) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(exists_error(rel)),
                Err(e) if e.raw_os_error() == Some(EACCES) => {
                    if k == 5 {
                        return Err(Error::io(&e, target));
                    }
                    backoff(k);
                    k += 1;
                }
                Err(_) => return exclusive_create(rel, target, bytes), // no hard links here: never retried
            }
        }
    }

    /// Append (03 §5.3); returns the resolved path.
    pub fn append(&self, rel: &str, content: &str, confirm: bool) -> Result<PathBuf> {
        let target = self.checked_target(rel, confirm)?;
        let dir = target.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir).map_err(|e| Error::io(&e, dir))?;
        let mut f = retry("append_open", || OpenOptions::new().append(true).create(true).open(&target))
            .map_err(|e| Error::io(&e, &target))?;
        write_once(&mut f, content.as_bytes(), "append_write").map_err(|e| Error::io(&e, &target))?;
        Ok(target)
    }
}

fn exclusive_create(rel: &str, target: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = match retry("exclusive_open", || OpenOptions::new().write(true).create_new(true).open(target)) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(exists_error(rel)),
        Err(e) => return Err(Error::io(&e, target)),
    };
    if let Err(e) = write_once(&mut f, bytes, "exclusive_write") {
        drop(f);
        let _ = fs::remove_file(target);
        return Err(Error::io(&e, target));
    }
    Ok(())
}

/// One write request, looping only to finish a short write; never retried.
fn write_once(f: &mut fs::File, mut buf: &[u8], op: &'static str) -> io::Result<()> {
    while !buf.is_empty() {
        let limit = match faults::hit(op) {
            Some(Fault::Os(code)) => return Err(io::Error::from_raw_os_error(code)),
            Some(Fault::Short(n)) => n.min(buf.len()),
            None => buf.len(),
        };
        match f.write(&buf[..limit]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Lenient text read of a walked note (00 §3.1).
pub fn read_lenient(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| Error::io(&e, path))?;
    Ok(normalize_newlines(&decode_lenient(&bytes)))
}

// ------------------------------------------------------------ frontmatter

/// Index of the closing "\n---" when the text has frontmatter (03 §6, Q4).
fn fence_end(text: &str) -> Option<usize> {
    if !text.starts_with("---") {
        return None;
    }
    text[3..].find("\n---").map(|i| i + 3)
}

/// parse(text): the frontmatter mapping as (key, value) pairs; {} when absent
/// or invalid; Err only for quirk Q5.
pub fn parse(text: &str) -> Result<Vec<(Y, Y)>> {
    let Some(end) = fence_end(text) else { return Ok(vec![]) };
    Ok(match parse_doc(&text[3..end])? {
        Some(Y::Map(pairs)) => pairs,
        _ => vec![],
    })
}

pub fn get<'a>(meta: &'a [(Y, Y)], key: &str) -> Option<&'a Y> {
    meta.iter().find(|(k, _)| matches!(k, Y::Str(s) if s == key)).map(|(_, v)| v)
}

pub fn body(text: &str) -> &str {
    match fence_end(text) {
        Some(end) => &text[end + 4..],
        None => text,
    }
}

pub fn str_list(v: Option<&Y>) -> Vec<String> {
    match v {
        None | Some(Y::Null) => vec![],
        Some(Y::Seq(items)) => items.iter().map(display).collect(),
        Some(other) => vec![display(other)],
    }
}

/// json_safe(v) (03 §6).
pub fn json_safe(v: &Y) -> Value {
    match v {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(*b),
        Y::Int(i) => Value::from(*i),
        Y::Float(f) if f.is_finite() => Value::from(*f),
        Y::Float(f) => crate::json::nonfinite(*f),
        Y::Str(s) => Value::String(s.clone()),
        Y::Seq(items) => Value::Array(items.iter().map(json_safe).collect()),
        Y::Map(pairs) => Value::Object(map_safe(pairs)),
        other => Value::String(display(other)),
    }
}

pub fn map_safe(pairs: &[(Y, Y)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(display(k), json_safe(v));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_collapses_and_keeps_missing_tail() {
        let d = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        assert_eq!(resolve_path(&root.join("a/../b/./c.md")), root.join("b/c.md"));
        assert_eq!(resolve_path(&root.join("x/../../y")), root.parent().unwrap().join("y"));
    }

    #[test]
    fn frontmatter_fences() {
        assert_eq!(body("---\ntags: [x]\n---\n\n# T\n\nBody.\n"), "\n\n# T\n\nBody.\n");
        assert_eq!(body("---\nunclosed\n"), "---\nunclosed\n");
        assert_eq!(parse("---\na: 1\n----\n").unwrap().len(), 1);
        assert!(parse("\u{feff}---\na: 1\n---\n").unwrap().is_empty());
        assert!(parse("---\n- a\n---\n").unwrap().is_empty());
    }
}
