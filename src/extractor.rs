//! The LLM extractor (06 §5): CLI backends, invocation, reply parsing.
use crate::error::{Error, Kind, Result};
use crate::graph::{Entity, Extraction, Relation};
use crate::text::{display, split_lines, strip};
use crate::yaml::Y;
use regex::Regex;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

pub const ENTITY_TYPES: [&str; 6] = ["person", "organization", "domain", "topic", "project", "source"];
pub const RELATIONS: [&str; 6] = ["mentions", "works_at", "part_of", "operates_in", "about", "related_to"];
const TIMEOUT_SECS: u64 = 120;

/// Turns one note into an extraction; tests substitute fakes.
pub trait Extract: Send + Sync {
    fn extract(&self, path: &str, content: &str) -> Result<Extraction>;
}

/// Sends a prompt and returns the reply's JSON object; tests substitute fakes.
pub trait Complete: Send + Sync {
    fn complete_json(&self, prompt: &str) -> Result<Value>;
}

/// What running a command produced.
pub enum RunOutcome {
    Exit { code: i32, stdout: String, stderr: String },
    Timeout,
    SpawnError(String),
}

pub type Runner = Box<dyn Fn(&[String], &str) -> RunOutcome + Send + Sync>;
pub type Which = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

fn err(msg: impl Into<String>) -> Error {
    Error::new(Kind::ExtractorError, msg)
}

/// The extraction prompt (06 §5.3), byte for byte.
pub const PROMPT: &str = include_str!("prompts/extraction.txt");

/// PROMPT with {path} and {content} each filled once, at its own place.
pub fn prompt(path: &str, content: &str) -> String {
    let (head, rest) = PROMPT.split_once("{path}").expect("template has {path}");
    let (mid, tail) = rest.split_once("{content}").expect("template has {content}");
    [head, path, mid, content, tail].concat()
}

/// JSON value as a YAML-model value, so display() applies.
pub fn json_to_y(v: &Value) -> Y {
    match v {
        Value::Null => Y::Null,
        Value::Bool(b) => Y::Bool(*b),
        Value::Number(n) => n.as_i64().map(Y::Int).unwrap_or_else(|| Y::Float(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => Y::Str(s.clone()),
        Value::Array(a) => Y::Seq(a.iter().map(json_to_y).collect()),
        Value::Object(o) => Y::Map(o.iter().map(|(k, v)| (Y::Str(k.clone()), json_to_y(v))).collect()),
    }
}

fn falsy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => true,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        _ => false,
    }
}

/// display() of a field, trimmed; falsy → empty.
fn field(o: &serde_json::Map<String, Value>, k: &str) -> String {
    let v = o.get(k);
    if falsy(v) { String::new() } else { strip(&display(&json_to_y(v.unwrap_or(&Value::Null)))).to_string() }
}

fn known(v: String, set: &[&str], fallback: &str) -> String {
    let v = v.to_lowercase();
    if set.contains(&v.as_str()) { v } else { fallback.into() }
}

fn objects<'a>(reply: &'a Value, key: &str) -> Vec<&'a serde_json::Map<String, Value>> {
    reply.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_object).collect()).unwrap_or_default()
}

/// Coercion of a reply object (06 §5.2).
pub fn coerce(reply: &Value) -> Extraction {
    let mut x = Extraction::default();
    for o in objects(reply, "entities") {
        let name = field(o, "name");
        if name.is_empty() {
            continue;
        }
        let aliases: Vec<String> = match o.get("aliases") {
            Some(Value::String(s)) => s.chars().map(String::from).collect(), // quirk: a string is iterated per character
            Some(Value::Array(a)) => a.iter().map(|v| strip(&display(&json_to_y(v))).to_string()).collect(),
            _ => vec![],
        };
        x.entities.push(Entity {
            name,
            ty: known(field(o, "type"), &ENTITY_TYPES, "topic"),
            aliases: aliases.into_iter().map(|a| strip(&a).to_string()).filter(|a| !a.is_empty()).collect(),
            summary: field(o, "summary"),
        });
    }
    for o in objects(reply, "relations") {
        let (from, to) = (field(o, "from"), field(o, "to"));
        if from.is_empty() || to.is_empty() {
            continue;
        }
        x.relations.push(Relation {
            from,
            from_type: known(field(o, "from_type"), &ENTITY_TYPES, "topic"),
            rel: known(field(o, "rel"), &RELATIONS, "related_to"),
            to,
            to_type: known(field(o, "to_type"), &ENTITY_TYPES, "topic"),
            evidence: field(o, "evidence"),
        });
    }
    x
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\x60\x60\x60(?:json)?\s*(\{.*?\})\s*\x60\x60\x60").expect("static regex"));
static LOUD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(ERROR|FATAL)\b").expect("static regex"));

/// Reply parsing (06 §5.6): Ok(object), or Err(Ok(decode detail)) / Err(Err(ExtractorError)).
pub fn parse_reply(text: &str) -> std::result::Result<Value, std::result::Result<String, Error>> {
    if let Some(c) = FENCE.captures(text) {
        return serde_json::from_str::<Value>(&c[1]).map_err(|e| Ok(e.to_string()));
    }
    for (i, _) in text.match_indices('{') {
        if let Some(Ok(v)) = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>().next()
            && v.is_object()
        {
            return Ok(v);
        }
    }
    Err(Err(err("no JSON object in extractor output")))
}

pub fn stderr_summary(stderr: &str) -> String {
    let t = strip(stderr);
    if t.is_empty() {
        return String::new();
    }
    if let Some(l) = split_lines(t).into_iter().rev().find(|l| LOUD.is_match(l)) {
        return strip(l).chars().take(300).collect();
    }
    let n = t.chars().count();
    t.chars().skip(n.saturating_sub(300)).collect()
}

/// A CLI-backed extractor (06 §5.4–§5.7).
pub struct Cli {
    backend: String,
    model: String,
    which: Which,
    runner: Runner,
}

/// An extractor variable as given: an empty value is a value, not unset (D3).
fn raw_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl Cli {
    /// Only the backend name is checked here; the executable is looked up at each invocation.
    pub fn new(backend: Option<&str>, model: &str, which: Which, runner: Runner) -> Result<Cli> {
        let backend = backend.map(String::from).or_else(|| raw_var("ENIGMA_EXTRACTOR")).unwrap_or_else(|| "codex".into());
        if !["codex", "claude"].contains(&backend.as_str()) {
            return Err(err(format!("Unknown backend: {backend}")));
        }
        Ok(Cli { backend, model: model.into(), which, runner })
    }

    /// The command line for one invocation (06 §5.4).
    pub fn argv(&self) -> Result<Vec<String>> {
        let backend = self.backend.as_str();
        let exe = (self.which)(backend).ok_or_else(|| err(format!("{backend} CLI not found on PATH")))?;
        let path = exe.to_string_lossy().into_owned();
        let ext = exe.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut argv: Vec<String> = match ext.as_str() {
            "cmd" | "bat" => vec!["cmd".into(), "/c".into(), path],
            "ps1" => ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].iter().map(|s| s.to_string()).chain([path]).collect(),
            _ => vec![path],
        };
        argv.push(if backend == "claude" { "-p" } else { "exec" }.into());
        if backend == "claude" && !self.model.is_empty() {
            argv.extend(["--model".into(), self.model.clone()]);
        }
        // note text is untrusted, so the extractor gets no tools unless the owner opts out
        if !crate::config::falsy("ENIGMA_EXTRACTOR_NO_TOOLS") {
            match backend {
                "claude" => argv.extend(["--tools".into(), String::new(), "--strict-mcp-config".into()]),
                _ => argv.extend(["--sandbox".into(), "read-only".into()]),
            }
        }
        Ok(argv)
    }

    /// The extraction extractor (06 §5.8) with the real process runner.
    pub fn extraction(backend: Option<&str>) -> Result<Cli> {
        let model = raw_var("ENIGMA_EXTRACT_MODEL").unwrap_or_else(|| "haiku".into());
        Cli::new(backend, &model, which_box(), Box::new(run_process))
    }

    /// The consolidation extractor (06 §5.8).
    pub fn consolidation(backend: Option<&str>) -> Result<Cli> {
        let model = raw_var("ENIGMA_CONSOLIDATE_MODEL").unwrap_or_else(|| "sonnet".into());
        Cli::new(backend, &model, which_box(), Box::new(run_process))
    }

    fn invoke(&self, prompt: &str) -> Result<String> {
        let b = &self.backend;
        match (self.runner)(&self.argv()?, prompt) {
            RunOutcome::Timeout => Err(err(format!("{b} timed out after {TIMEOUT_SECS}s"))),
            RunOutcome::SpawnError(e) => Err(err(format!("failed to run {b}: {e}"))),
            RunOutcome::Exit { code: 0, stdout, .. } => Ok(stdout),
            RunOutcome::Exit { code, stderr, .. } => Err(err(format!("{b} exited {code}: {}", stderr_summary(&stderr)))),
        }
    }
}

impl Complete for Cli {
    /// complete_json (06 §5.7): exactly one retry on a parse failure.
    fn complete_json(&self, prompt: &str) -> Result<Value> {
        if let Ok(v) = parse_reply(&self.invoke(prompt)?) {
            return Ok(v);
        }
        let again = format!("{prompt}\n\nYour previous reply was not valid JSON. Reply with ONLY the JSON object.");
        match parse_reply(&self.invoke(&again)?) {
            Ok(v) => Ok(v),
            Err(Ok(detail)) => Err(err(format!("invalid JSON after retry: {detail}"))),
            Err(Err(e)) => Err(e),
        }
    }
}

impl Extract for Cli {
    fn extract(&self, path: &str, content: &str) -> Result<Extraction> {
        Ok(coerce(&self.complete_json(&prompt(path, content))?))
    }
}

fn which_box() -> Which {
    Box::new(which_on_path)
}

/// Finds an executable file named name on PATH.
pub fn which_on_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file() && std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
}

/// Runs argv with the prompt on stdin; stdout/stderr decoded with replacement.
pub fn run_process(argv: &[String], stdin: &str) -> RunOutcome {
    run_process_for(argv, stdin, TIMEOUT_SECS)
}

/// run_process with its own timeout in seconds.
pub fn run_process_for(argv: &[String], stdin: &str, timeout_secs: u64) -> RunOutcome {
    let mut child = match Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return RunOutcome::SpawnError(e.to_string()),
    };
    let input = stdin.as_bytes().to_vec();
    let mut sin = child.stdin.take();
    let feeder = std::thread::spawn(move || {
        if let Some(s) = sin.as_mut() {
            let _ = s.write_all(&input);
        }
    });
    let (mut so, mut se) = (child.stdout.take(), child.stderr.take());
    let out = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let errs = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() >= Duration::from_secs(timeout_secs) => {
                let _ = child.kill();
                let _ = child.wait();
                return RunOutcome::Timeout;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return RunOutcome::SpawnError(e.to_string()),
        }
    };
    let _ = feeder.join();
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&errs.join().unwrap_or_default()).into_owned();
    use std::os::unix::process::ExitStatusExt;
    let code = status.code().unwrap_or_else(|| -status.signal().unwrap_or(0));
    RunOutcome::Exit { code, stdout, stderr }
}
