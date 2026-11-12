//! acceptance/locking.yaml
mod common;
use common::{bin, mini_vault};
use enigma_mcp::error::Kind;
use enigma_mcp::lock::acquire;
use std::process::Command;

#[test]
fn locking_lk_01() {
    let d = tempfile::tempdir().unwrap();
    let _held = acquire(d.path()).unwrap();
    let start = std::time::Instant::now();
    let e = acquire(d.path()).err().unwrap();
    assert_eq!(e.kind, Kind::LockBusy);
    assert!(start.elapsed().as_millis() < 500, "fails immediately");
    assert!(e.msg.contains("another maintenance run") && e.msg.contains("caretaker.lock"));
}

#[test]
fn locking_lk_02() {
    let d = tempfile::tempdir().unwrap();
    drop(acquire(d.path()).unwrap());
    let _again = acquire(d.path()).unwrap();
}

#[test]
fn locking_lk_03() {
    assert!(Kind::LockBusy.is(Kind::VaultError));
}

fn librarian_cli(vault: &std::path::Path, st: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            cmd.env_remove(k);
        }
    }
    cmd.arg("librarian").arg(vault).args(args).env("ENIGMA_STATE_DIR", st).output().unwrap()
}

#[test]
fn locking_lk_04() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let _held = acquire(st.path()).unwrap();
    let o = librarian_cli(d.path(), st.path(), &["--confirm-first-run"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("error: LockBusy: "));
    assert!(o.stdout.is_empty(), "sweep never starts");
    assert!(!d.path().join("Claude/Librarian.md").exists());
    assert!(!st.path().join("librarian_state.json").exists());
}

#[test]
fn locking_lk_05() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let _held = acquire(st.path()).unwrap();
    let o = librarian_cli(d.path(), st.path(), &["--dry-run"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"applied\": false"));
}
