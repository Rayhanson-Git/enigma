//! acceptance/sanity.yaml
mod common;
use common::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn run(args: &[&str]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| (e.path().display().to_string(), std::fs::read(e.path()).unwrap()))
        .collect()
}

#[test]
fn sanity_sn_01() {
    let o = run(&["--version"]);
    assert!(o.status.success());
    assert_eq!(String::from_utf8(o.stdout).unwrap(), format!("enigma-mcp {VERSION}\n"));
    assert!(o.stderr.is_empty());
}

#[test]
fn sanity_sn_02() {
    let (a, b) = (run(&["-V"]), run(&["--version"]));
    assert_eq!(a.status.code(), b.status.code());
    assert_eq!(a.stdout, b.stdout);
}

#[test]
fn sanity_sn_03() {
    let parts: Vec<&str> = VERSION.split('.').collect();
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())));
    let toml = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(toml.contains(&format!("version = \"{VERSION}\"")));
    for e in walkdir::WalkDir::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("src")).into_iter().flatten() {
        if e.file_type().is_file() {
            assert!(!std::fs::read_to_string(e.path()).unwrap().contains(VERSION), "{} hard-codes the version", e.path().display());
        }
    }
}

#[test]
fn sanity_sn_04() {
    let o = Command::new(bin()).arg("--version").env_clear().env("HF_HUB_CACHE", "/nonexistent").output().unwrap();
    assert!(o.status.success());
    assert_eq!(String::from_utf8(o.stdout).unwrap(), format!("enigma-mcp {VERSION}\n"));
    assert!(o.stderr.is_empty());
}

/// The SN-05 session: fixture vault copy, fresh state dir, initialize sent.
fn handshake() -> (tempfile::TempDir, tempfile::TempDir, Mcp, serde_json::Value, Duration) {
    let vault = fixture_vault();
    let state = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let (m, init) = Mcp::start(&[
        ("ENIGMA_VAULT_PATH", vault.path().to_str().unwrap()),
        ("ENIGMA_STATE_DIR", state.path().to_str().unwrap()),
    ]);
    (vault, state, m, init, start.elapsed())
}

#[test]
fn sanity_sn_05() {
    let (_vault, _state, m, init, took) = handshake();
    assert!(took < Duration::from_secs(60));
    assert_eq!(init["id"], json!(1));
    assert_eq!(init["result"]["serverInfo"]["name"], "enigma");
    assert_eq!(init["result"]["serverInfo"]["version"], VERSION);
    assert!(init["result"]["capabilities"]["tools"].is_object());
    m.close();
}

#[test]
fn sanity_sn_06() {
    let (vault, _state, mut m, _init, _) = handshake();
    let before = snapshot(vault.path());
    let t = Instant::now();
    let r = m.call("search_brain", json!({"query": "probe", "limit": 1}));
    assert!(t.elapsed() < Duration::from_secs(30));
    assert!(!is_error(&r));
    assert!(r["structuredContent"]["result"].as_array().unwrap().len() <= 1);
    assert_eq!(m.close().code(), Some(0));
    assert_eq!(snapshot(vault.path()), before);
}

#[test]
fn sanity_sn_07() {
    let o = run(&["--help"]);
    assert!(o.status.success());
    let help = String::from_utf8(o.stdout).unwrap();
    assert!(help.contains("enigma-mcp"));
    let commands: Vec<&str> = help
        .split("Commands:\n")
        .nth(1)
        .unwrap()
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "index", "consolidate", "organize", "librarian", "cleanup", "sheets", "recall", "evals", "index-nightly",
            "install-conventions", "mcp-sync", "skill-sync", "hook-sync", "hook", "schedule", "provision"
        ]
    );
}

#[test]
fn sanity_sn_08() {
    let o = run(&["no-such-command"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(err.contains("Usage") && err.trim_end().lines().last().unwrap().starts_with("enigma-mcp: error: "), "{err}");
}
