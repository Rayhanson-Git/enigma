//! acceptance/sync.yaml: conventions, hooks, hook-sync, skill-sync, mcp-sync, provisioning, platform.
//! Every command runs in a Sandbox: HOME, TMPDIR, ENIGMA_STATE_DIR and CLAUDE_SETTINGS_PATH point into
//! a fresh temp dir and PATH holds only stub executables plus /usr/bin:/bin.
mod common;
use common::*;
use enigma_mcp::conventions::{self, CONSTITUTION, DECISIONS_SEED, GUIDE, INDEX_SEED, SKILLS};
use enigma_mcp::error::{Kind, Result};
use enigma_mcp::hooks::{self, NUDGE};
use enigma_mcp::mcp_sync::{self, Server};
use enigma_mcp::provision::{self, Fetch, Plugin};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let root = tempfile::tempdir().unwrap();
        for d in ["home", "tmp", "state", "bin"] {
            fs::create_dir_all(root.path().join(d)).unwrap();
        }
        Sandbox { root }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    fn settings(&self) -> PathBuf {
        self.p("settings.json")
    }
    /// A stub executable on the sandbox PATH.
    fn stub(&self, name: &str, script: &str) {
        let p = self.p("bin").join(name);
        fs::write(&p, script).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        for (k, _) in std::env::vars() {
            if k.starts_with("ENIGMA_") || k.starts_with("CLAUDE") || k == "HF_HUB_OFFLINE" {
                c.env_remove(k);
            }
        }
        c.args(args)
            .env("HOME", self.p("home"))
            .env("TMPDIR", self.p("tmp"))
            .env("ENIGMA_STATE_DIR", self.p("state"))
            .env("CLAUDE_SETTINGS_PATH", self.settings())
            .env("PATH", format!("{}:/usr/bin:/bin", self.p("bin").display()));
        c
    }
    fn run(&self, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Output {
        let mut c = self.cmd(args);
        c.envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    }
    fn go(&self, args: &[&str]) -> Output {
        self.run(args, b"", &[])
    }
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}
fn err(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}
fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}
fn exe() -> PathBuf {
    Path::new(bin()).canonicalize().unwrap()
}
fn spec(rel: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("spec").join(rel)).unwrap()
}
fn files_under(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .flatten()
        .map(|e| e.path().strip_prefix(root).unwrap().display().to_string())
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- conventions install

const ALL_CREATED: &str = "Claude/Inbox, Claude/Sessions, Claude/Concepts, Claude/Answers, Claude/Digests, Claude/README.md, Claude/Index.md, Claude/Decisions.md, CLAUDE.md, AGENTS.md";

fn installed() -> (Sandbox, TempDir, Output) {
    let s = Sandbox::new();
    let v = tempfile::tempdir().unwrap();
    let o = s.go(&["install-conventions", v.path().to_str().unwrap()]);
    (s, v, o)
}

#[test]
fn sync_cv_01() {
    let (_s, v, o) = installed();
    assert_eq!((out(&o), code(&o)), (format!("Created: {ALL_CREATED}\n"), 0));
    for f in conventions::FOLDERS {
        assert_eq!(fs::read_dir(v.path().join(f)).unwrap().count(), 0, "{f} is empty");
    }
    let created = conventions::install(tempfile::tempdir().unwrap().path(), Some("x")).unwrap();
    assert_eq!(created.join(", "), ALL_CREATED);
}

#[test]
fn sync_cv_02() {
    let (_s, v, _) = installed();
    let readme = fs::read(v.path().join("Claude/README.md")).unwrap();
    assert_eq!(readme, spec("constitution/README.md"));
    let text = String::from_utf8(readme).unwrap();
    assert!(text.starts_with("---\n") && text.contains("Constitution") && text.contains("Claude/Answers/") && text.contains("Claude/Digests/"));
    assert!(text.to_lowercase().contains("save what lasts"));
    assert_eq!(CONSTITUTION.as_bytes(), spec("constitution/README.md"));
}

#[test]
fn sync_cv_03() {
    let (_s, v, _) = installed();
    assert_eq!(fs::read_to_string(v.path().join("Claude/Index.md")).unwrap(), "# Index\n\nMap of agent-written notes. Session entries append below.\n\n");
    assert_eq!(INDEX_SEED, "# Index\n\nMap of agent-written notes. Session entries append below.\n\n");
}

#[test]
fn sync_cv_04() {
    let (_s, v, _) = installed();
    let want = "---\ncreated: 2026-01-01\nagent: claude\ntags: [decisions]\n---\n\n# Decisions\n\nAppend-only log. One line per decision:\n`- YYYY-MM-DD — <decision> ([[session note]])`\n\n";
    assert_eq!(fs::read_to_string(v.path().join("Claude/Decisions.md")).unwrap(), want);
    assert_eq!(DECISIONS_SEED, want);
}

#[test]
fn sync_cv_05() {
    let (_s, v, _) = installed();
    let claude = fs::read_to_string(v.path().join("CLAUDE.md")).unwrap();
    assert_eq!(claude, fs::read_to_string(v.path().join("AGENTS.md")).unwrap());
    assert_eq!(claude, GUIDE.replace("{OWNER}", "the vault owner"));
    assert!(!claude.contains("{OWNER}"));
    assert!(claude.contains("Routing rules") && claude.contains("Claude/README") && claude.contains("search_brain"));
    assert_eq!(GUIDE.as_bytes(), spec("constitution/vault-guide.md"));
}

#[test]
fn sync_cv_06() {
    for (owner, want) in [("Alex", "Alex"), ("  Sam\r\nLee  ", "Sam  Lee"), ("   ", "the vault owner")] {
        let s = Sandbox::new();
        let v = tempfile::tempdir().unwrap();
        s.run(&["install-conventions", v.path().to_str().unwrap()], b"", &[("ENIGMA_OWNER", owner)]);
        for g in ["CLAUDE.md", "AGENTS.md"] {
            let text = fs::read_to_string(v.path().join(g)).unwrap();
            assert!(text.contains(want) && !text.contains("{OWNER}"), "{owner:?} in {g}");
        }
    }
    // an explicit owner wins over ENIGMA_OWNER, even when empty
    assert_eq!(conventions::owner_name(Some("")), "the vault owner");
    assert_eq!(conventions::owner_name(Some(" Pat ")), "Pat");
    let v = tempfile::tempdir().unwrap();
    conventions::install(v.path(), Some("")).unwrap();
    assert_eq!(fs::read_to_string(v.path().join("CLAUDE.md")).unwrap(), GUIDE.replace("{OWNER}", "the vault owner"));
}

#[test]
fn sync_cv_07() {
    let (s, v, _) = installed();
    fs::write(v.path().join("Claude/Index.md"), "# Index\n\n- [[existing]]\n").unwrap();
    fs::write(v.path().join("AGENTS.md"), "Keep my instructions").unwrap();
    assert!(conventions::install(v.path(), Some("Someone Else")).unwrap().is_empty());
    let o = s.go(&["install-conventions", v.path().to_str().unwrap()]);
    assert_eq!((out(&o), code(&o)), ("Created: nothing (already installed)\n".into(), 0));
    assert!(fs::read_to_string(v.path().join("Claude/Index.md")).unwrap().contains("existing"));
    assert_eq!(fs::read_to_string(v.path().join("AGENTS.md")).unwrap(), "Keep my instructions");
}

#[test]
fn sync_cv_08() {
    let v = tempfile::tempdir().unwrap();
    fs::create_dir_all(v.path().join("Claude/Inbox")).unwrap();
    fs::write(v.path().join("CLAUDE.md"), "mine").unwrap();
    let created = conventions::install(v.path(), None).unwrap();
    assert_eq!(
        created,
        ["Claude/Sessions", "Claude/Concepts", "Claude/Answers", "Claude/Digests", "Claude/README.md", "Claude/Index.md", "Claude/Decisions.md", "AGENTS.md"]
    );
    assert_eq!(fs::read_to_string(v.path().join("CLAUDE.md")).unwrap(), "mine");
}

#[test]
fn sync_cv_09() {
    let s = Sandbox::new();
    let o = s.go(&["install-conventions"]);
    assert_eq!((err(&o), code(&o)), ("Usage: enigma-mcp install-conventions <vault-path>\n".to_string(), 1));
    assert!(o.stdout.is_empty());
    let missing = s.p("x/missing");
    let o = s.go(&["install-conventions", missing.to_str().unwrap()]);
    assert_eq!((err(&o), code(&o)), (format!("Vault not found: {}\n", missing.display()), 1));
    assert!(!s.p("x").exists());
    let file = s.p("plain.txt");
    fs::write(&file, "x").unwrap();
    let o = s.go(&["install-conventions", file.to_str().unwrap()]);
    assert_eq!((err(&o), code(&o)), (format!("Vault not found: {}\n", file.display()), 1));
}

// ---------------------------------------------------------------- hook: session-start

fn gravel() -> TempDir {
    fixture_vault()
}

fn hook(s: &Sandbox, sub: &str, stdin: &str, env: &[(&str, &str)]) -> Output {
    s.run(&["hook", sub], stdin.as_bytes(), env)
}

#[test]
fn sync_hk_01() {
    let s = Sandbox::new();
    let v = gravel();
    let vp = v.path().to_str().unwrap();
    let recall = s.go(&["recall", "--vault", vp, "--context", "--project", "gravel-bike"]);
    assert!(out(&recall).contains("Gravel bike tubeless setup"), "{}", out(&recall));
    for cwd in ["/home/ana/projects/gravel-bike", "C:\\Users\\ana\\code\\gravel-bike", "C:\\repo\\gravel-bike\\\\", "/home/ana/gravel-bike/", "  /srv/gravel-bike  "] {
        let o = hook(&s, "session-start", &json!({"cwd": cwd}).to_string(), &[("ENIGMA_VAULT_PATH", vp)]);
        assert_eq!((out(&o), code(&o)), (out(&recall), 0), "{cwd}");
        assert_eq!(hooks::project_of(&json!({"cwd": cwd}).to_string()).as_deref(), Some("gravel-bike"));
    }
}

#[test]
fn sync_hk_02() {
    let s = Sandbox::new();
    let v = gravel();
    for stdin in ["{}", "{\"cwd\": \"\"}", "{\"cwd\": null}", "{\"cwd\": \"/\"}", ""] {
        let o = hook(&s, "session-start", stdin, &[]);
        assert_eq!((out(&o), err(&o), code(&o)), (String::new(), String::new(), 0), "{stdin:?}");
        // with a vault set, recall still does not run
        let o = hook(&s, "session-start", stdin, &[("ENIGMA_VAULT_PATH", v.path().to_str().unwrap())]);
        assert_eq!((out(&o), err(&o), code(&o)), (String::new(), String::new(), 0), "{stdin:?}");
    }
}

#[test]
fn sync_hk_03() {
    let s = Sandbox::new();
    for stdin in ["not json", "[1, 2]", "{\"cwd\": 42}"] {
        let o = hook(&s, "session-start", stdin, &[]);
        assert_eq!((out(&o), code(&o)), (String::new(), 0), "{stdin}");
    }
    let o = s.run(&["hook", "session-start"], b"\xff\xfe", &[]);
    assert_eq!((out(&o), code(&o)), (String::new(), 0));
}

#[test]
fn sync_hk_04() {
    let s = Sandbox::new();
    let o = hook(&s, "session-start", "{\"cwd\": \"/tmp/t/gravel-bike\"}", &[("ENIGMA_VAULT_PATH", "/tmp/t/no-such-vault")]);
    assert_eq!(out(&o), "");
    assert_eq!(err(&o), "enigma session-start: vault not found at '/tmp/t/no-such-vault'; set ENIGMA_VAULT_PATH to enable recall.\n");
    assert_eq!(code(&o), 0);
}

#[test]
fn sync_hk_05() {
    let s = Sandbox::new();
    for env in [vec![], vec![("ENIGMA_VAULT_PATH", "")]] {
        let o = hook(&s, "session-start", "{\"cwd\": \"/tmp/t/gravel-bike\"}", &env);
        assert_eq!(out(&o), "");
        assert_eq!(err(&o), "enigma session-start: ENIGMA_VAULT_PATH is not set; set it to enable recall.\n");
        assert_eq!(code(&o), 0);
    }
}

#[test]
fn sync_hk_06() {
    let s = Sandbox::new();
    // recall finds nothing for this project: empty output, nothing printed
    let v = gravel();
    let o = hook(&s, "session-start", "{\"cwd\": \"/x/no-such-project-anywhere\"}", &[("ENIGMA_VAULT_PATH", v.path().to_str().unwrap())]);
    assert_eq!((out(&o), err(&o), code(&o)), (String::new(), String::new(), 0));
    // the recall step is bounded by 15 s; a step past its bound yields nothing
    assert_eq!(hooks::RECALL_TIMEOUT_SECS, 15);
    let slow = ["sh".to_string(), "-c".into(), "echo partial; echo noise >&2; sleep 5".into()];
    assert!(matches!(enigma_mcp::extractor::run_process_for(&slow, "", 1), enigma_mcp::extractor::RunOutcome::Timeout));
}

// ---------------------------------------------------------------- hook: stop-nudge

fn tu(name: &str) -> String {
    json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "name": name, "input": {}}]}}).to_string()
}

fn transcript(s: &Sandbox, lines: &[String]) -> String {
    let p = s.p("transcript.jsonl");
    fs::write(&p, lines.join("\n") + "\n").unwrap();
    json!({"transcript_path": p}).to_string()
}

fn tus(name: &str, n: usize) -> Vec<String> {
    (0..n).map(|_| tu(name)).collect()
}

#[test]
fn sync_hk_10() {
    let s = Sandbox::new();
    let o = hook(&s, "stop-nudge", &transcript(&s, &tus("Read", 10)), &[]);
    assert_eq!((out(&o), err(&o), code(&o)), (format!("{NUDGE}\n"), String::new(), 0));
    assert_eq!(NUDGE, "Significant session with no log_session — consider filing a session log before finishing.");
}

#[test]
fn sync_hk_11() {
    let s = Sandbox::new();
    let mut with_log = tus("Read", 10);
    with_log.push(tu("mcp__enigma__log_session"));
    let mut v2 = tus("Edit", 25);
    v2.push(tu("log_session_v2"));
    for (lines, nudge) in [(tus("Read", 9), false), (tus("Read", 10), true), (with_log, false), (v2, false), (tus("Read", 2), false)] {
        let o = hook(&s, "stop-nudge", &transcript(&s, &lines), &[]);
        assert_eq!(code(&o), 0);
        assert_eq!(out(&o), if nudge { format!("{NUDGE}\n") } else { String::new() }, "{} lines", lines.len());
    }
}

#[test]
fn sync_hk_12() {
    let text = ["not json".to_string(), String::new(), tu("Read")].join("\n");
    assert_eq!(hooks::tool_uses(&text), (1, false));
    let user = json!({"type": "user", "message": {"role": "user", "content": "hello"}}).to_string();
    assert_eq!(hooks::tool_uses(&user), (0, false));
    assert_eq!(hooks::tool_uses(&json!({"content": [{"type": "tool_use", "name": "X"}]}).to_string()), (1, false));
    // whatever the entry's own type; message not an object or content not a list counts nothing
    let odd = [
        json!({"type": "weird", "message": {"content": [{"type": "tool_use"}, {"type": "text"}, "x"]}}).to_string(),
        json!({"message": "text", "content": [{"type": "tool_use"}]}).to_string(),
        json!({"message": {"content": {"type": "tool_use"}}}).to_string(),
        "[1]".into(),
        format!("   {}   ", tu("log_session")),
    ];
    assert_eq!(hooks::tool_uses(&odd.join("\n")), (2, true));
}

#[test]
fn sync_hk_13() {
    let s = Sandbox::new();
    let nope = json!({"transcript_path": s.p("tmp/nope.jsonl")}).to_string();
    let dir = json!({"transcript_path": s.p("tmp")}).to_string();
    for stdin in ["{}", "{\"transcript_path\": \"\"}", nope.as_str(), dir.as_str(), "not json"] {
        let o = hook(&s, "stop-nudge", stdin, &[]);
        assert_eq!((out(&o), code(&o)), (String::new(), 0), "{stdin}");
    }
}

#[test]
fn sync_hk_14() {
    let s = Sandbox::new();
    let p = s.p("t.jsonl");
    let mut bytes = Vec::new();
    for (i, l) in tus("Read", 10).iter().enumerate() {
        bytes.extend_from_slice(l.as_bytes());
        bytes.push(b'\n');
        if i == 4 {
            bytes.push(0xFF);
        }
    }
    fs::write(&p, bytes).unwrap();
    let o = hook(&s, "stop-nudge", &json!({"transcript_path": p}).to_string(), &[]);
    assert_eq!((out(&o), code(&o)), (format!("{NUDGE}\n"), 0));
}

// ---------------------------------------------------------------- hook-sync

fn expected(sub: &str) -> String {
    format!("\"{}\" hook {sub}", exe().display())
}

fn settings(s: &Sandbox) -> Value {
    serde_json::from_slice(&fs::read(s.settings()).unwrap()).unwrap()
}

fn class(o: &Output) -> Value {
    serde_json::from_str(&out(o)).unwrap()
}

#[test]
fn sync_hs_01() {
    let s = Sandbox::new();
    let o = s.go(&["hook-sync", "--check"]);
    assert_eq!(out(&o), "{\n  \"missing\": [\n    \"SessionStart\",\n    \"Stop\"\n  ],\n  \"present\": [],\n  \"drift\": []\n}\n");
    assert_eq!(code(&o), 1);
    assert!(!s.settings().exists());
}

#[test]
fn sync_hs_02() {
    let s = Sandbox::new();
    let orig = json!({"model": "sonnet", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "some-other-tool.exe"}]}],
        "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "lint.sh"}]}]}});
    fs::write(s.settings(), orig.to_string()).unwrap();
    let o = s.go(&["hook-sync", "--install"]);
    assert_eq!(class(&o), json!({"missing": ["SessionStart", "Stop"], "present": [], "drift": []}));
    assert_eq!(code(&o), 0);
    let after = settings(&s);
    assert_eq!(after["model"], "sonnet");
    assert_eq!(after["hooks"]["PreToolUse"], orig["hooks"]["PreToolUse"]);
    let ss = after["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(ss.len(), 2);
    assert_eq!(ss[0], orig["hooks"]["SessionStart"][0]);
    assert_eq!(ss[1], json!({"hooks": [{"type": "command", "command": expected("session-start")}]}));
    assert_eq!(after["hooks"]["Stop"], json!([{"hooks": [{"type": "command", "command": expected("stop-nudge")}]}]));
    let raw = fs::read_to_string(s.settings()).unwrap();
    assert_eq!(raw, format!("{}\n", enigma_mcp::json::indented(&after)));
    assert_eq!(after.as_object().unwrap().keys().collect::<Vec<_>>(), ["model", "hooks"]);
    assert_eq!(after["hooks"].as_object().unwrap().keys().collect::<Vec<_>>(), ["SessionStart", "PreToolUse", "Stop"]);
}

#[test]
fn sync_hs_03() {
    let s = Sandbox::new();
    s.go(&["hook-sync", "--check"]);
    assert!(!s.settings().exists());
    for text in ["{\"model\": \"x\"}", "{\"hooks\": {\"Stop\": [{\"hooks\": [{\"command\": \"/old/enigma-mcp hook stop-nudge\"}]}]}}"] {
        fs::write(s.settings(), text).unwrap();
        s.go(&["hook-sync", "--check"]);
        assert_eq!(fs::read_to_string(s.settings()).unwrap(), text);
    }
}

#[test]
fn sync_hs_04() {
    let s = Sandbox::new();
    let orig = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "/old/bin/enigma-mcp hook session-start"}]}]}});
    fs::write(s.settings(), orig.to_string()).unwrap();
    let o = s.go(&["hook-sync", "--check"]);
    assert_eq!((class(&o), code(&o)), (json!({"missing": ["Stop"], "present": [], "drift": ["SessionStart"]}), 1));
    let o = s.go(&["hook-sync", "--install"]);
    assert_eq!(code(&o), 0);
    let after = settings(&s);
    assert_eq!(after["hooks"]["SessionStart"], orig["hooks"]["SessionStart"]);
    assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["command"], json!(expected("stop-nudge")));
}

#[test]
fn sync_hs_05() {
    let s = Sandbox::new();
    let text = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "other-tool"}, {"type": "command", "command": expected("session-start")}]}]}});
    fs::write(s.settings(), text.to_string()).unwrap();
    assert_eq!(class(&s.go(&["hook-sync", "--check"])), json!({"missing": ["Stop"], "present": ["SessionStart"], "drift": []}));
}

#[test]
fn sync_hs_06() {
    let s = Sandbox::new();
    s.go(&["hook-sync", "--install"]);
    let first = fs::read(s.settings()).unwrap();
    let o = s.go(&["hook-sync", "--install"]);
    assert_eq!(class(&o), json!({"missing": [], "present": ["SessionStart", "Stop"], "drift": []}));
    assert_eq!(fs::read(s.settings()).unwrap(), first);
    let after = settings(&s);
    assert_eq!((after["hooks"]["SessionStart"].as_array().unwrap().len(), after["hooks"]["Stop"].as_array().unwrap().len()), (1, 1));
}

#[test]
fn sync_hs_07() {
    let e = exe();
    let cls = |event: &str, cmd: &str| {
        let s = json!({"hooks": {event: [{"matcher": "x"}, {"hooks": [{"type": "command"}, {"type": "command", "command": cmd}]}]}});
        let r = hooks::classify(s.as_object().unwrap(), &e);
        ["missing", "present", "drift"].into_iter().find(|k| r[*k].as_array().unwrap().contains(&json!(event))).unwrap()
    };
    assert_eq!(cls("SessionStart", "enigma-mcp hook session-start"), "drift");
    assert_eq!(cls("SessionStart", &expected("session-start")), "present");
    assert_eq!(cls("Stop", "\"/opt/e/enigma-mcp\" hook stop-nudge"), "drift");
    assert_eq!(cls("SessionStart", "C:\\e\\enigma-mcp.exe hook session-start"), "drift");
    assert_eq!(cls("SessionStart", "enigma-mcp hook stop-nudge"), "missing");
    assert_eq!(cls("SessionStart", "some-other-tool.exe"), "missing");
    // the first matching hook object in file order decides
    let s = json!({"hooks": {"Stop": [{"hooks": [{"command": "/a/enigma-mcp hook stop-nudge"}]}, {"hooks": [{"command": expected("stop-nudge")}]}]}});
    assert_eq!(hooks::classify(s.as_object().unwrap(), &e)["drift"], json!(["Stop"]));
}

#[test]
fn sync_hs_08() {
    let s = Sandbox::new();
    for text in ["", "  \n "] {
        fs::write(s.settings(), text).unwrap();
        assert_eq!(code(&s.go(&["hook-sync", "--check"])), 1);
        assert_eq!(fs::read_to_string(s.settings()).unwrap(), text);
    }
    fs::write(s.settings(), "[]").unwrap();
    let o = s.go(&["hook-sync", "--install"]);
    assert_eq!((err(&o), code(&o)), (format!("error: {}: settings.json must be a JSON object\n", s.settings().display()), 2));
    assert_eq!(fs::read_to_string(s.settings()).unwrap(), "[]");
    fs::write(s.settings(), "{not json").unwrap();
    let o = s.go(&["hook-sync", "--install"]);
    assert!(err(&o).starts_with("error: ") && code(&o) == 2);
    assert_eq!(fs::read_to_string(s.settings()).unwrap(), "{not json");
    fs::write(s.settings(), "{\"hooks\": null, \"theme\": \"dark\"}").unwrap();
    assert_eq!(code(&s.go(&["hook-sync", "--install"])), 0);
    let after = settings(&s);
    assert_eq!(after["theme"], "dark");
    assert_eq!(after["hooks"].as_object().unwrap().keys().collect::<Vec<_>>(), ["SessionStart", "Stop"]);
    // malformed hook data is refused, never replaced: one line, exit 1, S byte-identical (D8)
    let path = s.settings().display().to_string();
    for (text, why) in [
        ("{\"hooks\": [1]}", "\"hooks\" is not a JSON object".to_string()),
        ("{\"hooks\": \"x\"}", "\"hooks\" is not a JSON object".to_string()),
        ("{\"hooks\": {\"Stop\": {\"a\": 1}}}", "hooks.Stop is not a list".to_string()),
        ("{\"hooks\": {\"SessionStart\": \"x\"}}", "hooks.SessionStart is not a list".to_string()),
        ("{\"hooks\": {\"Stop\": [1, {\"hooks\": []}]}}", "hooks.Stop holds an entry that is not a JSON object".to_string()),
    ] {
        for mode in ["--check", "--install"] {
            fs::write(s.settings(), text).unwrap();
            let o = s.go(&["hook-sync", mode]);
            assert_eq!((err(&o), code(&o)), (format!("error: InvalidData: {path}: {why}; settings left unchanged\n"), 1), "{text} {mode}");
            assert!(o.stdout.is_empty());
            assert_eq!(fs::read_to_string(s.settings()).unwrap(), text);
        }
    }
    // other events are not ours to judge; a null event value still counts as none
    fs::write(s.settings(), "{\"hooks\": {\"PreToolUse\": \"odd\", \"Stop\": null}}").unwrap();
    assert_eq!(code(&s.go(&["hook-sync", "--install"])), 0);
    assert_eq!(settings(&s)["hooks"]["PreToolUse"], "odd");
}

#[test]
fn sync_hs_09() {
    let s = Sandbox::new();
    let o = s.go(&["hook-sync"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("specify --check or --install"));
    let o = s.go(&["hook-sync", "--check", "--install"]);
    assert_eq!(code(&o), 1);
    assert!(!s.settings().exists());
    // without CLAUDE_SETTINGS_PATH the (sandbox) home default is used
    let mut c = s.cmd(&["hook-sync", "--install"]);
    c.env_remove("CLAUDE_SETTINGS_PATH");
    assert!(c.output().unwrap().status.success());
    assert!(s.p("home/.claude/settings.json").is_file());
}

// ---------------------------------------------------------------- skill-sync

fn skills(s: &Sandbox, args: &[&str]) -> (Value, i32) {
    let dest = s.p("skills");
    let mut a = vec!["skill-sync", "--dest", dest.to_str().unwrap()];
    a.extend_from_slice(args);
    let o = s.go(&a);
    (serde_json::from_str(&out(&o)).unwrap(), code(&o))
}

const NAMES: [&str; 9] = ["clip", "connections", "decide", "digest", "distill", "dossier", "recall", "resume", "rounds"];

#[test]
fn sync_sk_01() {
    assert_eq!(SKILLS.iter().map(|(n, _)| *n).collect::<Vec<_>>(), NAMES);
    for (name, bytes) in SKILLS {
        assert_eq!(bytes.to_vec(), spec(&format!("../skills/{name}/SKILL.md")));
        let text = std::str::from_utf8(bytes).unwrap();
        assert!(text.starts_with("---\n"));
        let end = text[4..].find("\n---").unwrap() + 4;
        let Some(enigma_mcp::yaml::Y::Map(m)) = enigma_mcp::yaml::parse_doc(&text[4..end]).unwrap() else { panic!("{name}") };
        let get = |k: &str| m.iter().find(|(key, _)| enigma_mcp::text::display(key) == k).map(|(_, v)| enigma_mcp::text::display(v)).unwrap();
        assert_eq!(get("name"), *name);
        assert!(get("description").chars().count() > 40, "{name}");
    }
}

#[test]
fn sync_sk_02() {
    let s = Sandbox::new();
    let (r, c) = skills(&s, &[]);
    assert_eq!((r, c), (json!({"installed": NAMES, "updated": [], "up_to_date": [], "drift": []}), 0));
    for (name, bytes) in SKILLS {
        assert_eq!(fs::read(s.p("skills").join(name).join("SKILL.md")).unwrap(), bytes.to_vec());
    }
}

#[test]
fn sync_sk_03() {
    let s = Sandbox::new();
    write_file(&s.p("skills"), "recall/SKILL.md", "user edited this\n");
    let (r, c) = skills(&s, &[]);
    assert_eq!(r["drift"], json!(["recall"]));
    assert_eq!(r["installed"].as_array().unwrap().len(), 8);
    assert_eq!(c, 0);
    assert_eq!(fs::read_to_string(s.p("skills/recall/SKILL.md")).unwrap(), "user edited this\n");
}

#[test]
fn sync_sk_04() {
    let s = Sandbox::new();
    write_file(&s.p("skills"), "recall/SKILL.md", "user edited this\n");
    write_file(&s.p("skills"), "recall/notes.txt", "mine\n");
    let (r, _) = skills(&s, &["--force"]);
    assert_eq!(r["updated"], json!(["recall"]));
    assert_eq!(files_under(&s.p("skills/recall")), ["SKILL.md"]);
    assert_eq!(fs::read(s.p("skills/recall/SKILL.md")).unwrap(), SKILLS[6].1.to_vec());
}

#[test]
fn sync_sk_05() {
    let s = Sandbox::new();
    skills(&s, &[]);
    let p = s.p("skills/recall/SKILL.md");
    let old = filetime(&p);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let (r, c) = skills(&s, &[]);
    assert_eq!((r, c), (json!({"installed": [], "updated": [], "up_to_date": NAMES, "drift": []}), 0));
    assert_eq!(filetime(&p), old);
}

fn filetime(p: &Path) -> std::time::SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

#[test]
fn sync_sk_06() {
    let s = Sandbox::new();
    let (r, c) = skills(&s, &["--check"]);
    assert_eq!(r["installed"], json!(NAMES));
    assert_eq!(c, 1);
    assert!(!s.p("skills").exists());
    skills(&s, &[]);
    assert_eq!(skills(&s, &["--check"]).1, 0);
    fs::write(s.p("skills/recall/SKILL.md"), "user edited this\n").unwrap();
    let (r, c) = skills(&s, &["--check", "--force"]);
    assert_eq!((r["drift"].clone(), r["updated"].clone(), c), (json!(["recall"]), json!([]), 1));
    assert_eq!(fs::read_to_string(s.p("skills/recall/SKILL.md")).unwrap(), "user edited this\n");
}

#[test]
fn sync_sk_07() {
    let s = Sandbox::new();
    skills(&s, &[]);
    write_file(&s.p("skills"), "digest/extra.md", "x");
    write_file(&s.p("skills"), "my-own/SKILL.md", "mine");
    let (r, c) = skills(&s, &[]);
    assert_eq!(r["drift"], json!(["digest"]));
    assert!(!out(&s.go(&["skill-sync", "--dest", s.p("skills").to_str().unwrap()])).contains("my-own"));
    assert_eq!(fs::read_to_string(s.p("skills/my-own/SKILL.md")).unwrap(), "mine");
    assert_eq!(c, 0);
}

// ---------------------------------------------------------------- mcp-sync

/// A stub claude that records each argv (one line, args joined by '|') and exits per its rules.
fn recorder(s: &Sandbox, fail_on: Option<(&str, i32)>) -> PathBuf {
    let log = s.p("claude.log");
    let rule = fail_on.map_or(String::new(), |(w, c)| format!("case \"$*\" in *{w}*) exit {c};; esac\n"));
    s.stub("claude", &format!("#!/bin/sh\n(printf '%s' \"$0\"; for a in \"$@\"; do printf '|%s' \"$a\"; done; echo) >> '{}'\n{rule}exit 0\n", log.display()));
    log
}

fn manifest(s: &Sandbox, servers: Value) -> PathBuf {
    let p = s.p("m/mcp-servers.json");
    write_file(&s.p("m"), "mcp-servers.json", &json!({"servers": servers}).to_string());
    p
}

fn config(s: &Sandbox, v: &Value) -> PathBuf {
    let p = s.p("claude.json");
    fs::write(&p, v.to_string()).unwrap();
    p
}

fn mcp(s: &Sandbox, args: &[&str]) -> Output {
    let mut a = vec!["mcp-sync"];
    a.extend_from_slice(args);
    s.go(&a)
}

fn fetch_entry() -> Value {
    json!({"name": "fetch", "command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8"}, "why": "Web."})
}

fn arxiv_entry() -> Value {
    json!({"name": "arxiv", "command": "uvx", "args": ["arxiv-mcp-server@0.5.0"], "why": "Papers."})
}

#[test]
fn sync_ms_01() {
    let m = mcp_sync::builtin_manifest();
    let servers = m["servers"].as_array().unwrap();
    assert_eq!(servers.iter().map(|s| s["name"].as_str().unwrap()).collect::<Vec<_>>(), ["enigma", "fetch", "arxiv"]);
    assert_eq!(
        servers[0],
        json!({"name": "enigma", "transport": "stdio", "command": "{ENIGMA_BIN}", "args": [], "env": {"ENIGMA_VAULT_PATH": "{VAULT}"}, "why": "The memory vault: persistent shared memory for agents."})
    );
    assert_eq!(
        servers[1],
        json!({"name": "fetch", "transport": "stdio", "command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8"}, "why": "Web ingest: turns a URL into clean markdown for /clip."})
    );
    assert_eq!(
        servers[2],
        json!({"name": "arxiv", "transport": "stdio", "command": "uvx", "args": ["arxiv-mcp-server@0.5.0"], "env": {}, "why": "Paper ingest: arXiv search and download as markdown. Paper content is untrusted input."})
    );
    let text = m.to_string();
    assert!(!text.contains(":\\\\"));
}

#[test]
fn sync_ms_02() {
    let m = json!({"servers": [{"name": "x", "command": "{ENIGMA_BIN}", "url": "", "args": ["{REPO}/a", "{VAULT}"], "env": {"{VAULT}": "{VAULT}"}, "why": "{VAULT}"}]});
    let before = m.clone();
    let r = mcp_sync::resolve(&m, Path::new("/repo"), Some("/v"), Path::new("/bin/e")).unwrap();
    assert_eq!(r, vec![Server { name: "x".into(), url: None, command: Some("/bin/e".into()), args: vec!["/repo/a".into(), "/v".into()], env: vec![("{VAULT}".into(), "/v".into())], why: "{VAULT}".into() }]);
    assert_eq!(m, before);
    assert_eq!(mcp_sync::resolve(&m, Path::new("/repo"), None, Path::new("/e")).unwrap_err(), "manifest entry needs {VAULT}: pass --vault or set ENIGMA_VAULT_PATH");
    assert!(mcp_sync::resolve(&json!({"servers": [arxiv_entry()]}), Path::new("/r"), None, Path::new("/e")).is_ok());
    // CLI: the built-in enigma entry needs a vault
    let s = Sandbox::new();
    recorder(&s, None);
    let c = config(&s, &json!({}));
    let o = mcp(&s, &["--config", c.to_str().unwrap()]);
    assert_eq!((out(&o), code(&o)), ("ABORT (nothing changed): manifest entry needs {VAULT}: pass --vault or set ENIGMA_VAULT_PATH\n".into(), 2));
    assert!(!s.p("claude.log").exists());
    // {REPO} of a manifest file is its absolute folder; {ENIGMA_BIN} is EXE
    let m = manifest(&s, json!([{"name": "r", "command": "{ENIGMA_BIN}", "args": ["{REPO}"]}]));
    let c = config(&s, &json!({"mcpServers": {"r": {"command": exe(), "args": [s.p("m")]}}}));
    let o = mcp(&s, &["--check", "--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!((out(&o), code(&o)), ("present : r\n".into(), 0));
}

#[test]
fn sync_ms_03() {
    let s = Sandbox::new();
    recorder(&s, None);
    let c = config(&s, &json!({}));
    let m = manifest(&s, json!([{"name": "broken", "transport": "stdio"}]));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(out(&o), "ABORT (nothing changed): manifest entry 'broken': stdio transport requires a command (or provide a url)\n");
    assert_eq!(code(&o), 2);
    assert!(!s.p("claude.log").exists());
    let m = manifest(&s, json!([{"transport": "stdio"}]));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert!(out(&o).contains("manifest entry null: stdio transport"));
}

#[test]
fn sync_ms_04() {
    let s = Sandbox::new();
    recorder(&s, None);
    let bad = manifest(&s, json!([{"name": "broken"}]));
    let absent = s.p("absent.json");
    let ok = manifest(&s, json!([arxiv_entry()]));
    let o = mcp(&s, &["--check", "--manifest", ok.to_str().unwrap(), "--config", absent.to_str().unwrap()]);
    assert_eq!((out(&o), code(&o)), ("MISSING : arxiv — Papers.\n".into(), 1));
    let bad_m = bad.to_str().unwrap();
    let c = s.p("c.json");
    fs::write(&c, "{broken").unwrap();
    let o = mcp(&s, &["--manifest", bad_m, "--config", c.to_str().unwrap()]);
    assert!(out(&o).starts_with(&format!("ABORT (nothing changed): cannot parse {}: ", c.display())), "{}", out(&o));
    assert_eq!(out(&o).lines().count(), 1);
    assert_eq!(code(&o), 2);
    for text in ["{\"mcpServers\": []}", "[\"not\", \"a\", \"dict\"]"] {
        fs::write(&c, text).unwrap();
        let o = mcp(&s, &["--manifest", bad_m, "--config", c.to_str().unwrap()]);
        assert_eq!(out(&o), format!("ABORT (nothing changed): unexpected structure in {}: mcpServers must be an object\n", c.display()));
        assert_eq!(code(&o), 2);
    }
    assert!(!s.p("claude.log").exists());
}

#[test]
fn sync_ms_05() {
    let s = Sandbox::new();
    let m = manifest(&s, json!([{"name": "fetch", "command": "uvx", "args": ["mcp-server-fetch@2026.6.4"]}, arxiv_entry()]));
    let c = config(&s, &json!({"mcpServers": {"fetch": {"type": "stdio", "command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {}}, "somethingelse": {"command": "x"}}}));
    let o = mcp(&s, &["--check", "--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(out(&o), "present : fetch\nextra   : somethingelse (not in manifest; left alone)\nMISSING : arxiv — Papers.\n");
    assert_eq!(code(&o), 1);
}

#[test]
fn sync_ms_06() {
    let srv = |v: Value| mcp_sync::resolve(&json!({"servers": [v]}), Path::new("/r"), None, Path::new("/e")).unwrap().remove(0);
    let fetch = srv(fetch_entry());
    let d = mcp_sync::drift(&json!({"command": "uvx", "args": ["mcp-server-fetch@1.0.0"], "env": {"PYTHONIOENCODING": "utf-8"}}), &fetch);
    assert_eq!(d.as_deref(), Some("args: config=['mcp-server-fetch@1.0.0'] manifest=['mcp-server-fetch@2026.6.4']"));
    assert_eq!(mcp_sync::drift(&json!({"command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8", "UNRELATED": "1"}}), &fetch), None);
    assert_eq!(mcp_sync::drift(&json!({"args": []}), &fetch).as_deref(), Some("command: config=null manifest='uvx'"));
    assert_eq!(mcp_sync::drift(&json!({"command": "uvx", "args": ["mcp-server-fetch@2026.6.4"]}), &fetch).as_deref(), Some("env[PYTHONIOENCODING]: config=null manifest='utf-8'"));
    let web = srv(json!({"name": "w", "url": "https://a/mcp", "command": "ignored"}));
    assert_eq!(mcp_sync::drift(&json!({"url": "https://b/mcp", "command": "other"}), &web).as_deref(), Some("url: config='https://b/mcp' manifest='https://a/mcp'"));
    // the two report lines
    let s = Sandbox::new();
    let m = manifest(&s, json!([fetch_entry()]));
    let c = config(&s, &json!({"mcpServers": {"fetch": {"command": "uvx", "args": ["mcp-server-fetch@1.0.0"], "env": {"PYTHONIOENCODING": "utf-8"}}}}));
    let o = mcp(&s, &["--check", "--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(
        out(&o),
        "DRIFTED : fetch — args: config=['mcp-server-fetch@1.0.0'] manifest=['mcp-server-fetch@2026.6.4']\n  to fix drift manually: claude mcp remove --scope user fetch && re-run this sync\n"
    );
}

#[test]
fn sync_ms_07() {
    let s = Sandbox::new();
    recorder(&s, None);
    let m = manifest(&s, json!([fetch_entry()]));
    let empty = config(&s, &json!({"mcpServers": {}}));
    assert_eq!(code(&mcp(&s, &["--check", "--manifest", m.to_str().unwrap(), "--config", empty.to_str().unwrap()])), 1);
    let good = config(&s, &json!({"mcpServers": {"fetch": {"command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8"}}}}));
    assert_eq!(code(&mcp(&s, &["--check", "--manifest", m.to_str().unwrap(), "--config", good.to_str().unwrap()])), 0);
    assert!(!s.p("claude.log").exists(), "check never runs claude");
}

#[test]
fn sync_ms_08() {
    let s = Sandbox::new();
    let log = recorder(&s, None);
    let m = manifest(&s, json!([fetch_entry(), {"name": "web", "url": "https://example.com/mcp", "why": "W."}, {"name": "e2", "command": "c", "env": {"B": "2", "A": "1"}}]));
    let c = config(&s, &json!({"mcpServers": {}}));
    let before = fs::read(&c).unwrap();
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("register: claude mcp add --scope user fetch --env PYTHONIOENCODING=utf-8 -- uvx mcp-server-fetch@2026.6.4\n"));
    assert!(out(&o).contains("register: claude mcp add --scope user --transport http web https://example.com/mcp\n"));
    assert!(out(&o).ends_with("register: claude mcp add --scope user e2 --env B=2 --env A=1 -- c\n"));
    let claude = s.p("bin/claude").display().to_string();
    let calls: Vec<String> = fs::read_to_string(log).unwrap().lines().map(String::from).collect();
    assert_eq!(
        calls,
        [
            format!("{claude}|mcp|add|--scope|user|fetch|--env|PYTHONIOENCODING=utf-8|--|uvx|mcp-server-fetch@2026.6.4"),
            format!("{claude}|mcp|add|--scope|user|--transport|http|web|https://example.com/mcp"),
            format!("{claude}|mcp|add|--scope|user|e2|--env|B=2|--env|A=1|--|c"),
        ]
    );
    assert_eq!(fs::read(&c).unwrap(), before);
}

#[test]
fn sync_ms_09() {
    assert_eq!(mcp_sync::quote_word("Az09@%+=:,./_-"), "Az09@%+=:,./_-");
    assert_eq!(mcp_sync::quote_word(""), "''");
    assert_eq!(mcp_sync::quote_word("a b"), "'a b'");
    assert_eq!(mcp_sync::quote_word("it's"), "'it'\"'\"'s'");
    assert_eq!(mcp_sync::quote_word("$HOME"), "'$HOME'");
    let printed = mcp_sync::printed(&["mcp".into(), "--env".into(), "K=a b".into(), String::new()]);
    assert_eq!(printed, "claude mcp --env 'K=a b' ''");
}

#[test]
fn sync_ms_10() {
    let s = Sandbox::new();
    let m = manifest(&s, json!([fetch_entry(), arxiv_entry()]));
    let c = config(&s, &json!({}));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    assert_eq!(
        out(&o),
        "MISSING : fetch — Web.\nMISSING : arxiv — Papers.\n\
         register: claude mcp add --scope user fetch --env PYTHONIOENCODING=utf-8 -- uvx mcp-server-fetch@2026.6.4\n\
         register: claude mcp add --scope user arxiv -- uvx arxiv-mcp-server@0.5.0\n\
         claude CLI not found on PATH. Run the printed command(s) manually, or install Claude Code. Nothing was registered.\n"
    );
}

#[test]
fn sync_ms_11() {
    let s = Sandbox::new();
    s.stub("claude", "#!/nonexistent/interpreter\n");
    let m = manifest(&s, json!([fetch_entry(), arxiv_entry(), {"name": "third", "command": "t"}]));
    let c = config(&s, &json!({}));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    let text = out(&o);
    let tail: Vec<&str> = text.lines().skip(3).collect();
    assert_eq!(
        tail,
        [
            "register: claude mcp add --scope user fetch --env PYTHONIOENCODING=utf-8 -- uvx mcp-server-fetch@2026.6.4",
            "register: claude mcp add --scope user arxiv -- uvx arxiv-mcp-server@0.5.0",
            "register: claude mcp add --scope user third -- t",
            "claude CLI not found on PATH. Run the printed command(s) manually, or install Claude Code.",
        ]
    );
}

#[test]
fn sync_ms_15() {
    // read and parse problems that are not ABORTs: one 'error:' line on stderr, exit 1, nothing run (D9)
    let s = Sandbox::new();
    recorder(&s, None);
    let ok = manifest(&s, json!([arxiv_entry()]));
    let absent = s.p("absent.json");
    let run = |m: &Path, c: &Path| mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    let locked = s.p("locked.json");
    fs::write(&locked, "{}").unwrap();
    fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let folder = s.p("folder.json");
    fs::create_dir(&folder).unwrap();
    for (c, why) in [(&locked, "Permission denied (os error 13)"), (&folder, "Is a directory (os error 21)")] {
        let o = run(&ok, c);
        assert_eq!((out(&o), err(&o), code(&o)), (String::new(), format!("error: IoError: {why}: {}\n", c.display()), 1));
    }
    let bad = s.p("bad.json");
    fs::write(&bad, "{broken").unwrap();
    let o = run(&bad, &absent);
    assert!(err(&o).starts_with(&format!("error: SyncError: cannot parse {}: ", bad.display())), "{}", err(&o));
    assert_eq!((out(&o), err(&o).lines().count(), code(&o)), (String::new(), 1, 1));
    for text in ["{}", "{\"servers\": {}}", "[]"] {
        fs::write(&bad, text).unwrap();
        let o = run(&bad, &absent);
        assert_eq!((out(&o), err(&o), code(&o)), (String::new(), format!("error: SyncError: {}: the manifest has no \"servers\" list\n", bad.display()), 1), "{text}");
    }
    // a bad config is still read first
    let o = run(&bad, &locked);
    assert!(err(&o).starts_with("error: IoError: Permission denied"));
    fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    assert!(!s.p("claude.log").exists());
}

#[test]
fn sync_ms_16() {
    // starting claude fails for a reason other than not-found: the run stops at that server, exit 1 (D10)
    let s = Sandbox::new();
    s.stub("claude", "#!/etc/hosts\n");
    let m = manifest(&s, json!([fetch_entry(), arxiv_entry()]));
    let c = config(&s, &json!({}));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    let text = out(&o);
    assert!(text.ends_with("register: claude mcp add --scope user fetch --env PYTHONIOENCODING=utf-8 -- uvx mcp-server-fetch@2026.6.4\n"), "{text}");
    assert!(!text.contains("arxiv-mcp-server@0.5.0\n") && !text.contains("FAILED"));
    let claude = s.p("bin/claude");
    assert_eq!(err(&o), format!("error: IoError: Permission denied (os error 13): {}\n", claude.display()));
}

#[test]
fn sync_ms_12() {
    let s = Sandbox::new();
    let log = recorder(&s, Some(("arxiv", 4)));
    let m = manifest(&s, json!([arxiv_entry(), fetch_entry()]));
    let c = config(&s, &json!({}));
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    let text = out(&o);
    assert!(text.contains("register: claude mcp add --scope user arxiv -- uvx arxiv-mcp-server@0.5.0\n  FAILED (exit 4) — continuing with the rest\nregister: claude mcp add --scope user fetch"));
    assert_eq!(fs::read_to_string(log).unwrap().lines().count(), 2, "later servers still registered");
    assert_eq!(code(&o), 1);
}

#[test]
fn sync_ms_13() {
    let s = Sandbox::new();
    let log = recorder(&s, None);
    let m = manifest(&s, json!([{"name": "fetch", "command": "uvx", "args": ["DIFFERENT@9.9.9"]}]));
    let c = config(&s, &json!({"mcpServers": {"fetch": {"command": "uvx", "args": ["mcp-server-fetch@2026.6.4"]}}}));
    let before = fs::read(&c).unwrap();
    let o = mcp(&s, &["--manifest", m.to_str().unwrap(), "--config", c.to_str().unwrap()]);
    assert!(out(&o).starts_with("DRIFTED : fetch — args: "));
    assert_eq!(code(&o), 0);
    assert!(!log.exists());
    assert_eq!(fs::read(&c).unwrap(), before);
}

#[test]
fn sync_ms_14() {
    let s = Sandbox::new();
    let missing = s.p("nowhere/mcp-servers.json");
    let o = mcp(&s, &["--manifest", missing.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains(&format!("manifest not found at {} — pass --manifest pointing at your checkout's mcp-servers.json", missing.display())));
    // defaults: built-in manifest, ENIGMA_VAULT_PATH, ~/.claude.json (all inside the sandbox)
    let resolved = |v: &str| {
        json!({"enigma": {"command": exe(), "args": [], "env": {"ENIGMA_VAULT_PATH": v}},
               "fetch": {"command": "uvx", "args": ["mcp-server-fetch@2026.6.4"], "env": {"PYTHONIOENCODING": "utf-8"}},
               "arxiv": {"command": "uvx", "args": ["arxiv-mcp-server@0.5.0"]}})
    };
    fs::write(s.p("home/.claude.json"), json!({"mcpServers": resolved("/v")}).to_string()).unwrap();
    let o = s.run(&["mcp-sync", "--check"], b"", &[("ENIGMA_VAULT_PATH", "/v")]);
    assert_eq!((out(&o), code(&o)), ("present : enigma\npresent : fetch\npresent : arxiv\n".into(), 0));
    let o = s.go(&["mcp-sync", "--check", "--vault", "/other"]);
    assert!(out(&o).contains("DRIFTED : enigma — env[ENIGMA_VAULT_PATH]: config='/v' manifest='/other'\n"), "{}", out(&o));
    // nothing missing and not --check: exit 0 without claude on PATH
    let o = s.run(&["mcp-sync"], b"", &[("ENIGMA_VAULT_PATH", "/v")]);
    assert_eq!(code(&o), 0);
}

// ---------------------------------------------------------------- provisioning

/// Serves synthetic assets; records every URL asked for.
struct FakeFetch {
    assets: HashMap<String, Vec<u8>>,
    asked: RefCell<Vec<String>>,
}

impl Fetch for FakeFetch {
    fn fetch(&self, url: &str) -> Result<Option<Vec<u8>>> {
        self.asked.borrow_mut().push(url.to_string());
        Ok(self.assets.get(url).cloned())
    }
}

fn sha(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn asset(id: &str, file: &str) -> Vec<u8> {
    match file {
        "manifest.json" => json!({"id": id, "version": "1.0.0"}).to_string().into_bytes(),
        _ => format!("{file} of {id}").into_bytes(),
    }
}

/// A plugin whose lock matches the synthetic assets, and a fetcher serving them.
fn synthetic(ids: &[(&str, bool)]) -> (Vec<Plugin>, FakeFetch) {
    let mut lock = Vec::new();
    let mut assets = HashMap::new();
    for (id, css) in ids {
        let files: &[&str] = if *css { &["manifest.json", "main.js", "styles.css"] } else { &["manifest.json", "main.js"] };
        let p = Plugin { id: id.to_string(), repo: format!("o/{id}"), version: "1.0.0".into(), sha256: files.iter().map(|f| (f.to_string(), sha(&asset(id, f)))).collect() };
        for f in ["manifest.json", "main.js", "styles.css"] {
            assets.insert(provision::asset_url(&p, f), asset(id, f));
        }
        lock.push(p);
    }
    (lock, FakeFetch { assets, asked: RefCell::new(vec![]) })
}

fn dataview() -> (Plugin, FakeFetch) {
    let (mut lock, f) = synthetic(&[("dataview", true)]);
    (lock.remove(0), f)
}

#[test]
fn sync_pv_01() {
    let lock = provision::bundled_lock();
    let ids: Vec<(&str, &str, &str)> = lock.iter().map(|p| (p.id.as_str(), p.repo.as_str(), p.version.as_str())).collect();
    assert_eq!(
        ids,
        [
            ("smart-connections", "brianpetro/obsidian-smart-connections", "4.5.3"),
            ("obsidian-tasks-plugin", "obsidian-tasks-group/obsidian-tasks", "8.2.2"),
            ("obsidian-livesync", "vrtmrz/obsidian-livesync", "0.25.79"),
            ("text-extractor", "scambier/obsidian-text-extractor", "0.7.0"),
            ("dataview", "blacksmithgu/obsidian-dataview", "0.5.68"),
            ("omnisearch", "scambier/obsidian-omnisearch", "1.29.3"),
            ("tag-wrangler", "pjeby/tag-wrangler", "0.6.4"),
            ("obsidian-kanban", "mgmeyers/obsidian-kanban", "2.0.51"),
            ("table-editor-obsidian", "tgrosinger/advanced-tables-obsidian", "0.23.2"),
            ("obsidian-importer", "obsidianmd/obsidian-importer", "1.8.12"),
            ("extended-graph", "ElsaTam/obsidian-extended-graph", "2.7.6"),
            ("graph-styler", "moonweave/obsidian-graph-styler", "0.1.5"),
            ("translucent-bg", "mellowmarsh-git/Obsidian-Translucent-BG", "1.1.2"),
            ("obsidian-style-settings", "mgmeyers/obsidian-style-settings", "1.0.9"),
        ]
    );
    for p in &lock {
        let names: Vec<&str> = p.sha256.iter().map(|(n, _)| n.as_str()).collect();
        let css = !["text-extractor", "tag-wrangler"].contains(&p.id.as_str());
        assert_eq!(names, if css { vec!["manifest.json", "main.js", "styles.css"] } else { vec!["manifest.json", "main.js"] }, "{}", p.id);
    }
    assert_eq!(lock.iter().find(|p| p.id == "dataview").unwrap().sha256[1].1, "794e9eaede73920bb8d54b0eda4f5de2182d698cc638774500f24f14bcd4da0b");
}

#[test]
fn sync_pv_02() {
    let d = "a".repeat(64);
    let msg = |v: Value| provision::validate(&v).unwrap_err();
    let e = msg(json!({"id": "x", "repo": "a/b"}));
    assert_eq!((e.kind, e.msg.as_str()), (Kind::ProvisionError, "plugins.json entry missing 'version': {'id': 'x', 'repo': 'a/b'}"));
    assert_eq!(msg(json!({"id": "", "repo": "a/b", "version": "1"})).msg, "plugins.json entry missing 'id': {'id': '', 'repo': 'a/b', 'version': '1'}");
    assert_eq!(msg(json!({"id": "x", "repo": "a/b", "version": "1"})).msg, "plugins.json entry missing 'sha256' lock: {'id': 'x', 'repo': 'a/b', 'version': '1'}");
    let base = |sha: Value| json!({"id": "x", "repo": "a/b", "version": "1", "sha256": sha});
    assert_eq!(msg(base(json!({"manifest.json": d}))).msg, "x: sha256 lock missing 'main.js'");
    assert_eq!(msg(base(json!({"manifest.json": d, "main.js": "ABC"}))).msg, "x: sha256 for 'main.js' is not 64 lowercase hex chars");
    assert_eq!(msg(base(json!({"manifest.json": d, "main.js": d.to_uppercase()}))).msg, "x: sha256 for 'main.js' is not 64 lowercase hex chars");
    assert_eq!(msg(base(json!({"manifest.json": d, "main.js": d, "data.json": d}))).msg, "x: sha256 lock names unknown asset 'data.json'");
    assert!(provision::validate(&base(json!({"manifest.json": d, "main.js": d, "styles.css": d}))).is_ok());
}

#[test]
fn sync_pv_03() {
    let v = tempfile::tempdir().unwrap();
    let (p, f) = dataview();
    assert_eq!(provision::install_plugin(v.path(), &p, &f).unwrap(), "installed");
    let dir = v.path().join(".obsidian/plugins/dataview");
    for file in ["manifest.json", "main.js", "styles.css"] {
        assert_eq!(fs::read(dir.join(file)).unwrap(), asset("dataview", file));
    }
}

#[test]
fn sync_pv_04() {
    let v = tempfile::tempdir().unwrap();
    let (p, f) = dataview();
    provision::install_plugin(v.path(), &p, &f).unwrap();
    f.asked.borrow_mut().clear();
    assert_eq!(provision::install_plugin(v.path(), &p, &f).unwrap(), "ok");
    assert!(f.asked.borrow().is_empty());
    assert_eq!(provision::installed_version(v.path(), "dataview").as_deref(), Some("1.0.0"));
    fs::write(v.path().join(".obsidian/plugins/dataview/manifest.json"), "{bad").unwrap();
    assert_eq!(provision::installed_version(v.path(), "dataview"), None);
    assert_eq!(provision::install_plugin(v.path(), &p, &f).unwrap(), "installed");
}

#[test]
fn sync_pv_05() {
    let v = tempfile::tempdir().unwrap();
    let (mut lock, f) = synthetic(&[("tag-wrangler", false)]);
    provision::install_plugin(v.path(), &lock.remove(0), &f).unwrap();
    assert!(!f.asked.borrow().iter().any(|u| u.ends_with("styles.css")));
    assert!(!v.path().join(".obsidian/plugins/tag-wrangler/styles.css").exists());
}

#[test]
fn sync_pv_06() {
    let refuse = |f: FakeFetch, p: &Plugin| {
        let v = tempfile::tempdir().unwrap();
        let e = provision::install_plugin(v.path(), p, &f).unwrap_err();
        assert_eq!(e.kind, Kind::ProvisionError);
        assert!(!v.path().join(".obsidian/plugins/dataview").exists());
        (e.msg, f.asked.into_inner())
    };
    for file in ["manifest.json", "main.js", "styles.css"] {
        let (p, mut f) = dataview();
        let url = provision::asset_url(&p, file);
        f.assets.insert(url, b"tampered".to_vec());
        let want = p.sha256.iter().find(|(n, _)| n == file).unwrap().1.clone();
        let (msg, asked) = refuse(f, &p);
        assert_eq!(msg, format!("dataview: {file} sha256 {} does not match lock {want} — refusing", sha(b"tampered")));
        assert!(asked.last().unwrap().ends_with(file), "the first failure stops");
    }
    let (p, mut f) = dataview();
    f.assets.remove(&provision::asset_url(&p, "main.js"));
    assert_eq!(refuse(f, &p).0, "dataview: no main.js in release 1.0.0 of o/dataview");
    // the declared id is checked only after every asset verified
    let evil = json!({"id": "evil-other-plugin"}).to_string().into_bytes();
    let (mut p, mut f) = dataview();
    p.sha256[0].1 = sha(&evil);
    f.assets.insert(provision::asset_url(&p, "manifest.json"), evil);
    let (msg, asked) = refuse(f, &p);
    assert_eq!(msg, "dataview: release manifest declares id 'evil-other-plugin' — refusing");
    assert_eq!(asked.len(), 3);
}

#[test]
fn sync_pv_07() {
    let v = tempfile::tempdir().unwrap();
    let path = v.path().join(".obsidian/community-plugins.json");
    let ids = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(provision::enable_plugins(v.path(), &ids(&["dataview", "omnisearch"])).unwrap(), ["dataview", "omnisearch"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "[\n  \"dataview\",\n  \"omnisearch\"\n]");
    fs::write(&path, "[\"users-own-plugin\", \"dataview\"]").unwrap();
    assert_eq!(provision::enable_plugins(v.path(), &ids(&["dataview", "omnisearch"])).unwrap(), ["omnisearch"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "[\n  \"users-own-plugin\",\n  \"dataview\",\n  \"omnisearch\"\n]");
    let before = filetime(&path);
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(provision::enable_plugins(v.path(), &ids(&["dataview"])).unwrap().is_empty());
    assert_eq!(filetime(&path), before);
}

#[test]
fn sync_pv_08() {
    let v = tempfile::tempdir().unwrap();
    assert_eq!(provision::overlays(v.path()).unwrap(), [".smart-env/smart_env.json"]);
    let want = "{\n  \"is_obsidian_vault\": true,\n  \"smart_sources\": {\n    \"min_chars\": 200,\n    \"embed_model\": {\n      \"adapter\": \"transformers\",\n      \"transformers\": {\n        \"model_key\": \"TaylorAI/bge-micro-v2\"\n      }\n    }\n  },\n  \"models\": {\n    \"embedding_platform\": \"transformers\"\n  }\n}\n";
    assert_eq!(fs::read_to_string(v.path().join(".smart-env/smart_env.json")).unwrap(), want);
    assert_eq!(want.len(), 277);
    let v = tempfile::tempdir().unwrap();
    write_file(v.path(), ".smart-env/smart_env.json", "{\"user\": \"tweaked\"}");
    assert!(provision::overlays(v.path()).unwrap().is_empty());
    assert_eq!(fs::read_to_string(v.path().join(".smart-env/smart_env.json")).unwrap(), "{\"user\": \"tweaked\"}");
    // no per-plugin overlays ship today: nothing under .obsidian/plugins
    assert!(!v.path().join(".obsidian").exists());
}

fn fourteen() -> (Vec<Plugin>, FakeFetch) {
    let lock = provision::bundled_lock();
    let ids: Vec<(String, bool)> = lock.iter().map(|p| (p.id.clone(), p.sha256.len() == 3)).collect();
    let refs: Vec<(&str, bool)> = ids.iter().map(|(i, c)| (i.as_str(), *c)).collect();
    synthetic(&refs)
}

#[test]
fn sync_pv_09() {
    let v = tempfile::tempdir().unwrap();
    let (lock, f) = fourteen();
    let r = provision::provision(v.path(), &lock, &f).unwrap();
    assert_eq!(r.as_object().unwrap().keys().collect::<Vec<_>>(), ["plugins", "errors", "enabled", "overlays", "conventions"]);
    let ids: Vec<String> = lock.iter().map(|p| p.id.clone()).collect();
    assert_eq!(r["plugins"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ids);
    assert!(r["plugins"].as_object().unwrap().values().all(|s| s == "installed"));
    assert_eq!(r["errors"], json!({}));
    assert_eq!(r["enabled"], json!(ids));
    assert_eq!(r["overlays"], json!([".smart-env/smart_env.json"]));
    assert_eq!(r["conventions"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect::<Vec<_>>().join(", "), ALL_CREATED);
    assert!(v.path().join("Claude/README.md").is_file());
}

#[test]
fn sync_pv_10() {
    let v = tempfile::tempdir().unwrap();
    let (lock, f) = fourteen();
    provision::provision(v.path(), &lock, &f).unwrap();
    f.asked.borrow_mut().clear();
    let r = provision::provision(v.path(), &lock, &f).unwrap();
    assert!(r["plugins"].as_object().unwrap().values().all(|s| s == "ok"));
    assert!(f.asked.borrow().is_empty());
    assert_eq!((r["enabled"].clone(), r["overlays"].clone(), r["conventions"].clone()), (json!([]), json!([]), json!([])));
}

#[test]
fn sync_pv_11() {
    let v = tempfile::tempdir().unwrap();
    let (lock, mut f) = fourteen();
    let dv = lock.iter().find(|p| p.id == "dataview").unwrap();
    f.assets.remove(&provision::asset_url(dv, "main.js"));
    let r = provision::provision(v.path(), &lock, &f).unwrap();
    assert!(r["errors"]["dataview"].as_str().unwrap().contains("main.js"));
    assert_eq!(r["plugins"]["omnisearch"], "installed");
    let enabled = fs::read_to_string(v.path().join(".obsidian/community-plugins.json")).unwrap();
    assert!(!enabled.contains("\"dataview\""));

    let v = tempfile::tempdir().unwrap();
    let (lock, mut f) = fourteen();
    let tw = lock.iter().find(|p| p.id == "tag-wrangler").unwrap();
    f.assets.get_mut(&provision::asset_url(tw, "main.js")).unwrap().extend_from_slice(b"extra");
    let r = provision::provision(v.path(), &lock, &f).unwrap();
    assert!(r["errors"]["tag-wrangler"].as_str().unwrap().contains("does not match lock"));
    assert!(!v.path().join(".obsidian/plugins/tag-wrangler").exists());
    let enabled: Value = serde_json::from_slice(&fs::read(v.path().join(".obsidian/community-plugins.json")).unwrap()).unwrap();
    assert_eq!(enabled.as_array().unwrap().len(), 13);
}

#[test]
fn sync_pv_12() {
    let s = Sandbox::new();
    let missing = s.p("no-vault");
    let (lock, f) = fourteen();
    let e = provision::provision(&missing, &lock, &f).unwrap_err();
    assert_eq!((e.kind, e.msg), (Kind::ProvisionError, format!("Vault root does not exist: {}", missing.display())));
    assert!(!missing.exists() && f.asked.borrow().is_empty());
    let v = tempfile::tempdir().unwrap();
    provision::install_plugin(v.path(), &lock[4], &f).unwrap(); // dataview at its pin
    write_file(v.path(), ".obsidian/plugins/omnisearch/manifest.json", "{\"version\": \"0.1\"}");
    let c = provision::check(v.path(), &lock).unwrap();
    assert_eq!(c.as_object().unwrap().keys().collect::<Vec<_>>(), lock.iter().map(|p| &p.id).collect::<Vec<_>>());
    assert_eq!(c["dataview"], json!({"pinned": "1.0.0", "installed": "1.0.0", "status": "ok"}));
    assert_eq!(c["omnisearch"], json!({"pinned": "1.0.0", "installed": "0.1", "status": "drift"}));
    assert_eq!(c["tag-wrangler"], json!({"pinned": "1.0.0", "installed": null, "status": "missing"}));
}

#[test]
fn sync_pv_13() {
    let s = Sandbox::new();
    let v = tempfile::tempdir().unwrap();
    let vp = v.path().to_str().unwrap();
    let o = s.go(&["provision", vp, "--check"]);
    assert_eq!(code(&o), 0);
    let report: Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(out(&o), format!("{}\n", enigma_mcp::json::indented(&report)));
    assert_eq!(report["dataview"], json!({"pinned": "0.5.68", "installed": null, "status": "missing"}));
    assert_eq!(files_under(v.path()), Vec::<String>::new(), "--check changes nothing");
    // every plugin already at its pin: the real run fetches nothing (no network in tests)
    for p in provision::bundled_lock() {
        write_file(v.path(), &format!(".obsidian/plugins/{}/manifest.json", p.id), &json!({"id": p.id, "version": p.version}).to_string());
    }
    let o = s.go(&["provision", vp]);
    assert_eq!(code(&o), 0);
    let text = out(&o);
    let (json_part, rest) = text.split_at(text.find("\n\n").unwrap() + 1);
    let report: Value = serde_json::from_str(json_part).unwrap();
    assert!(report["plugins"].as_object().unwrap().values().all(|s| s == "ok"));
    assert_eq!(rest, format!("\n{}\n", provision::NEXT_STEPS));
    assert_eq!(
        provision::NEXT_STEPS,
        "Provisioning done. Remaining human steps:\n 1. Open the vault in Obsidian once and turn OFF Restricted Mode\n    (Settings -> Community plugins) so the installed plugins load.\n 2. Complete LiveSync setup via its Setup-URI flow — server URI and the\n    E2E passphrase live in your password manager, never in this repo.\n 3. Register the vault with the MCP server (enigma-mcp mcp-sync) and run the\n    index_brain tool to build the semantic graph.\n"
    );
    let with_errors = provision::epilogue(&json!({"errors": {"x": "boom"}}));
    assert_eq!(with_errors, format!("\nWARNING: some plugins failed — re-run after checking pins/network.\n\n{}\n", provision::NEXT_STEPS));
    let missing = s.p("gone");
    let o = s.go(&["provision", missing.to_str().unwrap()]);
    assert_eq!((err(&o), code(&o)), (format!("error: Vault root does not exist: {}\n", missing.display()), 2));
    // --check makes no existence check: a missing vault or a plain file reports all 14 missing, exit 0
    let file = s.p("plain.txt");
    fs::write(&file, "x").unwrap();
    for vault in [&missing, &file] {
        let o = s.go(&["provision", vault.to_str().unwrap(), "--check"]);
        assert_eq!((code(&o), err(&o)), (0, String::new()));
        let report: Value = serde_json::from_str(&out(&o)).unwrap();
        let all = report.as_object().unwrap();
        assert_eq!(all.len(), 14);
        assert!(all.values().all(|p| p["status"] == "missing" && p["installed"].is_null()));
    }
    assert!(!missing.exists());
}

// ---------------------------------------------------------------- platform

#[test]
fn sync_pl_01() {
    use enigma_mcp::faults::{self, Fault};
    let d = tempfile::tempdir().unwrap();
    let v = enigma_mcp::vault::Vault::open(d.path()).unwrap();
    let write = |rel: &str| v.write(rel, "body", false, false);
    faults::reset();
    for (errno, name) in [(1, "EPERM"), (45, "ENOTSUP"), (18, "EXDEV"), (31, "EMLINK")] {
        faults::arm("publish", Fault::Os(errno), 1);
        let rel = format!("Claude/{name}.md");
        write(&rel).unwrap();
        assert_eq!(fs::read_to_string(d.path().join(&rel)).unwrap(), "body", "{name}: exclusive-create fallback");
    }
    faults::reset();
    faults::arm("publish", Fault::Os(1), 5); // persistent EPERM: no retry, straight to the fallback
    write("Claude/EPERM-always.md").unwrap();
    assert_eq!(faults::calls("publish"), 1);
    faults::reset();
    faults::arm("publish", Fault::Os(13), 5);
    let e = write("Claude/EACCES.md").unwrap_err();
    assert_eq!(e.kind, Kind::IoError, "EACCES is reported after the retries");
    faults::reset();
    assert_eq!(write("Claude/EPERM.md").unwrap_err().kind, Kind::NoteExists);
}

#[test]
fn sync_pl_02() {
    let s = Sandbox::new();
    let help = String::from_utf8(Command::new(bin()).arg("--help").output().unwrap().stdout).unwrap();
    for sub in ["index-nightly", "install-conventions", "mcp-sync", "skill-sync", "hook-sync", "hook", "provision"] {
        assert!(help.contains(sub), "{sub}");
    }
    // text files written outside notes use LF only
    s.go(&["hook-sync", "--install"]);
    let v = tempfile::tempdir().unwrap();
    provision::enable_plugins(v.path(), &["a".into()]).unwrap();
    provision::overlays(v.path()).unwrap();
    let plist = enigma_mcp::nightly::render(enigma_mcp::nightly::INDEX_PLIST, Path::new("/b"), Path::new("/v"), Path::new("/h"));
    for text in [fs::read(s.settings()).unwrap(), fs::read(v.path().join(".obsidian/community-plugins.json")).unwrap(), fs::read(v.path().join(".smart-env/smart_env.json")).unwrap(), plist.into_bytes()] {
        assert!(!text.contains(&b'\r'));
    }
}
