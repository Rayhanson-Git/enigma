//! The nightly index runner and launchd templates (15 §9).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};

const MAX_ROUNDS: usize = 20;

pub const INDEX_PLIST: &str = include_str!("launchd/com.enigma.index.plist");
pub const LIBRARIAN_PLIST: &str = include_str!("launchd/com.enigma.librarian.plist");

/// A template with {BIN}, {VAULT} and {HOME} filled, XML-escaped (15 §9.1).
pub fn render(template: &str, bin: &Path, vault: &Path, home: &Path) -> String {
    let xml = |p: &Path| p.display().to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    template.replace("{BIN}", &xml(bin)).replace("{VAULT}", &xml(vault)).replace("{HOME}", &xml(home))
}

/// LOCK: $TMPDIR/enigma-index.lock, or /tmp/enigma-index.lock.
pub fn lock_path() -> PathBuf {
    match std::env::var("TMPDIR") {
        Ok(t) if !t.is_empty() => PathBuf::from(t).join("enigma-index.lock"),
        _ => PathBuf::from("/tmp/enigma-index.lock"),
    }
}

fn stamp() -> String {
    crate::mover::second_stamp(&crate::text::now())
}

/// Command lines of running processes as (pid, command).
fn processes() -> Vec<(u32, String)> {
    let Ok(out) = Command::new("ps").args(["-Ao", "pid=,command="]).stderr(Stdio::null()).output() else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (pid, cmd) = l.trim_start().split_once(' ')?;
            Some((pid.parse().ok()?, cmd.trim_start().to_string()))
        })
        .collect()
}

/// The last SIGINT/SIGTERM/SIGHUP received while the lock is held (0: none).
static SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL.store(sig, Ordering::SeqCst);
}

/// After an interrupt: remove the lock, then die of the same signal (15 §9 step 8).
fn stop_if_signalled(lock: &Path) {
    let sig = SIGNAL.load(Ordering::SeqCst);
    if sig == 0 {
        return;
    }
    let _ = std::fs::remove_dir_all(lock);
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
    std::process::exit(128 + sig); // not reached: the default action ends the process
}

fn take_lock(lock: &Path) -> Result<(), String> {
    if std::fs::create_dir(lock).is_ok() {
        return Ok(());
    }
    let pid = std::fs::read_to_string(lock.join("pid")).ok().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let live = pid.as_ref().and_then(|p| p.parse::<u32>().ok()).is_some_and(|p| processes().iter().any(|(q, cmd)| *q == p && cmd.contains("index-nightly")));
    if live {
        return Err(format!("another index run (pid {}) holds {}; skipping", pid.unwrap_or_default(), lock.display()));
    }
    println!("removing stale lock {} (pid {} is not running)", lock.display(), pid.as_deref().unwrap_or("unknown"));
    let _ = std::fs::remove_dir_all(lock);
    std::fs::create_dir(lock).map_err(|_| format!("another run took {}; skipping", lock.display()))
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// One round: EXE index VAULT with stdout and stderr captured together.
fn round(exe: &Path, vault: &Path, state_dir: Option<&Path>) -> std::io::Result<(bool, String)> {
    let (mut reader, writer) = std::io::pipe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("index").arg(vault).env("ENIGMA_VAULT_PATH", vault).stdin(Stdio::null()).stdout(writer.try_clone()?).stderr(writer);
    if let Some(st) = state_dir {
        cmd.env("ENIGMA_STATE_DIR", st);
    }
    for (k, default) in [("ENIGMA_EXTRACTOR", "claude"), ("ENIGMA_EXTRACTOR_NO_TOOLS", "1")] {
        if std::env::var(k).unwrap_or_default().is_empty() {
            cmd.env(k, default);
        }
    }
    let mut child = cmd.spawn()?;
    drop(cmd); // closes our copies of the pipe's write end
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    let status = child.wait()?;
    Ok((status.success(), String::from_utf8_lossy(&out).into_owned()))
}

/// index-nightly VAULT [STATE_DIR] (15 §9); returns the exit code.
pub fn run(vault: &Path, state_dir: Option<&Path>, exe: &Path) -> u8 {
    println!("[{}] index-nightly start", stamp());
    if std::fs::read_dir(vault).is_err() {
        println!("cannot read vault (macOS privacy permission for enigma-mcp/launchd?)");
        return 2;
    }
    let lock = lock_path();
    if let Err(msg) = take_lock(&lock) {
        println!("{msg}");
        return 0;
    }
    let _ = std::fs::write(lock.join("pid"), format!("{}\n", std::process::id()));
    // a signal does not stop the running round; it ends the run once that round returns
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe { libc::signal(sig, on_signal as *const () as libc::sighandler_t) };
    }
    let code = rounds(vault, state_dir, exe, &lock);
    stop_if_signalled(&lock);
    let _ = std::fs::remove_dir_all(&lock);
    code
}

fn rounds(vault: &Path, state_dir: Option<&Path>, exe: &Path, lock: &Path) -> u8 {
    let me = std::process::id();
    if processes().iter().any(|(pid, cmd)| *pid != me && cmd.contains("enigma-mcp index ")) {
        println!("indexer already running; skipping");
        return 0;
    }
    for _ in 0..MAX_ROUNDS {
        stop_if_signalled(lock);
        let (ok, out) = match round(exe, vault, state_dir) {
            Ok(r) => r,
            Err(e) => (false, format!("cannot run {}: {e}", exe.display())),
        };
        stop_if_signalled(lock);
        if !ok {
            println!("{}", last_lines(&out, 5));
            return 1;
        }
        println!("{}", last_lines(&out, 12));
        if out.contains("\"remaining\": 0") {
            break;
        }
    }
    println!("[{}] index-nightly done", stamp());
    0
}
