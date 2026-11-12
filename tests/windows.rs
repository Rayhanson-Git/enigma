#![cfg(windows)]
//! Windows-only behaviour, including what the POSIX-only test files (schedule, nightly) leave out.
mod common;
use common::*;
use enigma_mcp::faults::{self, Fault};
use std::process::{Command, Output, Stdio};

fn cli(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            c.env_remove(k);
        }
    }
    c.args(args).envs(env.iter().copied()).stdin(Stdio::null()).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn schedule_refuses_on_windows() {
    // 15 §9.2: launchd jobs are macOS only; nothing is written or run
    let d = mini_vault();
    let home = tempfile::tempdir().unwrap();
    let vault = d.path().to_str().unwrap();
    for args in [vec!["schedule", "install", "--job", "librarian", vault], vec!["schedule", "uninstall", "--job", "librarian"]] {
        let o = cli(&args, &[("HOME", home.path().to_str().unwrap())]);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(text(&o.stderr).contains("Task Scheduler"), "{}", text(&o.stderr));
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0, "nothing written under HOME");
    }
}

#[test]
fn vault_root_has_no_verbatim_prefix() {
    // 01 §3
    let d = mini_vault();
    let v = open(&d);
    assert!(!v.root.to_string_lossy().starts_with(r"\\?\"), "{}", v.root.display());
    assert!(v.root.is_dir());
}

#[test]
fn claude_folder_matches_in_any_case() {
    // 03 §3: on Windows both sides are lowercased before comparing
    let d = mini_vault();
    let v = open(&d);
    v.write("claude/Sessions/x.md", "x", false, false).unwrap();
    v.append("CLAUDE/Index.md", "y\n", false).unwrap();
    assert!(read(&d, "Claude/Index.md").ends_with("y\n"));
    assert!(v.write("ClaudeFake/x.md", "x", false, false).unwrap_err().msg.contains("outside Claude/"));
    assert!(v.write("Daily.md", "x", true, false).unwrap_err().msg.contains("outside Claude/"));
    assert!(v.read("../outside.md").unwrap_err().msg.contains("escapes the vault"));
}

#[test]
fn notes_match_md_in_any_case_and_sort_lowercased() {
    // 00 §2 and §3.3
    let d = vault_with(&[("Zeta.MD", "# Zeta\n"), ("alpha.md", "# Alpha\n"), ("Beta.md", "# Beta\n"), ("notes.txt", "no\n")]);
    let rels: Vec<String> = enigma_mcp::search::walk(&open(&d), None).unwrap().into_iter().map(|(_, r)| r).collect();
    assert_eq!(rels, ["alpha.md", "Beta.md", "Zeta.MD"]);
}

#[test]
fn a_sharing_violation_is_a_lock_error() {
    // 03 §5.1: retried, then the append goes through
    faults::reset();
    let d = mini_vault();
    faults::arm("append_open", Fault::Os(32), 2);
    open(&d).append("Claude/Index.md", "z\n", false).unwrap();
    assert_eq!(faults::calls("append_open"), 3);
    assert!(read(&d, "Claude/Index.md").ends_with("z\n"));
}

#[test]
fn owner_name_lookup_is_safe() {
    // the account may have no display name; the call must simply answer
    if let Some(name) = enigma_mcp::conventions::system_owner() {
        assert!(!name.trim().is_empty());
    }
}

#[test]
fn nightly_runner_indexes_through_a_cmd_extractor() {
    // 15 §9 on Windows: the lock, the process list, PATHEXT lookup and a .cmd extractor, end to end
    let root = tempfile::tempdir().unwrap();
    let p = |rel: &str| root.path().join(rel);
    for dir in ["home", "tmp", "state", "bin", "vault"] {
        std::fs::create_dir_all(p(dir)).unwrap();
    }
    write_file(&p("vault"), "Note.md", "# Note\n\nOne note to index.\n");
    stub_program(&p("bin"), "claude", "", "[Console]::In.ReadToEnd() | Out-Null\n'{\"entities\": [], \"relations\": []}'\n");
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let cache = var("HF_HUB_CACHE")
        .or_else(|| var("HF_HOME").map(|h| format!("{h}\\hub")))
        .unwrap_or_else(|| format!("{}\\.cache\\huggingface\\hub", var("HOME").or_else(|| var("USERPROFILE")).unwrap_or_default()));
    let o = cli(
        &["index-nightly", p("vault").to_str().unwrap(), p("state").to_str().unwrap()],
        &[("HOME", p("home").to_str().unwrap()), ("TMPDIR", p("tmp").to_str().unwrap()), ("HF_HUB_CACHE", &cache), ("PATH", &sandbox_path(&p("bin")))],
    );
    let out = text(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{out}\n{}", text(&o.stderr));
    assert!(out.contains("\"remaining\": 0") && out.contains("index-nightly done"), "{out}");
    assert!(!p("tmp/enigma-index.lock").exists(), "the lock is removed");
}
