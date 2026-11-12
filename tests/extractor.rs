//! acceptance/extractor.yaml (process runner and PATH lookup are fakes)
mod common;
use enigma_mcp::error::Kind;
use enigma_mcp::extractor::*;
use enigma_mcp::graph::{Entity, Extraction, Relation};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<(Vec<String>, String)>>>;

fn ok(stdout: &str) -> RunOutcome {
    RunOutcome::Exit { code: 0, stdout: stdout.into(), stderr: String::new() }
}

/// A runner replaying outcomes in order and logging (argv, stdin).
fn scripted(outcomes: Vec<RunOutcome>) -> (Runner, Log) {
    let log: Log = Arc::default();
    let queue = Mutex::new(VecDeque::from(outcomes));
    let l = log.clone();
    let runner: Runner = Box::new(move |argv, stdin| {
        l.lock().unwrap().push((argv.to_vec(), stdin.to_string()));
        queue.lock().unwrap().pop_front().unwrap_or_else(|| ok("{}"))
    });
    (runner, log)
}

fn at(p: &str) -> Which {
    let p = PathBuf::from(p);
    Box::new(move |_| Some(p.clone()))
}

fn cli(backend: &str, model: &str, outcomes: Vec<RunOutcome>) -> (Cli, Log) {
    let (runner, log) = scripted(outcomes);
    (Cli::new(Some(backend), model, at(backend), runner).unwrap(), log)
}

fn good() -> Value {
    json!({
        "entities": [
            {"name": "Orbital Freight", "type": "organization", "aliases": ["ORBF"], "summary": "A company."},
            {"name": "Trade Routes", "type": "domain", "aliases": [], "summary": "Logistics domain."}
        ],
        "relations": [{"from": "Orbital Freight", "from_type": "organization", "rel": "operates_in", "to": "Trade Routes", "to_type": "domain", "evidence": "ships cargo"}]
    })
}

fn ent(name: &str, ty: &str, aliases: &[&str], summary: &str) -> Entity {
    Entity { name: name.into(), ty: ty.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), summary: summary.into() }
}

/// Runs the named child test in a fresh process with extra environment.
fn in_child(test: &str, env: &[(&str, &str)]) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([test, "--exact", "--nocapture"]).env("EX_CHILD", "1");
    for k in ["ENIGMA_EXTRACTOR", "ENIGMA_EXTRACTOR_NO_TOOLS", "ENIGMA_EXTRACT_MODEL", "ENIGMA_CONSOLIDATE_MODEL"] {
        cmd.env_remove(k);
    }
    let out = cmd.envs(env.iter().copied()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains("1 passed"), "{test}: {text}");
}

fn child() -> bool {
    std::env::var("EX_CHILD").is_ok()
}

#[test]
fn extractor_ex_01() {
    let p = prompt("Notes/x.md", "content").to_lowercase();
    assert!(p.contains("file path") || p.contains("file name") || p.contains("filename"));
    assert!(p.contains("canonical") && p.contains("alias"));
    assert!(p.ends_with("note path: notes/x.md\nnote content:\n---\ncontent\n---"));
}

#[test]
fn extractor_ex_02() {
    let x = coerce(&good());
    assert_eq!(
        x,
        Extraction {
            entities: vec![ent("Orbital Freight", "organization", &["ORBF"], "A company."), ent("Trade Routes", "domain", &[], "Logistics domain.")],
            relations: vec![Relation {
                from: "Orbital Freight".into(),
                from_type: "organization".into(),
                rel: "operates_in".into(),
                to: "Trade Routes".into(),
                to_type: "domain".into(),
                evidence: "ships cargo".into(),
            }],
        }
    );
}

#[test]
fn extractor_ex_03() {
    let x = coerce(&json!({
        "entities": [{"name": "Rocket", "type": "spaceship", "aliases": null}],
        "relations": [{"from": "A", "from_type": "alien", "rel": "zaps", "to": "B", "to_type": "planet"}]
    }));
    assert_eq!(x.entities[0], ent("Rocket", "topic", &[], ""));
    let r = &x.relations[0];
    assert_eq!((r.rel.as_str(), r.from_type.as_str(), r.to_type.as_str()), ("related_to", "topic", "topic"));
}

#[test]
fn extractor_ex_04() {
    let x = coerce(&json!({
        "entities": [{"name": "  ", "type": "person"}, {"name": null}, "not an object", {"name": "Kept"}],
        "relations": [{"from": " ", "to": "B"}, {"from": "A", "to": ""}]
    }));
    assert_eq!(x.entities.len(), 1);
    assert!(x.relations.is_empty());
}

#[test]
fn extractor_ex_05() {
    let (codex, _) = cli("codex", "", vec![]);
    assert_eq!(codex.argv().unwrap(), ["codex", "exec", "--sandbox", "read-only"]);
    let (claude, _) = cli("claude", "", vec![]);
    assert_eq!(claude.argv().unwrap(), ["claude", "-p", "--tools", "", "--strict-mcp-config"]);
}

#[test]
fn extractor_ex_06() {
    in_child("ex_06_off", &[("ENIGMA_EXTRACTOR_NO_TOOLS", "0")]);
    for v in [None, Some("1"), Some(""), Some("maybe")] {
        in_child("ex_06_set", &v.map(|v| ("ENIGMA_EXTRACTOR_NO_TOOLS", v)).into_iter().collect::<Vec<_>>());
    }
}

#[test]
fn ex_06_off() {
    if !child() {
        return;
    }
    assert_eq!(cli("claude", "", vec![]).0.argv().unwrap(), ["claude", "-p"]);
    assert_eq!(cli("codex", "", vec![]).0.argv().unwrap(), ["codex", "exec"]);
}

#[test]
fn ex_06_set() {
    if !child() {
        return;
    }
    let claude = cli("claude", "", vec![]).0;
    assert_eq!(claude.argv().unwrap(), ["claude", "-p", "--tools", "", "--strict-mcp-config"]);
    assert!(!claude.argv().unwrap().contains(&"--mcp-config".to_string()));
    assert_eq!(cli("codex", "", vec![]).0.argv().unwrap(), ["codex", "exec", "--sandbox", "read-only"]);
}

#[test]
fn extractor_ex_07() {
    let (c, _) = cli("codex", "", vec![ok("Sure! Here it is: {\"entities\": [], \"relations\": []} hope that helps")]);
    assert_eq!(c.complete_json("p").unwrap(), json!({"entities": [], "relations": []}));
    let (c, log) = cli("codex", "", vec![ok("no json here"), ok("{\"entities\": []}")]);
    assert_eq!(c.complete_json("p").unwrap(), json!({"entities": []}));
    let calls = log.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].1.ends_with("\n\nYour previous reply was not valid JSON. Reply with ONLY the JSON object."));
    drop(calls);
    let (c, log) = cli("codex", "", vec![ok("junk"), ok("still junk")]);
    assert_eq!(c.complete_json("p").unwrap_err().kind, Kind::ExtractorError);
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[test]
fn extractor_ex_08() {
    let (c, _) = cli("codex", "", vec![RunOutcome::Exit { code: 1, stdout: String::new(), stderr: "boom".into() }]);
    let e = c.extract("a.md", "x").unwrap_err();
    assert_eq!(e.kind, Kind::ExtractorError);
    assert_eq!(e.msg, "codex exited 1: boom");
}

#[test]
fn extractor_ex_09() {
    let (runner, _) = scripted(vec![]);
    let e = Cli::new(Some("gpt9000"), "", at("x"), runner).err().unwrap();
    assert!(e.msg.contains("Unknown backend") && e.kind == Kind::ExtractorError);
    in_child("ex_09_env", &[("ENIGMA_EXTRACTOR", "claude")]);
}

#[test]
fn ex_09_env() {
    if !child() {
        return;
    }
    let (runner, _) = scripted(vec![]);
    let which: Which = Box::new(|name: &str| Some(PathBuf::from(name)));
    let c = Cli::new(None, "", which, runner).unwrap();
    assert_eq!(c.argv().unwrap(), ["claude", "-p", "--tools", "", "--strict-mcp-config"]);
}

#[test]
fn extractor_ex_10() {
    assert!(ENTITY_TYPES.contains(&"organization"));
    assert!(RELATIONS.contains(&"related_to"));
}

#[test]
fn extractor_ex_11() {
    let content = "x".repeat(100 * 1024);
    let (c, log) = cli("codex", "", vec![ok("{}")]);
    c.extract("big.md", &content).unwrap();
    let (argv, stdin) = log.lock().unwrap()[0].clone();
    assert!(stdin.contains(&content));
    assert!(argv.iter().all(|a| a.len() < 100));
    // a real child process reading 100 KB from stdin
    let dir = tempfile::tempdir().unwrap();
    let script = common::stub_program(
        dir.path(),
        "codex",
        "#!/bin/sh\ncat > /dev/null\necho '{\"entities\": [{\"name\": \"Echo\"}]}'\n",
        "[Console]::In.ReadToEnd() | Out-Null\n'{\"entities\": [{\"name\": \"Echo\"}]}'\n",
    );
    let real = Cli::new(Some("codex"), "", at(script.to_str().unwrap()), Box::new(run_process)).unwrap();
    assert_eq!(real.extract("big.md", &content).unwrap().entities[0].name, "Echo");
}

#[test]
fn extractor_ex_12() {
    let (c, _) = cli("codex", "", vec![RunOutcome::Timeout]);
    assert_eq!(c.complete_json("p").unwrap_err().msg, "codex timed out after 120s");
    let (c, _) = cli("codex", "", vec![RunOutcome::SpawnError("No such file".into())]);
    assert_eq!(c.complete_json("p").unwrap_err().msg, "failed to run codex: No such file");
    let (runner, log) = scripted(vec![]);
    let nowhere: Which = Box::new(|_: &str| None);
    // the lookup happens at each invocation, not when the extractor is created
    let c = Cli::new(Some("claude"), "", nowhere, runner).unwrap();
    let e = c.complete_json("p").unwrap_err();
    assert_eq!((e.kind, e.msg.as_str()), (Kind::ExtractorError, "claude CLI not found on PATH"));
    assert!(log.lock().unwrap().is_empty(), "nothing runs and nothing is retried");
    assert!(run_process(&["/definitely/not/a/binary".into()], "").is_spawn_error());
}

trait SpawnCheck {
    fn is_spawn_error(&self) -> bool;
}
impl SpawnCheck for RunOutcome {
    fn is_spawn_error(&self) -> bool {
        matches!(self, RunOutcome::SpawnError(_))
    }
}

#[test]
fn extractor_ex_13() {
    let (c, _) = cli("codex", "", vec![ok("first {\"a\": 1} then {\"b\": 2}")]);
    assert_eq!(c.complete_json("p").unwrap(), json!({"a": 1}));
    let fenced = "stray {\"x\": 1} text\n\u{60}\u{60}\u{60}json\n{\"entities\": []}\n\u{60}\u{60}\u{60}\n";
    let (c, _) = cli("codex", "", vec![ok(fenced)]);
    assert_eq!(c.complete_json("p").unwrap(), json!({"entities": []}));
}

#[test]
fn extractor_ex_14() {
    let mk = |p: &str| Cli::new(Some("codex"), "", at(p), scripted(vec![]).0).unwrap().argv().unwrap().to_vec();
    assert_eq!(mk("C:/bin/codex.cmd"), ["cmd", "/c", "C:/bin/codex.cmd", "exec", "--sandbox", "read-only"]);
    assert_eq!(mk("C:/bin/codex.ps1"), ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:/bin/codex.ps1", "exec", "--sandbox", "read-only"]);
    assert_eq!(mk("C:/bin/codex.exe"), ["C:/bin/codex.exe", "exec", "--sandbox", "read-only"]);
}

#[test]
fn extractor_ex_15() {
    let (c, _) = cli("claude", "sonnet", vec![ok("{\"merges\": []}")]);
    assert_eq!(c.complete_json("any prompt").unwrap(), json!({"merges": []}));
    let (c, _) = cli("claude", "sonnet", vec![ok("junk"), ok("junk")]);
    assert_eq!(c.complete_json("any prompt").unwrap_err().kind, Kind::ExtractorError);
}

#[test]
fn extractor_ex_16() {
    assert!(cli("claude", "haiku", vec![]).0.argv().unwrap().windows(2).any(|w| w == ["--model", "haiku"]));
    assert!(!cli("codex", "haiku", vec![]).0.argv().unwrap().contains(&"--model".to_string()));
    assert!(!cli("claude", "", vec![]).0.argv().unwrap().contains(&"--model".to_string()));
}

#[test]
fn extractor_ex_17() {
    let dir = tempfile::tempdir().unwrap();
    common::stub_program(dir.path(), "claude", "#!/bin/sh\nexit 1\n", "exit 1\n");
    let path = dir.path().to_str().unwrap();
    in_child("ex_17_models", &[("PATH", path), ("ENIGMA_EXTRACTOR", "claude"), ("EX17_WANT", "haiku,sonnet")]);
    in_child(
        "ex_17_models",
        &[("PATH", path), ("ENIGMA_EXTRACTOR", "claude"), ("ENIGMA_EXTRACT_MODEL", "opus"), ("ENIGMA_CONSOLIDATE_MODEL", "big"), ("EX17_WANT", "opus,big")],
    );
}

#[test]
fn ex_17_models() {
    if !child() {
        return;
    }
    let want = std::env::var("EX17_WANT").unwrap();
    let (e, c) = want.split_once(',').unwrap();
    assert!(Cli::extraction(None).unwrap().argv().unwrap().windows(2).any(|w| w == ["--model", e]));
    assert!(Cli::consolidation(None).unwrap().argv().unwrap().windows(2).any(|w| w == ["--model", c]));
}

#[test]
fn extractor_ex_18() {
    assert_eq!(stderr_summary("info: start\nERROR: usage limit hit\nwarn: bye\n"), "ERROR: usage limit hit");
    assert_eq!(stderr_summary("fatal error early\nok\nFATAL: disk full"), "FATAL: disk full");
    let long = "y".repeat(400);
    assert_eq!(stderr_summary(&long), "y".repeat(300));
    assert_eq!(stderr_summary(""), "");
    assert_eq!(stderr_summary("   \n "), "");
    assert_eq!(stderr_summary("TERRORS everywhere"), "TERRORS everywhere");
}

#[test]
fn extractor_ex_19() {
    // PATH is consulted at every invocation: a CLI that appears or moves later is found then
    let (runner, log) = scripted(vec![ok("{}"), ok("{}")]);
    let turn = Arc::new(Mutex::new(0));
    let t = turn.clone();
    let which: Which = Box::new(move |_: &str| {
        let mut n = t.lock().unwrap();
        *n += 1;
        (*n > 1).then(|| PathBuf::from(format!("/bin{n}/codex")))
    });
    let c = Cli::new(Some("codex"), "", which, runner).unwrap();
    assert_eq!(c.extract("a.md", "x").unwrap_err().msg, "codex CLI not found on PATH");
    c.extract("a.md", "x").unwrap();
    c.extract("b.md", "x").unwrap();
    let argv0: Vec<String> = log.lock().unwrap().iter().map(|(a, _)| a[0].clone()).collect();
    assert_eq!(argv0, ["/bin2/codex", "/bin3/codex"]);
}

#[test]
fn extractor_ex_20() {
    let dir = tempfile::tempdir().unwrap();
    common::stub_program(dir.path(), "claude", "#!/bin/sh\nexit 1\n", "exit 1\n");
    let path = dir.path().to_str().unwrap();
    in_child("ex_20_empty", &[("PATH", path), ("ENIGMA_EXTRACTOR", ""), ("ENIGMA_EXTRACT_MODEL", ""), ("ENIGMA_CONSOLIDATE_MODEL", "")]);
}

#[test]
fn ex_20_empty() {
    if !child() {
        return;
    }
    // an empty value is a value, not unset
    let e = Cli::extraction(None).err().unwrap();
    assert_eq!((e.kind, e.msg.as_str()), (Kind::ExtractorError, "Unknown backend: "));
    for c in [Cli::extraction(Some("claude")).unwrap(), Cli::consolidation(Some("claude")).unwrap()] {
        // on Windows the stand-in is a .cmd, so the command starts with cmd /c <path>
        assert_eq!(c.argv().unwrap()[if cfg!(windows) { 3 } else { 1 }..], ["-p", "--tools", "", "--strict-mcp-config"]);
    }
}

#[test]
fn extractor_ex_21() {
    let spec = common::spec_block("06-indexer-extractor.md", "Normative text:");
    assert_eq!(PROMPT, spec);
    assert_eq!(PROMPT.matches('\u{2014}').count(), 1);
    let want = spec.replace("{path}", "Notes/x.md").replace("{content}", "line one\nline two");
    assert_eq!(prompt("Notes/x.md", "line one\nline two"), want);
    // placeholder text inside the values is sent unchanged
    let p = prompt("{content}.md", "say {path}");
    assert!(p.contains("Note path: {content}.md\nNote content:\n---\nsay {path}\n---") && p.ends_with("---"));
}
