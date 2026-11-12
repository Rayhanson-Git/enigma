//! Shared fixtures: the mini vault (MV), fake embedders, a stdio MCP client.
#![allow(dead_code)]
use chrono::{NaiveDate, NaiveDateTime};
use enigma_mcp::embed::Embedder;
use enigma_mcp::error::Result;
use enigma_mcp::vault::Vault;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

pub const LIGHTHOUSE: &str = "---\ntags: [project, grid]\n---\n\n# Lighthouse Grid\n\nGrid outage ingestion pipeline with a queue-free architecture.\n";

pub fn write_file(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// The mini vault of acceptance/README.md.
pub fn mini_vault() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write_file(r, ".obsidian/app.json", "{}");
    write_file(r, "Projects/Lighthouse Grid.md", LIGHTHOUSE);
    write_file(r, "Daily.md", "# Daily\n\nRemember to check the pipeline.\n");
    fs::create_dir_all(r.join("Claude/Sessions")).unwrap();
    fs::create_dir_all(r.join("Claude/Inbox")).unwrap();
    write_file(r, "Claude/Index.md", "# Index\n\n");
    write_file(r, "Claude/Concepts/Postgres.md", "---\ntags: [concept]\n---\n\n# Postgres\n\nRelational database used by the sync relay.\n");
    d
}

/// A vault holding only the given notes.
pub fn vault_with(notes: &[(&str, &str)]) -> TempDir {
    let d = tempfile::tempdir().unwrap();
    for (rel, text) in notes {
        write_file(d.path(), rel, text);
    }
    d
}

pub fn open(d: &TempDir) -> Vault {
    Vault::open(d.path()).unwrap()
}

/// NOW = 2026-07-05 14:30 local.
pub fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 5).unwrap().and_hms_opt(14, 30, 0).unwrap()
}

pub fn read(d: &TempDir, rel: &str) -> String {
    fs::read_to_string(d.path().join(rel)).unwrap()
}

/// Every *.tmp-write file below a folder.
pub fn temp_writes(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp-write"))
        .map(|e| e.into_path())
        .collect()
}

pub fn copy_dir(from: &Path, to: &Path) {
    for e in walkdir::WalkDir::new(from).into_iter().flatten() {
        let dest = to.join(e.path().strip_prefix(from).unwrap());
        if e.file_type().is_dir() {
            fs::create_dir_all(&dest).unwrap();
        } else {
            fs::copy(e.path(), &dest).unwrap();
        }
    }
}

pub fn fixture_vault() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/fixtures/vault"), d.path());
    d
}

/// The fake embedder of acceptance/README.md: [len(text), 0.0], counting calls.
#[derive(Default)]
pub struct LenEmb {
    pub texts: AtomicUsize,
}

impl Embedder for LenEmb {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f64>>> {
        self.texts.fetch_add(texts.len(), Ordering::SeqCst);
        Ok(texts.iter().map(|t| vec![t.chars().count() as f64, 0.0]).collect())
    }
}

/// Embedder from a closure.
pub struct FnEmb(pub fn(&str) -> Vec<f64>);

impl Embedder for FnEmb {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f64>>> {
        Ok(texts.iter().map(|t| (self.0)(t)).collect())
    }
}

/// Keyword fake: [has "freight", has "baking", has "budget"].
pub fn keyword_vec(t: &str) -> Vec<f64> {
    ["freight", "baking", "budget"].iter().map(|w| if t.contains(w) { 1.0 } else { 0.0 }).collect()
}

/// Semantic fake: "owe" or "invoice" -> [1, 0], else [0, 1].
pub fn semantic_vec(t: &str) -> Vec<f64> {
    if t.contains("owe") || t.contains("invoice") { vec![1.0, 0.0] } else { vec![0.0, 1.0] }
}

// ---------------------------------------------------------- stdio client

pub struct Mcp {
    pub child: Child,
    stdin: Option<ChildStdin>,
    out: BufReader<ChildStdout>,
    next: i64,
    early: std::collections::HashMap<i64, Value>,
}

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_enigma-mcp")
}

/// A server process with a clean ENIGMA_* environment plus the given pairs.
pub fn spawn(env: &[(&str, &str)]) -> Child {
    let mut cmd = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            cmd.env_remove(k);
        }
    }
    cmd.envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.spawn().unwrap()
}

impl Mcp {
    /// Spawns the server and completes the initialize handshake.
    pub fn start(env: &[(&str, &str)]) -> (Mcp, Value) {
        let mut child = spawn(env);
        let stdin = child.stdin.take();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut m = Mcp { child, stdin, out, next: 1, early: Default::default() };
        let init = m.request(
            "initialize",
            json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "probe", "version": "0"}}),
        );
        m.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        (m, init)
    }

    pub fn vault(vault: &Path, state: &Path, extra: &[(&str, &str)]) -> Mcp {
        let mut env = vec![("ENIGMA_VAULT_PATH", vault.to_str().unwrap()), ("ENIGMA_STATE_DIR", state.to_str().unwrap())];
        env.extend_from_slice(extra);
        Mcp::start(&env).0
    }

    fn send(&mut self, msg: Value) {
        let s = self.stdin.as_mut().unwrap();
        writeln!(s, "{msg}").unwrap();
        s.flush().unwrap();
    }

    /// Sends a request and returns its whole response, skipping other messages.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.submit(method, params);
        self.response(id)
    }

    /// Sends a request without waiting; returns its id.
    pub fn submit(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Waits for the response to request id (buffering others).
    pub fn response(&mut self, id: i64) -> Value {
        if let Some(v) = self.early.remove(&id) {
            return v;
        }
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "server closed stdout");
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            match v["id"].as_i64() {
                Some(i) if i == id => return v,
                Some(i) => {
                    self.early.insert(i, v);
                }
                None => {}
            }
        }
    }

    /// tools/call; returns the result object.
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert!(r.get("error").is_none(), "protocol error: {r}");
        r["result"].clone()
    }

    pub fn tool_names(&mut self) -> Vec<String> {
        let r = self.request("tools/list", json!({}));
        r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    /// Closes stdin and waits for exit.
    pub fn close(mut self) -> ExitStatus {
        drop(self.stdin.take());
        self.child.wait().unwrap()
    }
}

/// Text of each content block.
pub fn texts(result: &Value) -> Vec<String> {
    result["content"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap().to_string()).collect()
}

pub fn is_error(result: &Value) -> bool {
    result["isError"] == json!(true)
}

/// The JSON objects of a list result.
pub fn objects(result: &Value) -> Vec<Value> {
    texts(result).iter().map(|t| serde_json::from_str(t).unwrap()).collect()
}

/// Marks a file as a program. POSIX needs the mode bits; on Windows the extension decides.
pub fn make_executable(p: &Path) {
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(p).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(p, perms).unwrap();
    }
    #[cfg(not(unix))]
    let _ = p;
}

/// Puts a stand-in program called `name` in `dir`: the POSIX shell script `sh`,
/// or on Windows `name.cmd` running the PowerShell script `ps`.
pub fn stub_program(dir: &Path, name: &str, sh: &str, ps: &str) -> PathBuf {
    if cfg!(windows) {
        fs::write(dir.join(format!("{name}.ps1")), ps).unwrap();
        let cmd = dir.join(format!("{name}.cmd"));
        fs::write(&cmd, format!("@powershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{name}.ps1\" %*\r\n")).unwrap();
        cmd
    } else {
        let script = dir.join(name);
        fs::write(&script, sh).unwrap();
        make_executable(&script);
        script
    }
}

/// A symbolic link at `link` pointing to `target` (relative targets are relative to the link's folder).
pub fn symlink(target: impl AsRef<Path>, link: &Path) {
    let target = target.as_ref();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    // shortcut: needs Developer Mode or an elevated shell on Windows (CI runners have it)
    #[cfg(windows)]
    if link.parent().unwrap().join(target).is_dir() {
        std::os::windows::fs::symlink_dir(target, link).unwrap();
    } else {
        std::os::windows::fs::symlink_file(target, link).unwrap();
    }
}

/// A number that changes when a file is replaced by a new one.
pub fn file_identity(p: &Path) -> u64 {
    let m = fs::metadata(p).unwrap();
    #[cfg(unix)]
    return std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(windows)]
    return std::os::windows::fs::MetadataExt::creation_time(&m);
}

/// A directory holding a fake "codex" CLI: Orbital Freight for notes whose
/// prompt mentions Lighthouse, empty extractions otherwise, and no merges for
/// consolidation prompts. Never a real LLM CLI.
pub fn fake_codex_dir() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    stub_program(
        d.path(),
        "codex",
        "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in\n  *\"duplicate entries\"*) echo '{\"merges\": []}' ;;\n  *\"Note path: \"*Lighthouse*) echo '{\"entities\": [{\"name\": \"Orbital Freight\", \"type\": \"organization\", \"aliases\": [], \"summary\": \"A company.\"}], \"relations\": []}' ;;\n  *) echo '{\"entities\": [], \"relations\": []}' ;;\nesac\n",
        "$text = [Console]::In.ReadToEnd()\nif ($text -like '*duplicate entries*') { '{\"merges\": []}' }\nelseif ($text -match '(?s)Note path: .*Lighthouse') { '{\"entities\": [{\"name\": \"Orbital Freight\", \"type\": \"organization\", \"aliases\": [], \"summary\": \"A company.\"}], \"relations\": []}' }\nelse { '{\"entities\": [], \"relations\": []}' }\n",
    );
    d
}

/// The separator between PATH entries.
pub const PATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

/// PATH with the fake CLI directory first.
pub fn path_with(dir: &Path) -> String {
    format!("{}{PATH_SEP}{}", dir.display(), std::env::var("PATH").unwrap_or_default())
}

/// A PATH holding one folder plus only the system's own tools.
pub fn sandbox_path(dir: &Path) -> String {
    if cfg!(windows) {
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        format!(r"{};{sys}\System32;{sys}\System32\WindowsPowerShell\v1.0", dir.display())
    } else {
        format!("{}:/usr/bin:/bin", dir.display())
    }
}

/// The fenced ```text block after `marker` in a spec file, without its final LF.
pub fn spec_block(file: &str, marker: &str) -> String {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("spec").join(file)).unwrap();
    let from = text.find(marker).unwrap_or_else(|| panic!("{marker} not in {file}"));
    let start = from + text[from..].find("```text\n").unwrap() + "```text\n".len();
    let end = start + text[start..].find("\n```").unwrap();
    text[start..end].to_string()
}
