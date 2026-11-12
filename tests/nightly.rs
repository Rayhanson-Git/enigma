//! acceptance/sync.yaml NT-01..11: the nightly index runner. Cases run one at a time because the runner
//! skips while another indexer process is running. HOME, TMPDIR and the state dir are temp dirs; PATH
//! holds only stub extractors plus /usr/bin:/bin. Rounds load the real embedding model, read-only from
//! the existing cache (HF_HUB_CACHE), as SV-19 does.
mod common;
use common::*;
use enigma_mcp::nightly::{INDEX_PLIST, LIBRARIAN_PLIST, render};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tempfile::TempDir;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn model_cache() -> String {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    var("HF_HUB_CACHE")
        .or_else(|| var("HF_HOME").map(|h| format!("{h}/hub")))
        .unwrap_or_else(|| format!("{}/.cache/huggingface/hub", std::env::var("HOME").unwrap_or_default()))
}

struct Sandbox {
    root: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let root = tempfile::tempdir().unwrap();
        for d in ["home", "tmp", "state", "bin", "vault"] {
            fs::create_dir_all(root.path().join(d)).unwrap();
        }
        write_file(&root.path().join("vault"), "Note.md", "# Note\n\nOne note to index.\n");
        let s = Sandbox { root };
        s.stub("claude", &extractor_stub(&s.p("claude.log"), ""));
        s
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    fn lock(&self) -> PathBuf {
        self.p("tmp/enigma-index.lock")
    }
    fn stub(&self, name: &str, script: &str) {
        let p = self.p("bin").join(name);
        fs::write(&p, script).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    fn cmd(&self, extra: &[&str], env: &[(&str, &str)]) -> Command {
        let mut c = Command::new(bin());
        for (k, _) in std::env::vars() {
            if k.starts_with("ENIGMA_") || k.starts_with("CLAUDE") || k.starts_with("HF_") {
                c.env_remove(k);
            }
        }
        c.arg("index-nightly")
            .arg(self.p("vault"))
            .args(extra)
            .env("HOME", self.p("home"))
            .env("TMPDIR", self.p("tmp"))
            .env("ENIGMA_STATE_DIR", self.p("state"))
            .env("HF_HUB_CACHE", model_cache())
            .env("PATH", format!("{}:/usr/bin:/bin", self.p("bin").display()))
            .envs(env.iter().copied())
            .stdin(Stdio::null());
        c
    }
    fn run(&self, extra: &[&str], env: &[(&str, &str)]) -> Output {
        self.cmd(extra, env).output().unwrap()
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.p("claude.log")).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
    }
}

/// An extractor stub: logs its environment, runs `extra`, prints an empty extraction.
fn extractor_stub(log: &Path, extra: &str) -> String {
    format!(
        "#!/bin/sh\ninput=$(cat)\nprintf 'VAULT=%s EXTRACTOR=%s NOTOOLS=%s HF=%s STATE=%s\\n' \"$ENIGMA_VAULT_PATH\" \"$ENIGMA_EXTRACTOR\" \"$ENIGMA_EXTRACTOR_NO_TOOLS\" \"${{HF_HUB_OFFLINE-unset}}\" \"$ENIGMA_STATE_DIR\" >> '{}'\n{extra}\necho '{{\"entities\": [], \"relations\": []}}'\n",
        log.display()
    )
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn rounds(text: &str) -> usize {
    text.matches("\"remaining\": ").count()
}

/// A long-lived process whose command line contains `marker`.
fn holder(marker: &str) -> Child {
    Command::new("sh").args(["-c", "sleep 60; true", marker]).spawn().unwrap()
}

fn stamp_line(line: &str, what: &str) -> bool {
    let re = regex::Regex::new(&format!(r"^\[\d{{4}}-\d{{2}}-\d{{2}} \d{{2}}:\d{{2}}:\d{{2}}\] index-nightly {what}$")).unwrap();
    re.is_match(line)
}

#[test]
fn nightly_nt_01() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    let vault = s.p("vault").display().to_string();
    let o = s.run(&[], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let state = s.p("state").display().to_string();
    assert_eq!(s.calls(), [format!("VAULT={vault} EXTRACTOR=claude NOTOOLS=1 HF=unset STATE={state}")]);
    for (env, want) in [
        (vec![("HF_HUB_OFFLINE", "1")], "EXTRACTOR=claude NOTOOLS=1 HF=1"),
        (vec![("ENIGMA_EXTRACTOR_NO_TOOLS", "0")], "EXTRACTOR=claude NOTOOLS=0 HF=unset"),
        (vec![("ENIGMA_EXTRACTOR", ""), ("ENIGMA_EXTRACTOR_NO_TOOLS", "")], "EXTRACTOR=claude NOTOOLS=1 HF=unset"),
        (vec![("ENIGMA_EXTRACTOR", "codex")], "EXTRACTOR=codex NOTOOLS=1 HF=unset"),
    ] {
        let s = Sandbox::new();
        s.stub("codex", &extractor_stub(&s.p("claude.log"), ""));
        s.run(&[], &env);
        assert_eq!(s.calls().len(), 1, "{env:?}");
        assert!(s.calls()[0].contains(want), "{env:?}: {}", s.calls()[0]);
    }
    // a state-dir argument becomes ENIGMA_STATE_DIR for the run
    let s = Sandbox::new();
    let other = s.p("other-state");
    s.run(&[other.to_str().unwrap()], &[]);
    assert!(s.calls()[0].ends_with(&format!("STATE={}", other.display())));
    assert!(other.join("manifest.json").is_file());
}

#[test]
fn nightly_nt_02() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    let o = s.run(&[], &[]);
    let text = out(&o);
    let lines: Vec<&str> = text.lines().collect();
    assert!(stamp_line(lines[0], "start") && stamp_line(lines.last().unwrap(), "done"), "{text}");
    assert_eq!((rounds(&text), o.status.code()), (1, Some(0)));
    // 26 notes the stub keeps editing: 25 per round never drain, so all 20 rounds run
    let s = Sandbox::new();
    for i in 0..25 {
        write_file(&s.p("vault"), &format!("N{i:02}.md"), "restless\n");
    }
    let touch = "p=$(printf '%s\\n' \"$input\" | sed -n 's/^Note path: //p' | head -1); [ -n \"$p\" ] && echo x >> \"$ENIGMA_VAULT_PATH/$p\"";
    s.stub("claude", &extractor_stub(&s.p("claude.log"), touch));
    let o = s.run(&[], &[]);
    let text = out(&o);
    assert_eq!(rounds(&text), 20, "{text}");
    assert!(!text.contains("\"remaining\": 0"));
    assert!(stamp_line(text.lines().last().unwrap(), "done"));
    assert_eq!(o.status.code(), Some(0));
    // after each round the last 12 lines of its output are printed
    assert!(text.contains("{\n  \"processed\": 25,"));
    // a failed round (LockBusy: a Librarian sweep holds the caretaker lock) prints its last 5 lines, exit 1
    let s = Sandbox::new();
    let _held = enigma_mcp::lock::acquire(&s.p("state")).unwrap();
    let o = s.run(&[], &[]);
    let text = out(&o);
    assert_eq!(o.status.code(), Some(1), "{text}");
    assert!(text.contains("LockBusy"), "{text}");
    assert!(!text.contains("index-nightly done"));
    assert!(!s.lock().exists(), "LOCK removed after a failed round");
}

#[test]
fn nightly_nt_03() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    fs::remove_dir_all(s.p("vault")).unwrap();
    let o = s.run(&[], &[]);
    let lines: Vec<String> = out(&o).lines().map(String::from).collect();
    assert!(stamp_line(&lines[0], "start"));
    assert_eq!(lines[1..], ["cannot read vault (macOS privacy permission for enigma-mcp/launchd?)"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!s.lock().exists());
    assert!(s.calls().is_empty());
}

#[test]
fn nightly_nt_10() {
    let s = Sandbox::new();
    let o = Command::new(bin()).arg("index-nightly").env("TMPDIR", s.p("tmp")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stderr), "usage: enigma-mcp index-nightly <vault> [state-dir]\n");
    assert_eq!((o.stdout.len(), o.status.code()), (0, Some(1)));
    assert!(!s.lock().exists());
}

#[test]
fn nightly_nt_04() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    let mut gone = Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    fs::create_dir(s.lock()).unwrap();
    fs::write(s.lock().join("pid"), format!("{pid}\n")).unwrap();
    let o = s.run(&[], &[]);
    let text = out(&o);
    assert!(text.contains(&format!("removing stale lock {} (pid {pid} is not running)", s.lock().display())), "{text}");
    assert_eq!((rounds(&text), o.status.code()), (1, Some(0)));
    assert!(!s.lock().exists());
}

#[test]
fn nightly_nt_05() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    fs::create_dir(s.lock()).unwrap();
    let o = s.run(&[], &[]);
    let text = out(&o);
    assert!(text.contains(&format!("removing stale lock {} (pid unknown is not running)", s.lock().display())), "{text}");
    assert_eq!(rounds(&text), 1);
}

#[test]
fn nightly_nt_06() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    let mut live = holder("enigma-mcp index-nightly /some/vault");
    fs::create_dir(s.lock()).unwrap();
    fs::write(s.lock().join("pid"), format!("{}\n", live.id())).unwrap();
    let o = s.run(&[], &[]);
    live.kill().unwrap();
    live.wait().unwrap();
    let text = out(&o);
    assert!(text.contains(&format!("another index run (pid {}) holds {}; skipping", live.id(), s.lock().display())), "{text}");
    assert_eq!(o.status.code(), Some(0));
    assert!(s.calls().is_empty());
    assert_eq!(fs::read_to_string(s.lock().join("pid")).unwrap(), format!("{}\n", live.id()));
}

#[test]
fn nightly_nt_07() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // re-creation after reclaiming fails: the lock path is a plain file that cannot be removed as a folder
    let s = Sandbox::new();
    fs::write(s.lock(), "").unwrap();
    let o = s.run(&[], &[]);
    assert!(out(&o).contains(&format!("another run took {}; skipping", s.lock().display())), "{}", out(&o));
    assert_eq!(o.status.code(), Some(0));
    assert!(s.calls().is_empty());
    // a live pid whose command line lacks index-nightly is stale
    let s = Sandbox::new();
    let mut other = holder("unrelated");
    fs::create_dir(s.lock()).unwrap();
    fs::write(s.lock().join("pid"), format!("{}\n", other.id())).unwrap();
    let o = s.run(&[], &[]);
    other.kill().unwrap();
    other.wait().unwrap();
    assert!(out(&o).contains(&format!("(pid {} is not running)", other.id())));
    assert_eq!(rounds(&out(&o)), 1);
}

/// Starts a runner whose extractor stub sleeps `secs`; returns it once the first extractor call is running.
fn interrupted_runner(s: &Sandbox, secs: u32, own_group: bool) -> Child {
    let started = s.p("stub.started");
    s.stub("claude", &extractor_stub(&s.p("claude.log"), &format!("touch '{}'; sleep {secs}", started.display())));
    let mut cmd = s.cmd(&[], &[]);
    if own_group {
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    }
    let runner = cmd.stdout(Stdio::piped()).spawn().unwrap();
    let start = Instant::now();
    while !started.exists() {
        assert!(start.elapsed() < Duration::from_secs(60), "the round never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    runner
}

#[test]
fn nightly_nt_11() {
    use std::os::unix::process::ExitStatusExt;
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for (sig, num) in [("INT", 2), ("TERM", 15), ("HUP", 1)] {
        // signal to the runner alone: the running round finishes, then the runner dies of the signal
        let s = Sandbox::new();
        let runner = interrupted_runner(&s, 2, false);
        assert!(s.lock().is_dir());
        assert_eq!(fs::read_to_string(s.lock().join("pid")).unwrap(), format!("{}\n", runner.id()));
        let start = Instant::now();
        Command::new("kill").args([&format!("-{sig}"), &runner.id().to_string()]).status().unwrap();
        let o = runner.wait_with_output().unwrap();
        assert!(start.elapsed() >= Duration::from_secs(1), "{sig}: the round was not cut short");
        assert_eq!((o.status.code(), o.status.signal()), (None, Some(num)), "{sig}: killed by the signal");
        assert!(!s.lock().exists(), "{sig}: LOCK removed");
        assert_eq!(s.calls().len(), 1, "{sig}: no further extractor call or round");
        assert!(!out(&o).contains("index-nightly done"), "{sig}");
        // signal to the whole process group (Ctrl-C, a supervisor): everything stops at once
        let s = Sandbox::new();
        let runner = interrupted_runner(&s, 30, true);
        let start = Instant::now();
        Command::new("kill").args([&format!("-{sig}"), "--", &format!("-{}", runner.id())]).status().unwrap();
        let o = runner.wait_with_output().unwrap();
        assert!(start.elapsed() < Duration::from_secs(20), "{sig}: the group stopped at once");
        assert_eq!((o.status.code(), o.status.signal()), (None, Some(num)), "{sig}");
        assert!(!s.lock().exists(), "{sig}: LOCK removed");
        assert!(!out(&o).contains("index-nightly done"), "{sig}");
    }
}

#[test]
fn nightly_nt_08() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let s = Sandbox::new();
    let mut busy = holder("enigma-mcp index /elsewhere");
    let o = s.run(&[], &[]);
    busy.kill().unwrap();
    busy.wait().unwrap();
    let text = out(&o);
    assert!(text.lines().any(|l| l == "indexer already running; skipping"), "{text}");
    assert_eq!(o.status.code(), Some(0));
    assert!(s.calls().is_empty());
    assert!(!s.lock().exists());
}

#[test]
fn nightly_nt_09() {
    let (bin, vault, home) = (Path::new("/opt/enigma-mcp"), Path::new("/v"), Path::new("/Users/h"));
    let index = render(INDEX_PLIST, bin, vault, home);
    let librarian = render(LIBRARIAN_PLIST, bin, vault, home);
    for t in [INDEX_PLIST, LIBRARIAN_PLIST] {
        assert!(t.contains("{BIN}") && t.contains("{VAULT}") && t.contains("{HOME}"));
    }
    let path = "<key>PATH</key><string>/Users/h/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>";
    assert!(index.contains("<key>Label</key><string>com.enigma.index</string>"));
    assert!(index.contains("<string>/opt/enigma-mcp</string>\n    <string>index-nightly</string>\n    <string>/v</string>"));
    assert!(index.contains("<key>Hour</key><integer>4</integer><key>Minute</key><integer>30</integer>"));
    assert!(index.contains("<key>StandardOutPath</key><string>/Users/h/.enigma-mcp/index-nightly.log</string>"));
    assert!(index.contains("<key>StandardErrorPath</key><string>/Users/h/.enigma-mcp/index-nightly.log</string>"));
    assert!(index.contains(path));
    assert!(librarian.contains("<key>Label</key><string>com.enigma.librarian</string>"));
    assert!(librarian.contains("<string>/opt/enigma-mcp</string>\n    <string>librarian</string>\n    <string>/v</string>"));
    assert!(librarian.contains("<key>ENIGMA_EXTRACTOR</key><string>claude</string>"));
    assert!(librarian.contains("<key>ENIGMA_EXTRACTOR_NO_TOOLS</key><string>1</string>"));
    assert!(librarian.contains(path));
    assert!(librarian.contains("<key>Hour</key><integer>4</integer><key>Minute</key><integer>0</integer>"));
    assert!(librarian.contains("<key>StandardOutPath</key><string>/Users/h/.enigma-mcp/librarian-task.log</string>"));
    for t in [&index, &librarian] {
        assert!(t.contains("Install only one of"), "only one of the two is installed");
    }
    assert!(librarian.contains("--dry-run") && librarian.contains("--confirm-first-run"));
    assert!(!index.contains("--confirm-first-run"), "index-nightly has no first-run guard");
    for (t, job) in [(&index, "index"), (&librarian, "librarian")] {
        assert!(t.contains(&format!("enigma-mcp schedule install --job {job} VAULT")) && t.contains(&format!("enigma-mcp schedule uninstall --job {job}")));
    }
    let escaped = render(INDEX_PLIST, bin, Path::new("/v/R&D <x>"), home);
    assert!(escaped.contains("<string>/v/R&amp;D &lt;x&gt;</string>"));
}
