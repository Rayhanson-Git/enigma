//! Claude Code hooks and hook-sync (15 §3–§5).
use crate::error::{Error, Kind, Result};
use crate::extractor::{RunOutcome, json_to_y, run_process_for};
use crate::text::{display, quote_str};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const NUDGE: &str = "Significant session with no log_session \u{2014} consider filing a session log before finishing.";
const NUDGE_THRESHOLD: usize = 10;
pub const RECALL_TIMEOUT_SECS: u64 = 15;

/// EXE: the absolute canonical path of the running executable.
pub fn exe() -> PathBuf {
    std::env::current_exe().and_then(|p| p.canonicalize()).unwrap_or_default()
}

/// stdin text as a JSON object; empty → {} (15 §3 step 1, §4 step 1).
fn payload(stdin: &str) -> Option<Map<String, Value>> {
    if stdin.is_empty() {
        return Some(Map::new());
    }
    serde_json::from_str::<Value>(stdin).ok()?.as_object().cloned()
}

/// The project named by a payload's cwd (15 §3 step 2); empty when none.
pub fn project_of(stdin: &str) -> Option<String> {
    let cwd = payload(stdin)?.get("cwd").and_then(Value::as_str).unwrap_or_default().trim().replace('\\', "/");
    let project = cwd.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
    Some(project).filter(|p| !p.is_empty())
}

/// hook session-start (15 §3) → (stdout, stderr).
pub fn session_start(stdin: &str) -> (String, String) {
    let Some(project) = project_of(stdin) else { return Default::default() };
    let vault = std::env::var("ENIGMA_VAULT_PATH").unwrap_or_default();
    if vault.is_empty() {
        return (String::new(), "enigma session-start: ENIGMA_VAULT_PATH is not set; set it to enable recall.\n".into());
    }
    if !Path::new(&vault).is_dir() {
        let msg = format!("enigma session-start: vault not found at {}; set ENIGMA_VAULT_PATH to enable recall.\n", quote_str(&vault));
        return (String::new(), msg);
    }
    let argv: Vec<String> = [exe().display().to_string(), "recall".into(), "--vault".into(), vault, "--context".into(), "--project".into(), project].into();
    match run_process_for(&argv, "", RECALL_TIMEOUT_SECS) {
        RunOutcome::Exit { code: 0, stdout, .. } => (stdout, String::new()),
        _ => Default::default(),
    }
}

/// Counts tool uses in a transcript and whether any was a log_session (15 §4 step 3).
pub fn tool_uses(text: &str) -> (usize, bool) {
    let (mut count, mut logged) = (0, false);
    for line in text.lines() {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let message = match entry.get("message") {
            Some(m) => m.clone(),
            None => Value::Object(entry),
        };
        let Some(content) = message.get("content").and_then(Value::as_array) else { continue };
        for block in content.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")) {
            count += 1;
            logged |= block.get("name").map_or(String::new(), |n| display(&json_to_y(n))).contains("log_session");
        }
    }
    (count, logged)
}

/// hook stop-nudge (15 §4): Some(NUDGE) when the transcript earns one.
pub fn stop_nudge(stdin: &str) -> Option<&'static str> {
    let path = payload(stdin)?.get("transcript_path").and_then(Value::as_str).filter(|p| !p.is_empty())?.to_string();
    if !Path::new(&path).is_file() {
        return None;
    }
    // invalid UTF-8 is dropped, not replaced
    let text = String::from_utf8_lossy(&std::fs::read(&path).ok()?).replace('\u{FFFD}', "");
    let (count, logged) = tool_uses(&text);
    (count >= NUDGE_THRESHOLD && !logged).then_some(NUDGE)
}

// ---------------------------------------------------------------- hook-sync (§5)

pub const EVENTS: [(&str, &str); 2] = [("SessionStart", "session-start"), ("Stop", "stop-nudge")];

/// S: CLAUDE_SETTINGS_PATH when set and non-empty, else ~/.claude/settings.json.
pub fn settings_path() -> PathBuf {
    match std::env::var("CLAUDE_SETTINGS_PATH") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => crate::config::home().join(".claude/settings.json"),
    }
}

pub fn expected(exe: &Path, sub: &str) -> String {
    format!("\"{}\" hook {sub}", exe.display())
}

fn sync_error(msg: String) -> Error {
    Error::new(Kind::SyncError, msg)
}

pub fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    let text = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(Error::io(&e, path)),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text).map_err(|e| sync_error(format!("{}: {e}", path.display())))? {
        Value::Object(m) => Ok(m),
        _ => Err(sync_error(format!("{}: settings.json must be a JSON object", path.display()))),
    }
}

/// The command of the first hook object of ours for an event, if any.
fn ours(settings: &Map<String, Value>, event: &str, sub: &str) -> Option<String> {
    let markers = [format!("enigma-mcp hook {sub}"), format!("enigma-mcp.exe hook {sub}")];
    let entries = settings.get("hooks")?.get(event)?.as_array()?;
    entries.iter().filter_map(|e| e.get("hooks")?.as_array()).flatten().find_map(|h| {
        let cmd = h.get("command").and_then(Value::as_str).unwrap_or_default();
        let bare = cmd.replace('"', "");
        markers.iter().any(|m| bare.contains(m.as_str())).then(|| cmd.to_string())
    })
}

/// classify → {missing, present, drift} (15 §5).
pub fn classify(settings: &Map<String, Value>, exe: &Path) -> Value {
    let (mut missing, mut present, mut drift) = (vec![], vec![], vec![]);
    for (event, sub) in EVENTS {
        match ours(settings, event, sub) {
            None => missing.push(event),
            Some(cmd) if cmd == expected(exe, sub) => present.push(event),
            Some(_) => drift.push(event),
        }
    }
    json!({"missing": missing, "present": present, "drift": drift})
}

/// Refuses hook data hook-sync cannot merge into without discarding it (15 §5).
fn check_shape(settings: &Map<String, Value>, path: &Path) -> Result<()> {
    let bad = |what: String| Err(Error::new(Kind::InvalidData, format!("{}: {what}; settings left unchanged", path.display())));
    let hooks = match settings.get("hooks") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Object(h)) => h,
        Some(_) => return bad("\"hooks\" is not a JSON object".into()),
    };
    for (event, _) in EVENTS {
        match hooks.get(event) {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) if a.iter().all(Value::is_object) => {}
            Some(Value::Array(_)) => return bad(format!("hooks.{event} holds an entry that is not a JSON object")),
            Some(_) => return bad(format!("hooks.{event} is not a list")),
        }
    }
    Ok(())
}

/// hook-sync: the classification taken before any write; installs missing events when asked.
pub fn hook_sync(path: &Path, exe: &Path, install: bool) -> Result<Value> {
    let mut settings = read_settings(path)?;
    check_shape(&settings, path)?;
    let report = classify(&settings, exe);
    let missing: Vec<&str> = report["missing"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if install && !missing.is_empty() {
        let hooks = settings.entry("hooks").or_insert(Value::Null);
        if !hooks.is_object() {
            *hooks = json!({});
        }
        let hooks = hooks.as_object_mut().expect("just made an object");
        for (event, sub) in EVENTS.iter().filter(|(e, _)| missing.contains(e)) {
            let list = hooks.entry(*event).or_insert(Value::Null);
            if !list.is_array() {
                *list = json!([]);
            }
            if let Some(a) = list.as_array_mut() {
                a.push(json!({"hooks": [{"type": "command", "command": expected(exe, sub)}]}));
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(&e, parent))?;
        }
        // temp file, then rename: a crash can never leave settings.json cut short
        let tmp = path.with_extension("json.enigma-tmp");
        std::fs::write(&tmp, format!("{}\n", crate::json::indented(&settings))).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::io(&e, path)
        })?;
    }
    Ok(report)
}
