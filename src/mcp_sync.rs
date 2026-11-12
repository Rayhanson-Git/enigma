//! mcp-sync: register missing MCP servers with the claude CLI (15 §6).
use crate::error::{Error, Kind};
use crate::extractor::{exit_code, json_to_y, which_on_path};
use crate::text::quoted;
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CLAUDE_MISSING: &str = "claude CLI not found on PATH. Run the printed command(s) manually, or install Claude Code.";

/// The built-in manifest (15 §6.1).
pub fn builtin_manifest() -> Value {
    json!({"servers": [
        {"name": "enigma", "transport": "stdio", "command": "{ENIGMA_BIN}", "args": [], "env": {"ENIGMA_VAULT_PATH": "{VAULT}"},
         "why": "The memory vault: persistent shared memory for agents."},
        {"name": "fetch", "transport": "stdio", "command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8"},
         "why": "Web ingest: turns a URL into clean markdown for /clip."},
        {"name": "arxiv", "transport": "stdio", "command": "uvx", "args": ["arxiv-mcp-server@0.5.0"], "env": {},
         "why": "Paper ingest: arXiv search and download as markdown. Paper content is untrusted input."},
    ]})
}

/// A manifest entry with placeholders resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub name: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub why: String,
}

pub struct Options {
    pub check: bool,
    pub vault: Option<String>,
    /// The manifest file in use; None means the built-in manifest.
    pub manifest: Option<PathBuf>,
    pub config: PathBuf,
    pub exe: PathBuf,
}

fn s(v: Option<&Value>) -> Option<String> {
    v.filter(|v| !v.is_null()).map(|v| v.as_str().map_or_else(|| crate::text::display(&json_to_y(v)), String::from))
}

fn truthy(v: &Option<String>) -> bool {
    v.as_ref().is_some_and(|s| !s.is_empty())
}

/// Resolves the manifest's entries (15 §6.1); Err is the abort message.
pub fn resolve(manifest: &Value, repo: &Path, vault: Option<&str>, exe: &Path) -> Result<Vec<Server>, String> {
    let sub = |text: String| -> Result<String, String> {
        if text.contains("{VAULT}") && vault.is_none() {
            return Err("manifest entry needs {VAULT}: pass --vault or set ENIGMA_VAULT_PATH".into());
        }
        Ok(text.replace("{ENIGMA_BIN}", &exe.display().to_string()).replace("{REPO}", &repo.display().to_string()).replace("{VAULT}", vault.unwrap_or_default()))
    };
    let mut out = Vec::new();
    for e in manifest.get("servers").and_then(Value::as_array).into_iter().flatten() {
        let name = s(e.get("name"));
        let (url, command) = (s(e.get("url")), s(e.get("command")));
        if !truthy(&url) && !truthy(&command) {
            let shown = name.as_deref().map_or("null".to_string(), crate::text::quote_str);
            return Err(format!("manifest entry {shown}: stdio transport requires a command (or provide a url)"));
        }
        let args: Vec<String> = e.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(|a| s(Some(a))).collect();
        let env: Vec<(String, String)> =
            e.get("env").and_then(Value::as_object).into_iter().flatten().map(|(k, v)| (k.clone(), s(Some(v)).unwrap_or_default())).collect();
        out.push(Server {
            name: name.unwrap_or_else(|| "null".into()),
            url: url.filter(|u| !u.is_empty()).map(sub).transpose()?,
            command: command.map(sub).transpose()?,
            args: args.into_iter().map(&sub).collect::<Result<_, _>>()?,
            env: env.into_iter().map(|(k, v)| Ok((k, sub(v)?))).collect::<Result<_, String>>()?,
            why: s(e.get("why")).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Why mcp-sync stops before registering: an ABORT line (exit 2) or an error (exit 1).
pub enum Stop {
    Abort(String),
    Fail(Error),
}

impl From<String> for Stop {
    fn from(msg: String) -> Stop {
        Stop::Abort(msg)
    }
}

/// Reads C (15 §6.2): its mcpServers object.
pub fn read_config(path: &Path) -> Result<Map<String, Value>, Stop> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(Stop::Fail(Error::io(&e, path))),
    };
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    match v.as_object().map(|o| o.get("mcpServers").cloned().unwrap_or_else(|| json!({}))) {
        Some(Value::Object(m)) => Ok(m),
        _ => Err(format!("unexpected structure in {}: mcpServers must be an object", path.display()).into()),
    }
}

/// Reads a --manifest file (15 §6.1); every problem is an error (exit 1), not an ABORT.
fn read_manifest(p: &Path) -> Result<Value, Stop> {
    let sync = |msg: String| Stop::Fail(Error::new(Kind::SyncError, msg));
    let bytes = std::fs::read(p).map_err(|e| Stop::Fail(Error::io(&e, p)))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| sync(format!("cannot parse {}: {e}", p.display())))?;
    if !v.get("servers").is_some_and(Value::is_array) {
        return Err(sync(format!("{}: the manifest has no \"servers\" list", p.display())));
    }
    Ok(v)
}

fn shown(v: Option<&Value>) -> String {
    v.map_or("null".into(), |v| quoted(&json_to_y(v)))
}

/// The first difference between a config entry and a resolved server (15 §6.2).
pub fn drift(config: &Value, m: &Server) -> Option<String> {
    let diff = |field: &str, have: Option<&Value>, want: Value| (have != Some(&want)).then(|| format!("{field}: config={} manifest={}", shown(have), shown(Some(&want))));
    if let Some(url) = &m.url {
        return diff("url", config.get("url"), json!(url));
    }
    if let Some(d) = diff("command", config.get("command"), json!(m.command)) {
        return Some(d);
    }
    let args = config.get("args").cloned().unwrap_or_else(|| json!([]));
    if args != json!(m.args) {
        return Some(format!("args: config={} manifest={}", shown(Some(&args)), shown(Some(&json!(m.args)))));
    }
    m.env.iter().find_map(|(k, v)| diff(&format!("env[{k}]"), config.get("env").and_then(|e| e.get(k)), json!(v)))
}

/// The claude argv after the program name.
pub fn add_args(m: &Server) -> Vec<String> {
    let mut a: Vec<String> = ["mcp", "add", "--scope", "user"].map(String::from).into();
    if let Some(url) = &m.url {
        a.extend(["--transport".into(), "http".into(), m.name.clone(), url.clone()]);
        return a;
    }
    a.push(m.name.clone());
    for (k, v) in &m.env {
        a.extend(["--env".into(), format!("{k}={v}")]);
    }
    a.push("--".into());
    a.extend(m.command.clone());
    a.extend(m.args.iter().cloned());
    a
}

/// POSIX shell quoting of one word (shlex.quote rules).
/// One word under the Microsoft C runtime command-line rules (15 §6.3, Windows).
pub fn quote_word_windows(w: &str) -> String {
    if !w.is_empty() && !w.contains([' ', '\t', '"']) {
        return w.into();
    }
    let (mut out, mut slashes) = (String::from('"'), 0);
    for c in w.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                // backslashes before a quote are doubled, then the quote is escaped
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2)); // so the closing quote stays a quote
    out.push('"');
    out
}

pub fn quote_word(w: &str) -> String {
    if cfg!(windows) { quote_word_windows(w) } else { quote_word_posix(w) }
}

/// One word under the POSIX shell rules (15 §6.3).
pub fn quote_word_posix(w: &str) -> String {
    if w.is_empty() {
        return "''".into();
    }
    if w.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./_-".contains(c)) {
        return w.into();
    }
    format!("'{}'", w.replace('\'', "'\"'\"'"))
}

pub fn printed(args: &[String]) -> String {
    std::iter::once("claude".to_string()).chain(args.iter().map(|a| quote_word(a))).collect::<Vec<_>>().join(" ")
}

/// Runs mcp-sync, writing its report to out; returns the exit code.
pub fn run(o: &Options, out: &mut dyn Write) -> i32 {
    let say = |out: &mut dyn Write, line: &str| {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    };
    let prepared = (|| {
        let config = read_config(&o.config)?;
        let (manifest, repo) = match &o.manifest {
            Some(p) => {
                let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
                (read_manifest(p)?, abs.parent().map(Path::to_path_buf).unwrap_or_default())
            }
            None => (builtin_manifest(), std::env::current_dir().unwrap_or_default()),
        };
        Ok::<_, Stop>((config, resolve(&manifest, &repo, o.vault.as_deref(), &o.exe)?))
    })();
    let (config, servers) = match prepared {
        Ok(p) => p,
        Err(Stop::Abort(msg)) => {
            say(out, &format!("ABORT (nothing changed): {msg}"));
            return 2;
        }
        Err(Stop::Fail(e)) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let (mut present, mut drifted, mut missing) = (vec![], vec![], vec![]);
    for m in &servers {
        match config.get(&m.name) {
            None => missing.push(m),
            Some(c) => match drift(c, m) {
                Some(d) => drifted.push((m, d)),
                None => present.push(m),
            },
        }
    }
    let mut extras: Vec<&String> = config.keys().filter(|k| !servers.iter().any(|m| &m.name == *k)).collect();
    extras.sort();
    for m in &present {
        say(out, &format!("present : {}", m.name));
    }
    for (m, d) in &drifted {
        say(out, &format!("DRIFTED : {} \u{2014} {d}", m.name));
        say(out, &format!("  to fix drift manually: claude mcp remove --scope user {} && re-run this sync", m.name));
    }
    for name in extras {
        say(out, &format!("extra   : {name} (not in manifest; left alone)"));
    }
    for m in &missing {
        say(out, &format!("MISSING : {} \u{2014} {}", m.name, m.why));
    }
    if o.check {
        return i32::from(!missing.is_empty() || !drifted.is_empty());
    }
    if missing.is_empty() {
        return 0;
    }
    let pending: Vec<Vec<String>> = missing.iter().map(|m| add_args(m)).collect();
    let Some(claude) = which_on_path("claude") else {
        for p in &pending {
            say(out, &format!("register: {}", printed(p)));
        }
        say(out, &format!("{CLAUDE_MISSING} Nothing was registered."));
        return 3;
    };
    let mut failed = false;
    for (i, p) in pending.iter().enumerate() {
        say(out, &format!("register: {}", printed(p)));
        match std::process::Command::new(&claude).args(p).status() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                for rest in &pending[i + 1..] {
                    say(out, &format!("register: {}", printed(rest)));
                }
                say(out, CLAUDE_MISSING);
                return 3;
            }
            Err(e) => {
                // any other start failure ends the run at once (15 §6.3)
                eprintln!("error: {}", Error::io(&e, &claude));
                return 1;
            }
            Ok(st) if !st.success() => {
                let code = exit_code(st);
                say(out, &format!("  FAILED (exit {code}) \u{2014} continuing with the rest"));
                failed = true;
            }
            Ok(_) => {}
        }
    }
    i32::from(failed)
}
