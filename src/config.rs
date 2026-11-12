//! Environment configuration (01): vault root, state dir, agent name, profiles.
use crate::error::{Error, Result};
use crate::text::strip;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// An ENIGMA_* variable; empty counts as unset.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// True only for an explicit off value; unset, empty and anything else are not.
pub fn falsy(name: &str) -> bool {
    var(name).is_some_and(|v| matches!(strip(&v).to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
}

pub fn truthy(name: &str) -> bool {
    var(name).is_some_and(|v| matches!(strip(&v).to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Resolves a vault folder (01 §3, 03 §1).
pub fn open_root(p: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(p).map_err(|e| Error::io(&e, p))?;
    match std::fs::canonicalize(&abs) {
        Ok(c) if c.is_dir() => Ok(plain(c)),
        Ok(c) => Err(Error::vault(format!("Vault root does not exist: {}", plain(c).display()))),
        Err(_) => Err(Error::vault(format!("Vault root does not exist: {}", crate::vault::resolve_path(&abs).display()))),
    }
}

/// A resolved path as it is shown: on Windows without the `\\?\` verbatim prefix (01 §3).
pub fn plain(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

static ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The server's vault root: read from ENIGMA_VAULT_PATH, cached on success only.
pub fn vault_root() -> Result<PathBuf> {
    let mut cached = ROOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = cached.as_ref() {
        return Ok(r.clone());
    }
    let p = var("ENIGMA_VAULT_PATH").ok_or_else(|| Error::vault("ENIGMA_VAULT_PATH is not set; point it at the vault folder."))?;
    let root = open_root(Path::new(&p))?;
    *cached = Some(root.clone());
    Ok(root)
}

pub fn home() -> PathBuf {
    var("HOME").map(PathBuf::from).or_else(std::env::home_dir).unwrap_or_default()
}

/// The state dir derived from HOME and the vault (01 §4 step 2), not created.
pub fn derived_state_dir(root: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(root).map_err(|e| Error::io(&e, root))?;
    let canon = std::fs::canonicalize(&abs).map(plain).unwrap_or_else(|_| crate::vault::resolve_path(&abs));
    let digest = Sha256::digest(canon.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    Ok(home().join(".enigma-mcp").join(hex))
}

/// The state dir (01 §4), created before it is returned.
pub fn state_dir(vault_root: Option<&Path>) -> Result<PathBuf> {
    let dir = match var("ENIGMA_STATE_DIR") {
        Some(d) => PathBuf::from(d),
        None => {
            let root = match vault_root {
                Some(r) => r.to_path_buf(),
                None => PathBuf::from(var("ENIGMA_VAULT_PATH").ok_or_else(|| {
                    Error::vault("Cannot determine state directory: pass vault_root or set ENIGMA_VAULT_PATH.")
                })?),
            };
            derived_state_dir(&root)?
        }
    };
    let fresh = !dir.exists();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&e, &dir))?;
    if fresh {
        owner_only(&dir, 0o700).map_err(|e| Error::io(&e, &dir))?; // the state holds private note vectors
    }
    Ok(dir)
}

/// Limits a file or folder to its owner (no-op where the system has no such modes).
pub fn owner_only(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// The agent name (01 §5) for a raw ENIGMA_AGENT_NAME value.
pub fn agent_name_from(raw: Option<&str>) -> String {
    match raw.map(strip) {
        Some(s) if !s.is_empty() => crate::notes::safe_filename(s),
        _ => "claude".into(),
    }
}

pub fn agent_name() -> String {
    agent_name_from(var("ENIGMA_AGENT_NAME").as_deref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Full,
    ReadOnly,
}

/// The tool profile (01 §6); ENIGMA_TOOL_PROFILE is ignored.
pub fn profile() -> Profile {
    if truthy("ENIGMA_READONLY") { Profile::ReadOnly } else { Profile::Full }
}
