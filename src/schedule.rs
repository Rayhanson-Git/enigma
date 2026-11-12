//! enigma-mcp schedule: installs, removes or prints one launchd job (15 §9.2).
use crate::mcp_sync::quote_word;
use crate::{config, librarian, nightly};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Folders under HOME that macOS guards with a privacy permission.
const PROTECTED: [&str; 5] = ["Desktop", "Documents", "Downloads", "Library/Mobile Documents", "Library/CloudStorage"];

pub struct Job<'a> {
    pub name: &'a str,
    pub force: bool,
    pub dry_run: bool,
    pub exe: &'a Path,
}

fn label(job: &str) -> String {
    format!("com.enigma.{job}")
}

fn plist(home: &Path, job: &str) -> PathBuf {
    home.join("Library/LaunchAgents").join(format!("{}.plist", label(job)))
}

fn launchctl() -> String {
    config::var("ENIGMA_LAUNCHCTL").unwrap_or_else(|| "/bin/launchctl".into())
}

fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

fn shown(cmd: &[String]) -> String {
    cmd.iter().map(|w| quote_word(w)).collect::<Vec<_>>().join(" ")
}

/// Runs one launchctl command; Err is "(exit N)" or "(OS error)" plus the first stderr line.
fn launch(cmd: &[String]) -> Result<(), String> {
    match Command::new(&cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).stdout(Stdio::null()).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let first = String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or_default().trim().to_string();
            let code = o.status.code().map_or("signal".to_string(), |c| format!("exit {c}"));
            Err(if first.is_empty() { format!("({code})") } else { format!("({code}): {first}") })
        }
        Err(e) => Err(format!("({e})")),
    }
}

/// The protected folder holding the vault, if any.
fn protected(root: &Path, home: &Path) -> Option<PathBuf> {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    PROTECTED.iter().map(|f| home.join(f)).chain([PathBuf::from("/Volumes")]).find(|f| root.starts_with(f))
}

fn refuse(msg: String) -> u8 {
    eprintln!("error: {msg}");
    2
}

/// install / print (15 §9.2); returns the exit code.
pub fn install(job: &Job, vault: &Path) -> u8 {
    let root = match config::open_root(vault) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let home = config::home();
    let template = if job.name == "index" { nightly::INDEX_PLIST } else { nightly::LIBRARIAN_PLIST };
    let text = nightly::render(template, job.exe, &root, &home);
    let v = root.display();
    if let Some(folder) = protected(&root, &home) {
        eprintln!(
            "warning: {v} is in a folder macOS protects ({}); a launchd job can read it only after you grant Full Disk Access to {} (System Settings > Privacy & Security > Full Disk Access), otherwise the scheduled run fails with \"cannot read vault (macOS privacy permission for enigma-mcp/launchd?)\" or a permission error",
            folder.display(),
            job.exe.display()
        );
    }
    // the job gets no ENIGMA_STATE_DIR, so it uses the derived state dir
    let swept = config::derived_state_dir(&root).is_ok_and(|st| librarian::state_path(&st).exists());
    if job.name == "librarian" && !swept {
        eprintln!(
            "warning: no Librarian sweep has run on this vault yet; the scheduled sweep will refuse until you run \"enigma-mcp librarian {v} --dry-run\", review the report, then run \"enigma-mcp librarian {v} --confirm-first-run\""
        );
    }
    let other = if job.name == "index" { "librarian" } else { "index" };
    let (p, theirs) = (plist(&home, job.name), plist(&home, other));
    if theirs.exists() {
        return refuse(format!(
            "{} is already installed ({}); install only one of the two jobs. Remove it first with: enigma-mcp schedule uninstall --job {other}",
            label(other),
            theirs.display()
        ));
    }
    let existing = fs::read(&p).ok();
    if existing.as_deref() == Some(text.as_bytes()) {
        println!("already installed: {}", p.display());
        return 0;
    }
    if existing.is_some() && !job.force {
        return refuse(format!("{} exists and differs from the plist this command would install; re-run with --force to replace it", p.display()));
    }
    let (lc, dom) = (launchctl(), domain());
    let bootout = vec![lc.clone(), "bootout".into(), format!("{dom}/{}", label(job.name))];
    let bootstrap = vec![lc, "bootstrap".into(), dom.clone(), p.display().to_string()];
    if job.dry_run {
        println!("{text}");
        println!("# would write {}", p.display());
        if existing.is_some() {
            println!("# would run: {}", shown(&bootout));
        }
        println!("# would run: {}", shown(&bootstrap));
        return 0;
    }
    if existing.is_some() {
        let _ = launch(&bootout); // may not be loaded
    }
    let written = (|| -> std::io::Result<()> {
        fs::create_dir_all(p.parent().unwrap_or(&home))?;
        fs::create_dir_all(home.join(".enigma-mcp"))?; // launchd does not create the log folder
        fs::write(&p, &text)
    })();
    if let Err(e) = written {
        eprintln!("error: IoError: {e}: {}", p.display());
        return 1;
    }
    if let Err(why) = launch(&bootstrap) {
        eprintln!(
            "error: launchctl bootstrap failed {why}; {} stays installed and loads at next login; remove it with: enigma-mcp schedule uninstall --job {}",
            p.display(),
            job.name
        );
        return 1;
    }
    println!("installed: {}", p.display());
    println!("loaded: {} ({dom})", label(job.name));
    0
}

/// uninstall (15 §9.2); a clean no-op when nothing is installed.
pub fn uninstall(job: &Job) -> u8 {
    let p = plist(&config::home(), job.name);
    if !p.exists() {
        println!("not installed: {}", label(job.name));
        return 0;
    }
    let bootout = vec![launchctl(), "bootout".into(), format!("{}/{}", domain(), label(job.name))];
    if job.dry_run {
        println!("# would run: {}", shown(&bootout));
        println!("# would remove {}", p.display());
        return 0;
    }
    match launch(&bootout) {
        Ok(()) => println!("unloaded: {}", label(job.name)),
        Err(why) => eprintln!("warning: launchctl bootout failed {why}; the job was probably not loaded"),
    }
    if let Err(e) = fs::remove_file(&p) {
        eprintln!("error: IoError: {e}: {}", p.display());
        return 1;
    }
    println!("removed: {}", p.display());
    0
}
