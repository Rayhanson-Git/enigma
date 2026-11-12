//! Obsidian provisioning: pinned plugins, overlays, conventions (15 §7).
use crate::error::{Error, Kind, Result};
use crate::extractor::json_to_y;
use crate::text::display;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// The plugin lock (15 §7.1), generated from the spec table.
pub const LOCK_JSON: &str = include_str!("plugins.json");
pub const ASSETS: [&str; 3] = ["manifest.json", "main.js", "styles.css"];
pub const SMART_ENV: &str = "{\n  \"is_obsidian_vault\": true,\n  \"smart_sources\": {\n    \"min_chars\": 200,\n    \"embed_model\": {\n      \"adapter\": \"transformers\",\n      \"transformers\": {\n        \"model_key\": \"TaylorAI/bge-micro-v2\"\n      }\n    }\n  },\n  \"models\": {\n    \"embedding_platform\": \"transformers\"\n  }\n}\n";
pub const WARNING: &str = "WARNING: some plugins failed \u{2014} re-run after checking pins/network.";
pub const NEXT_STEPS: &str = "Provisioning done. Remaining human steps:\n 1. Open the vault in Obsidian once and turn OFF Restricted Mode\n    (Settings -> Community plugins) so the installed plugins load.\n 2. Complete LiveSync setup via its Setup-URI flow \u{2014} server URI and the\n    E2E passphrase live in your password manager, never in this repo.\n 3. Register the vault with the MCP server (enigma-mcp mcp-sync) and run the\n    index_brain tool to build the semantic graph.\n";

fn err(msg: String) -> Error {
    Error::new(Kind::ProvisionError, msg)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub id: String,
    pub repo: String,
    pub version: String,
    /// (asset file name, sha256 hex) in lock order.
    pub sha256: Vec<(String, String)>,
}

/// Validates one lock entry (15 §7.1).
pub fn validate(entry: &Value) -> Result<Plugin> {
    let shown = || display(&json_to_y(entry));
    let field = |f: &str| -> Result<String> {
        match entry.get(f).filter(|v| !v.is_null()).map(|v| display(&json_to_y(v))) {
            Some(s) if !s.is_empty() => Ok(s),
            _ => Err(err(format!("plugins.json entry missing '{f}': {}", shown()))),
        }
    };
    let (id, repo, version) = (field("id")?, field("repo")?, field("version")?);
    let Some(lock) = entry.get("sha256").and_then(Value::as_object) else {
        return Err(err(format!("plugins.json entry missing 'sha256' lock: {}", shown())));
    };
    for needed in ["manifest.json", "main.js"] {
        if !lock.contains_key(needed) {
            return Err(err(format!("{id}: sha256 lock missing '{needed}'")));
        }
    }
    let mut sha256 = Vec::new();
    for (name, hex) in lock {
        if !ASSETS.contains(&name.as_str()) {
            return Err(err(format!("{id}: sha256 lock names unknown asset '{name}'")));
        }
        match hex.as_str() {
            Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => sha256.push((name.clone(), h.to_string())),
            _ => return Err(err(format!("{id}: sha256 for '{name}' is not 64 lowercase hex chars"))),
        }
    }
    Ok(Plugin { id, repo, version, sha256 })
}

/// Validates a lock list.
pub fn parse_lock(text: &str) -> Result<Vec<Plugin>> {
    let v: Value = serde_json::from_str(text).map_err(|e| err(format!("plugins.json: {e}")))?;
    v.as_array().ok_or_else(|| err("plugins.json must be a list".into()))?.iter().map(validate).collect()
}

pub fn bundled_lock() -> Vec<Plugin> {
    parse_lock(LOCK_JSON).expect("the bundled lock is valid")
}

// ---------------------------------------------------------------- fetching (§7.2)

/// fetch(url) → bytes, or None when absent (HTTP 404).
pub trait Fetch {
    fn fetch(&self, url: &str) -> Result<Option<Vec<u8>>>;
}

/// The real fetcher: GitHub release downloads over HTTPS.
pub struct Http;

impl Fetch for Http {
    fn fetch(&self, url: &str) -> Result<Option<Vec<u8>>> {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(30))).build().into();
        match agent.get(url).header("User-Agent", "enigma-mcp-provisioner").call() {
            Ok(mut r) => {
                let body = r.body_mut().with_config().limit(u64::MAX).read_to_vec();
                body.map(Some).map_err(|e| err(format!("download failed {url}: {e}")))
            }
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(ureq::Error::StatusCode(code)) => Err(err(format!("download failed {url}: HTTP {code}"))),
            Err(e) => Err(err(format!("download failed {url}: {e}"))),
        }
    }
}

pub fn asset_url(p: &Plugin, file: &str) -> String {
    format!("https://github.com/{}/releases/download/{}/{file}", p.repo, p.version)
}

fn plugin_dir(vault: &Path, id: &str) -> std::path::PathBuf {
    vault.join(".obsidian/plugins").join(id)
}

/// `version` of the installed manifest.json; None when absent or unreadable.
pub fn installed_version(vault: &Path, id: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(&fs::read(plugin_dir(vault, id).join("manifest.json")).ok()?).ok()?;
    v.get("version").filter(|v| !v.is_null()).map(|v| display(&json_to_y(v)))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// install_plugin → "installed" or "ok" (15 §7.2). A refusal writes nothing.
pub fn install_plugin(vault: &Path, p: &Plugin, fetch: &dyn Fetch) -> Result<&'static str> {
    if installed_version(vault, &p.id).as_deref() == Some(p.version.as_str()) {
        return Ok("ok");
    }
    let mut got = Vec::new();
    for (file, want) in &p.sha256 {
        let bytes = fetch.fetch(&asset_url(p, file))?.ok_or_else(|| err(format!("{}: no {file} in release {} of {}", p.id, p.version, p.repo)))?;
        let actual = hex(&bytes);
        if &actual != want {
            return Err(err(format!("{}: {file} sha256 {actual} does not match lock {want} \u{2014} refusing", p.id)));
        }
        got.push((file, bytes));
    }
    let manifest = got.iter().find(|(f, _)| f.as_str() == "manifest.json").map(|(_, b)| b.as_slice()).unwrap_or_default();
    let declared = serde_json::from_slice::<Value>(manifest).ok().and_then(|m| m.get("id").cloned()).unwrap_or(Value::Null);
    if declared.as_str() != Some(p.id.as_str()) {
        return Err(err(format!("{}: release manifest declares id '{}' \u{2014} refusing", p.id, display(&json_to_y(&declared)))));
    }
    let dir = plugin_dir(vault, &p.id);
    fs::create_dir_all(&dir).map_err(|e| Error::io(&e, &dir))?;
    for (file, bytes) in got {
        let path = dir.join(file);
        fs::write(&path, bytes).map_err(|e| Error::io(&e, &path))?;
    }
    Ok("installed")
}

/// enable_plugins(ids) → the ids added to community-plugins.json.
pub fn enable_plugins(vault: &Path, ids: &[String]) -> Result<Vec<String>> {
    let path = vault.join(".obsidian/community-plugins.json");
    let existing: Vec<Value> = match fs::read(&path) {
        Ok(b) => serde_json::from_slice::<Value>(&b).map_err(|e| err(format!("{}: {e}", path.display())))?.as_array().cloned().unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(Error::io(&e, &path)),
    };
    let added: Vec<String> = ids.iter().filter(|id| !existing.contains(&json!(id))).cloned().collect();
    if !added.is_empty() {
        let all: Vec<Value> = existing.into_iter().chain(added.iter().map(|a| json!(a))).collect();
        fs::create_dir_all(vault.join(".obsidian")).map_err(|e| Error::io(&e, vault))?;
        fs::write(&path, crate::json::indented(&all)).map_err(|e| Error::io(&e, &path))?;
    }
    Ok(added)
}

/// Overlays (15 §7.4): none per plugin today, then the Smart Connections settings.
pub fn overlays(vault: &Path) -> Result<Vec<String>> {
    let path = vault.join(".smart-env/smart_env.json");
    if path.symlink_metadata().is_ok() {
        return Ok(vec![]);
    }
    let dir = vault.join(".smart-env");
    fs::create_dir_all(&dir).map_err(|e| Error::io(&e, &dir))?;
    fs::write(&path, SMART_ENV).map_err(|e| Error::io(&e, &path))?;
    Ok(vec![".smart-env/smart_env.json".into()])
}

/// provision(vault, fetch) (15 §7.3) over a lock (the CLI passes the bundled one).
pub fn provision(vault: &Path, lock: &[Plugin], fetch: &dyn Fetch) -> Result<Value> {
    if !vault.is_dir() {
        return Err(err(format!("Vault root does not exist: {}", vault.display())));
    }
    let obs = vault.join(".obsidian");
    fs::create_dir_all(&obs).map_err(|e| Error::io(&e, &obs))?;
    let (mut plugins, mut errors, mut good) = (Map::new(), Map::new(), Vec::new());
    for p in lock {
        match install_plugin(vault, p, fetch) {
            Ok(status) => {
                plugins.insert(p.id.clone(), json!(status));
                good.push(p.id.clone());
            }
            Err(e) if e.kind == Kind::ProvisionError => {
                errors.insert(p.id.clone(), json!(e.msg));
            }
            Err(e) => return Err(e),
        }
    }
    let enabled = enable_plugins(vault, &good)?;
    let overlays = overlays(vault)?;
    let conventions = crate::conventions::install(vault, None)?;
    Ok(json!({"plugins": plugins, "errors": errors, "enabled": enabled, "overlays": overlays, "conventions": conventions}))
}

/// check(vault) → {id: {pinned, installed, status}} (15 §7.3); no vault check, a missing vault reports every plugin missing.
pub fn check(vault: &Path, lock: &[Plugin]) -> Result<Value> {
    let mut out = Map::new();
    for p in lock {
        let installed = installed_version(vault, &p.id);
        let status = match &installed {
            None => "missing",
            Some(v) if *v == p.version => "ok",
            Some(_) => "drift",
        };
        out.insert(p.id.clone(), json!({"pinned": p.version, "installed": installed, "status": status}));
    }
    Ok(Value::Object(out))
}

/// The CLI text after the report (15 §7.5).
pub fn epilogue(report: &Value) -> String {
    let warn = if report["errors"].as_object().is_some_and(|e| !e.is_empty()) { format!("\n{WARNING}\n") } else { String::new() };
    format!("{warn}\n{NEXT_STEPS}\n")
}
