//! acceptance/sync.yaml SC-01..09: enigma-mcp schedule. HOME is a temp dir and ENIGMA_LAUNCHCTL a stub that
//! only logs its arguments; the real launchctl is never run.
mod common;
use common::*;
use enigma_mcp::nightly::{INDEX_PLIST, LIBRARIAN_PLIST, render};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct S {
    root: TempDir,
}

impl S {
    fn new() -> S {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        fs::create_dir_all(root.path().join("vault")).unwrap();
        let stub = root.path().join("launchctl");
        let log = root.path().join("launchctl.log");
        let code = root.path().join("exit-code");
        fs::write(&stub, format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit $(cat '{}' 2>/dev/null || echo 0)\n", log.display(), code.display())).unwrap();
        fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        S { root }
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }
    fn vault(&self) -> PathBuf {
        self.p("vault").canonicalize().unwrap()
    }
    fn agents(&self, job: &str) -> PathBuf {
        self.p(&format!("home/Library/LaunchAgents/com.enigma.{job}.plist"))
    }
    fn fail_with(&self, code: i32) {
        fs::write(self.p("exit-code"), code.to_string()).unwrap();
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.p("launchctl.log")).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
    }
    fn go(&self, args: &[&str]) -> Output {
        let mut c = Command::new(bin());
        for (k, _) in std::env::vars() {
            if k.starts_with("ENIGMA_") {
                c.env_remove(k);
            }
        }
        c.arg("schedule").args(args).env("HOME", self.p("home")).env("ENIGMA_LAUNCHCTL", self.p("launchctl")).output().unwrap()
    }
    fn rendered(&self, job: &str, vault: &Path) -> String {
        let exe = Path::new(bin()).canonicalize().unwrap();
        render(if job == "index" { INDEX_PLIST } else { LIBRARIAN_PLIST }, &exe, vault, &self.p("home"))
    }
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}
fn err(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

#[test]
fn schedule_sc_01() {
    let s = S::new();
    let v = s.vault();
    let o = s.go(&["install", "--job", "index", v.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let p = s.agents("index");
    assert_eq!(fs::read_to_string(&p).unwrap(), s.rendered("index", &v));
    assert!(s.p("home/.enigma-mcp").is_dir());
    assert_eq!(s.calls(), [format!("bootstrap gui/{} {}", uid(), p.display())]);
    assert_eq!(out(&o), format!("installed: {}\nloaded: com.enigma.index (gui/{})\n", p.display(), uid()));
}

#[test]
fn schedule_sc_02() {
    let s = S::new();
    let v = s.vault();
    let p = s.agents("librarian");
    let want = format!(
        "{}\n# would write {}\n# would run: {} bootstrap gui/{} {}\n",
        s.rendered("librarian", &v),
        p.display(),
        s.p("launchctl").display(),
        uid(),
        p.display()
    );
    for args in [vec!["print", "--job", "librarian"], vec!["install", "--dry-run", "--job", "librarian"]] {
        let mut a = args.clone();
        let vs = v.to_str().unwrap().to_string();
        a.push(&vs);
        let o = s.go(&a);
        assert_eq!((o.status.code(), out(&o)), (Some(0), want.clone()), "{args:?}");
    }
    assert!(!s.p("home/Library").exists() && !s.p("home/.enigma-mcp").exists());
    assert!(s.calls().is_empty());
}

#[test]
fn schedule_sc_03() {
    let s = S::new();
    let v = s.vault().display().to_string();
    s.go(&["install", "--job", "index", &v]);
    let o = s.go(&["install", "--job", "librarian", &v]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).ends_with(&format!(
        "error: com.enigma.index is already installed ({}); install only one of the two jobs. Remove it first with: enigma-mcp schedule uninstall --job index\n",
        s.agents("index").display()
    )));
    assert!(!s.agents("librarian").exists());
    assert_eq!(s.calls().len(), 1, "only the first install ran launchctl");
}

#[test]
fn schedule_sc_04() {
    let s = S::new();
    let v = s.vault();
    let p = s.agents("index");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, "mine").unwrap();
    let o = s.go(&["install", "--job", "index", v.to_str().unwrap()]);
    assert_eq!((o.status.code(), err(&o)), (Some(2), format!("error: {} exists and differs from the plist this command would install; re-run with --force to replace it\n", p.display())));
    assert_eq!(fs::read_to_string(&p).unwrap(), "mine");
    assert!(s.calls().is_empty());
    let o = s.go(&["install", "--job", "index", "--force", v.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(s.calls(), [format!("bootout gui/{}/com.enigma.index", uid()), format!("bootstrap gui/{} {}", uid(), p.display())]);
    assert_eq!(fs::read_to_string(&p).unwrap(), s.rendered("index", &v));
}

#[test]
fn schedule_sc_05() {
    let s = S::new();
    let v = s.vault().display().to_string();
    s.go(&["install", "--job", "index", &v]);
    let o = s.go(&["install", "--job", "index", &v]);
    assert_eq!((o.status.code(), out(&o)), (Some(0), format!("already installed: {}\n", s.agents("index").display())));
    assert_eq!(s.calls().len(), 1);
}

#[test]
fn schedule_sc_06() {
    let s = S::new();
    let o = s.go(&["uninstall", "--job", "index"]);
    assert_eq!((o.status.code(), out(&o)), (Some(0), "not installed: com.enigma.index\n".to_string()));
    assert!(s.calls().is_empty());
    s.go(&["install", "--job", "index", s.vault().to_str().unwrap()]);
    let p = s.agents("index");
    let o = s.go(&["uninstall", "--job", "index"]);
    assert_eq!((o.status.code(), out(&o)), (Some(0), format!("unloaded: com.enigma.index\nremoved: {}\n", p.display())));
    assert_eq!(s.calls().last().unwrap(), &format!("bootout gui/{}/com.enigma.index", uid()));
    assert!(!p.exists());
    // a failing bootout only warns
    s.go(&["install", "--job", "index", s.vault().to_str().unwrap()]);
    s.fail_with(3);
    let o = s.go(&["uninstall", "--job", "index"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(err(&o).starts_with("warning: launchctl bootout failed (exit 3)"), "{}", err(&o));
    assert!(!p.exists());
}

#[test]
fn schedule_sc_07() {
    let s = S::new();
    let icloud = s.p("home/Library/Mobile Documents/iCloud~md~obsidian/Documents/V");
    fs::create_dir_all(&icloud).unwrap();
    let o = s.go(&["install", "--job", "index", icloud.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let home = s.p("home").canonicalize().unwrap();
    let e = err(&o);
    assert!(e.starts_with(&format!("warning: {} is in a folder macOS protects ({}/Library/Mobile Documents)", icloud.canonicalize().unwrap().display(), home.display())), "{e}");
    assert!(e.contains("Full Disk Access") && e.lines().count() == 1);
    assert!(s.agents("index").is_file());
    let s = S::new();
    let o = s.go(&["install", "--job", "index", s.vault().to_str().unwrap()]);
    assert!(err(&o).is_empty());
}

#[test]
fn schedule_sc_08() {
    let s = S::new();
    let v = s.vault();
    let o = s.go(&["print", "--job", "librarian", v.to_str().unwrap()]);
    assert!(err(&o).starts_with("warning: no Librarian sweep has run on this vault yet;"), "{}", err(&o));
    let o = s.go(&["install", "--job", "librarian", v.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(err(&o).contains("--confirm-first-run"));
    // with a Librarian state in the derived state dir the warning goes
    s.go(&["uninstall", "--job", "librarian"]);
    let hex = derived(&s, &v);
    fs::create_dir_all(&hex).unwrap();
    fs::write(enigma_mcp::librarian::state_path(&hex), "{}").unwrap();
    let o = s.go(&["install", "--job", "librarian", v.to_str().unwrap()]);
    assert!(err(&o).is_empty(), "{}", err(&o));
}

/// The state dir the job will use: HOME/.enigma-mcp/<digest of the vault>.
fn derived(s: &S, v: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(v.to_string_lossy().as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect();
    s.p("home/.enigma-mcp").join(hex)
}

#[test]
fn schedule_sc_09() {
    let s = S::new();
    for action in ["install", "print"] {
        let o = s.go(&[action, "--job", "index"]);
        assert_eq!(o.status.code(), Some(2), "{action}");
    }
    let gone = s.p("gone");
    let o = s.go(&["install", "--job", "index", gone.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("error: VaultError: Vault root does not exist: "), "{}", err(&o));
    s.fail_with(5);
    let o = s.go(&["install", "--job", "index", s.vault().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("error: launchctl bootstrap failed (exit 5)"), "{}", err(&o));
    assert!(s.agents("index").is_file(), "P stays");
}
