//! acceptance/indexer.yaml
mod common;
use common::*;
use enigma_mcp::error::{Error, Kind, Result};
use enigma_mcp::extractor::Extract;
use enigma_mcp::graph::{Entity, Extraction, Graph, upsert_entity};
use enigma_mcp::indexer::*;
use enigma_mcp::search::fallback_path;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

#[derive(Default)]
struct Fake {
    out: HashMap<String, Extraction>,
    fail: HashSet<String>,
    calls: Mutex<Vec<String>>,
}

impl Extract for Fake {
    fn extract(&self, path: &str, _content: &str) -> Result<Extraction> {
        self.calls.lock().unwrap().push(path.to_string());
        if self.fail.contains(path) {
            return Err(Error::new(Kind::ExtractorError, "boom"));
        }
        Ok(self.out.get(path).cloned().unwrap_or_default())
    }
}

impl Fake {
    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

fn orbital() -> Extraction {
    Extraction {
        entities: vec![Entity { name: "Orbital Freight".into(), ty: "organization".into(), aliases: vec![], summary: "A company.".into() }],
        relations: vec![],
    }
}

fn fake_for(path: &str) -> Fake {
    Fake { out: HashMap::from([(path.to_string(), orbital())]), ..Default::default() }
}

fn opts(batch: usize, force: bool, retry: bool) -> Options {
    Options { batch, force, retry_failures: retry }
}

fn go(d: &tempfile::TempDir, st: &Path, ex: &Fake, o: &Options) -> Counts {
    run(&open(d), st, ex, None, o, &now()).unwrap()
}

#[test]
fn indexer_ix_01() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Graph/Topics/T.md", "t");
    write_file(d.path(), "copilot/p.md", "p");
    let scan = scan_notes(&open(&d)).unwrap();
    assert!(scan.contains_key("Daily.md"));
    assert!(scan.keys().all(|k| !k.starts_with("Claude/Graph/") && !k.starts_with("copilot/")));
    assert_eq!(scan["Daily.md"].len(), 64);
}

#[test]
fn indexer_ix_02() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Organizer.md", "o");
    write_file(d.path(), "Claude/Librarian.md", "l");
    let scan = scan_notes(&open(&d)).unwrap();
    assert!(!scan.contains_key("Claude/Organizer.md") && !scan.contains_key("Claude/Librarian.md"));
    assert_eq!(CARETAKER_NOTES, ["Claude/Organizer.md", "Claude/Librarian.md"]);
}

#[test]
fn indexer_ix_03() {
    let st = tempfile::tempdir().unwrap();
    std::fs::write(
        st.path().join("manifest.json"),
        r#"{"hashes": {"Claude/Organizer.md": "h", "Daily.md": "x"}, "failures": {"Claude/Librarian.md": {"error": "e", "attempts": 1}}}"#,
    )
    .unwrap();
    let m = Manifest::load(st.path()).unwrap();
    assert_eq!(m.hashes.keys().collect::<Vec<_>>(), ["Daily.md"]);
    assert!(m.failures.is_empty());
}

#[test]
fn indexer_ix_04() {
    for text in [r#"{"hashes": {}}"#, r#"{"hashes": null, "failures": null}"#] {
        let d = mini_vault();
        let st = tempfile::tempdir().unwrap();
        std::fs::write(st.path().join("manifest.json"), text).unwrap();
        let m = Manifest::load(st.path()).unwrap();
        assert!(m.hashes.is_empty() && m.failures.is_empty());
        assert_eq!(go(&d, st.path(), &Fake::default(), &opts(25, false, true)).failed, 0);
    }
    let st = tempfile::tempdir().unwrap();
    std::fs::write(st.path().join("manifest.json"), "{not json").unwrap();
    assert_eq!(Manifest::load(st.path()).unwrap_err().kind, Kind::InvalidData);
}

#[test]
fn indexer_ix_05() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let ex = fake_for("Daily.md");
    let c = go(&d, st.path(), &ex, &Options::default());
    assert!(c.processed > 0);
    assert_eq!((c.entities_created, c.remaining), (1, 0));
    ex.calls();
    assert_eq!(go(&d, st.path(), &ex, &Options::default()).processed, 0);
    assert!(ex.calls().is_empty());
}

#[test]
fn indexer_ix_06() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let ex = Fake::default();
    go(&d, st.path(), &ex, &Options::default());
    ex.calls();
    write_file(d.path(), "Claude/Inbox/new.md", "new note");
    go(&d, st.path(), &ex, &Options::default());
    assert_eq!(ex.calls(), ["Claude/Inbox/new.md"]);
}

#[test]
fn indexer_ix_07() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let ex = Fake::default();
    let n = scan_notes(&open(&d)).unwrap().len() as i64;
    go(&d, st.path(), &ex, &Options::default());
    assert_eq!(go(&d, st.path(), &ex, &opts(25, true, false)).processed, n);
}

#[test]
fn indexer_ix_08() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let n = scan_notes(&open(&d)).unwrap().len() as i64;
    let c = go(&d, st.path(), &Fake::default(), &opts(1, false, false));
    assert_eq!((c.processed, c.remaining), (1, n - 1));
}

#[test]
fn indexer_ix_09() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let ex = Fake { fail: HashSet::from(["Daily.md".to_string()]), ..Default::default() };
    assert_eq!(go(&d, st.path(), &ex, &Options::default()).failed, 1);
    let m = Manifest::load(st.path()).unwrap();
    assert_eq!(m.failures["Daily.md"], Failure { error: "boom".into(), attempts: 1 });
    assert!(std::fs::read_to_string(Manifest::path(st.path())).unwrap().contains("\"failures\": {\n    \"Daily.md\""));
    ex.calls();
    go(&d, st.path(), &ex, &Options::default());
    assert_eq!(ex.calls(), ["Daily.md"]);
}

#[test]
fn indexer_ix_10() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    go(&d, st.path(), &fake_for("Daily.md"), &Options::default());
    assert!(!Graph::load(&st.path().join("graph.db")).unwrap().find("orbital", None).is_empty());
}

#[test]
fn indexer_ix_11() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    upsert_entity(&open(&d), &orbital().entities[0], &now()).unwrap();
    let out = Command::new(bin())
        .args(["index", d.path().to_str().unwrap(), "--rebuild-only"])
        .env("ENIGMA_STATE_DIR", st.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"rebuilt\": true"), "{stdout}");
    assert!(stdout.starts_with("{\"rebuilt\": true, \"db\": ") && stdout.ends_with("}\n"));
    assert!(!Graph::load(&st.path().join("graph.db")).unwrap().find("orbital", None).is_empty());
}

#[test]
fn indexer_ix_12() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let ex = Fake { fail: HashSet::from(["Daily.md".to_string()]), ..Default::default() };
    for _ in 0..3 {
        go(&d, st.path(), &ex, &Options::default());
    }
    assert_eq!(Manifest::load(st.path()).unwrap().failures["Daily.md"].attempts, 3);
    ex.calls();
    let c = go(&d, st.path(), &ex, &Options::default());
    assert_eq!(c.skipped, 1);
    assert!(!ex.calls().contains(&"Daily.md".to_string()));
    go(&d, st.path(), &ex, &opts(25, true, false));
    assert!(ex.calls().contains(&"Daily.md".to_string()));
    go(&d, st.path(), &Fake::default(), &opts(25, true, false));
    assert!(!Manifest::load(st.path()).unwrap().failures.contains_key("Daily.md"));
}

#[test]
fn indexer_ix_13() {
    let st = tempfile::tempdir().unwrap();
    std::fs::write(st.path().join("manifest.json"), r#"{"hashes": {}, "failures": {"Daily.md": "old error"}}"#).unwrap();
    assert_eq!(Manifest::load(st.path()).unwrap().failures["Daily.md"], Failure { error: "old error".into(), attempts: 1 });
}

#[test]
fn indexer_ix_14() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    go(&d, st.path(), &Fake::default(), &Options::default());
    let db = st.path().join("graph.db");
    let before = file_identity(&db);
    assert_eq!(go(&d, st.path(), &Fake::default(), &Options::default()).processed, 0);
    assert_eq!(file_identity(&db), before, "graph.db was not rebuilt");
}

#[test]
fn indexer_ix_15() {
    // runs ix_15_child in fresh processes so the environment can differ safely
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (mini_vault(), mini_vault());
    for mode in ["with", "without"] {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["ix_15_child", "--exact", "--nocapture"])
            .env_remove("ENIGMA_STATE_DIR")
            .env_remove("ENIGMA_VAULT_PATH")
            .env("HOME", home.path())
            .env("IX15_MODE", mode)
            .env("IX15_A", a.path())
            .env("IX15_B", b.path())
            .envs(if mode == "with" { vec![("ENIGMA_VAULT_PATH", a.path())] } else { vec![] })
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"));
    }
}

#[test]
fn ix_15_child() {
    use enigma_mcp::config::state_dir;
    let Ok(mode) = std::env::var("IX15_MODE") else { return };
    let (a, b) = (std::env::var("IX15_A").unwrap(), std::env::var("IX15_B").unwrap());
    let (sa, sb) = (state_dir(Some(Path::new(&a))).unwrap(), state_dir(Some(Path::new(&b))).unwrap());
    assert_ne!(sa, sb);
    assert_eq!(sa.parent(), sb.parent());
    assert_eq!(sa.parent().unwrap(), Path::new(&std::env::var("HOME").unwrap()).join(".enigma-mcp"));
    assert_eq!(state_dir(Some(Path::new(&a))).unwrap(), sa);
    if mode == "with" {
        assert_eq!(state_dir(None).unwrap(), sa);
    } else {
        assert!(state_dir(None).unwrap_err().msg.contains("ENIGMA_VAULT_PATH"));
    }
}

#[test]
fn indexer_ix_16() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let emb = LenEmb::default();
    run(&open(&d), st.path(), &Fake::default(), Some(&emb), &Options::default(), &now()).unwrap();
    assert!(fallback_path(st.path()).exists());
    let st2 = tempfile::tempdir().unwrap();
    run(&open(&d), st2.path(), &Fake::default(), None, &Options::default(), &now()).unwrap();
    assert!(!fallback_path(st2.path()).exists());
}

#[test]
fn indexer_ix_17() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    write_file(d.path(), "story.md", "Orbital Freight story.\n");
    go(&d, st.path(), &fake_for("story.md"), &Options::default());
    let orb = "Claude/Graph/Organizations/Orbital Freight.md";
    assert!(read(&d, orb).contains("[[story|story]]"));
    write_file(d.path(), "story.md", "Rewritten without the company.\n");
    let c = go(&d, st.path(), &Fake::default(), &Options::default());
    assert_eq!(c.mentions_retracted, 1);
    assert!(!read(&d, orb).contains("[[story|"));
}

#[test]
fn indexer_ix_18() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let bad = Fake { fail: HashSet::from(["Daily.md".to_string()]), ..Default::default() };
    for _ in 0..3 {
        go(&d, st.path(), &bad, &Options::default());
    }
    let ok = Fake::default();
    let c = go(&d, st.path(), &ok, &opts(25, false, true));
    assert_eq!(c.failed, 0);
    assert!(ok.calls().contains(&"Daily.md".to_string()));
    assert!(Manifest::load(st.path()).unwrap().failures.is_empty());
    assert_eq!(go(&d, st.path(), &ok, &opts(25, false, true)).processed, 0);
}

/// `enigma-mcp <args>` with a clean ENIGMA_* environment, the given pairs and PATH = an empty folder.
fn cli_without_extractors(args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let empty = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(bin());
    for (k, _) in std::env::vars() {
        if k.starts_with("ENIGMA_") {
            cmd.env_remove(k);
        }
    }
    cmd.args(args).env("PATH", empty.path()).envs(env.iter().copied()).output().unwrap()
}

#[test]
fn indexer_ix_19() {
    // no codex on PATH: each note fails on its own, the run carries on and exits 0 (D1)
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let o = cli_without_extractors(&["index", d.path().to_str().unwrap(), "--batch", "2"], &[("ENIGMA_STATE_DIR", st.path().to_str().unwrap())]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let counts: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((counts["processed"].as_i64(), counts["failed"].as_i64(), counts["remaining"].as_i64()), (Some(0), Some(2), Some(2)));
    let m = Manifest::load(st.path()).unwrap();
    assert_eq!(m.failures.len(), 2);
    for f in m.failures.values() {
        assert_eq!((f.error.as_str(), f.attempts), ("codex CLI not found on PATH", 1));
    }
    assert!(st.path().join("graph.db").exists());
}

#[test]
fn indexer_ix_20() {
    // the backend name is checked when the extractor is created: ExtractorError, exit 1 (D2, D3)
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let v = d.path().to_str().unwrap();
    let state = ("ENIGMA_STATE_DIR", st.path().to_str().unwrap());
    for args in [vec!["index", v, "--backend", "bogus"], vec!["consolidate", v, "--backend", "bogus"]] {
        let o = cli_without_extractors(&args, &[state]);
        assert_eq!(o.status.code(), Some(1), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), "error: ExtractorError: Unknown backend: bogus\n");
    }
    let o = cli_without_extractors(&["index", v], &[state, ("ENIGMA_EXTRACTOR", "")]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "error: ExtractorError: Unknown backend: \n");
}

#[test]
fn indexer_ix_21() {
    // errors never stop a run: no CLI on the first run, CLI present on the next; the notes get indexed and their failures clear
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let (v, state) = (d.path().to_str().unwrap(), ("ENIGMA_STATE_DIR", st.path().to_str().unwrap()));
    let o = cli_without_extractors(&["index", v], &[state]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let first = Manifest::load(st.path()).unwrap();
    assert!(!first.failures.is_empty() && first.failures.values().all(|f| f.attempts == 1));
    let fake = fake_codex_dir();
    let path = format!("{}:/usr/bin:/bin", fake.path().display());
    let o = cli_without_extractors(&["index", v], &[state, ("PATH", &path)]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let counts: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((counts["processed"].as_u64(), counts["failed"].as_i64()), (Some(first.failures.len() as u64), Some(0)));
    let m = Manifest::load(st.path()).unwrap();
    assert!(m.failures.is_empty(), "failure records cleared");
    for path in first.failures.keys() {
        assert!(m.hashes.contains_key(path), "{path} indexed");
    }
    assert!(!Graph::load(&st.path().join("graph.db")).unwrap().find("orbital", None).is_empty());
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) {
    std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
}

#[cfg(unix)] // makes a note unreadable with POSIX mode bits
#[test]
fn indexer_ix_22() {
    // unreadable notes are per-note failures: the run finishes, saves progress and retries them later
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Bad.md"), b"\xff\xfe broken").unwrap();
    write_file(d.path(), "Locked.md", "locked");
    let locked = d.path().join("Locked.md");
    set_mode(&locked, 0o000);
    let ex = fake_for("Daily.md");
    let c = go(&d, st.path(), &ex, &Options::default());
    assert_eq!((c.failed, c.processed), (2, 4));
    let m = Manifest::load(st.path()).unwrap();
    assert!(m.failures["Bad.md"].error.starts_with("DecodeError: invalid UTF-8: byte 0xff at offset 0"), "{}", m.failures["Bad.md"].error);
    assert!(m.failures["Locked.md"].error.starts_with("IoError: "), "{}", m.failures["Locked.md"].error);
    assert!(m.hashes.contains_key("Daily.md") && !m.hashes.contains_key("Bad.md"));
    let c = go(&d, st.path(), &ex, &Options::default());
    assert_eq!((c.failed, c.processed), (2, 0));
    assert_eq!(Manifest::load(st.path()).unwrap().failures["Bad.md"].attempts, 2);
    std::fs::write(d.path().join("Bad.md"), "fixed").unwrap();
    set_mode(&locked, 0o644);
    let c = go(&d, st.path(), &ex, &Options::default());
    assert_eq!((c.failed, c.processed), (0, 2));
    assert!(Manifest::load(st.path()).unwrap().failures.is_empty());
}

#[test]
fn indexer_ix_22_cli() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Bad.md"), b"\xff\xfe broken").unwrap();
    let fake = fake_codex_dir();
    let path = format!("{}:/usr/bin:/bin", fake.path().display());
    let o = cli_without_extractors(&["index", d.path().to_str().unwrap()], &[("ENIGMA_STATE_DIR", st.path().to_str().unwrap()), ("PATH", &path)]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let counts: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((counts["failed"].as_i64(), counts["processed"].as_i64()), (Some(1), Some(4)));
}

/// Fails one path with a non-extractor error.
struct Hiccup;

impl Extract for Hiccup {
    fn extract(&self, path: &str, _content: &str) -> Result<Extraction> {
        if path == "Daily.md" { Err(Error::new(Kind::IoError, "disk hiccup")) } else { Ok(Extraction::default()) }
    }
}

#[test]
fn indexer_ix_23() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    let c = run(&open(&d), st.path(), &Hiccup, None, &Options::default(), &now()).unwrap();
    assert_eq!((c.failed, c.processed), (1, 3));
    let f = &Manifest::load(st.path()).unwrap().failures["Daily.md"];
    assert_eq!((f.error.as_str(), f.attempts), ("IoError: disk hiccup", 1));
}

#[test]
fn indexer_ix_24() {
    let d = mini_vault();
    let st = tempfile::tempdir().unwrap();
    std::fs::write(st.path().join("manifest.json"), "{broken").unwrap();
    let ex = fake_for("Daily.md");
    assert_eq!(run(&open(&d), st.path(), &ex, None, &Options::default(), &now()).unwrap_err().kind, Kind::InvalidData);
    assert!(ex.calls().is_empty(), "nothing extracted");
    // an unloadable cache is rebuilt and the run continues
    let st = tempfile::tempdir().unwrap();
    std::fs::write(st.path().join("graph.db"), "not a database").unwrap();
    let c = go(&d, st.path(), &ex, &Options::default());
    assert_eq!(c.processed, 4);
    assert!(!Graph::load(&st.path().join("graph.db")).unwrap().find("orbital", None).is_empty());
}
