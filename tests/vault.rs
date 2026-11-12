//! acceptance/vault.yaml
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::faults::{self, Fault};
use enigma_mcp::vault::Vault;
use std::fs;
use std::sync::{Arc, Barrier};

// A lock error, a "no hard links here" error and a plain I/O error, as each system numbers them.
#[cfg(unix)]
const EACCES: i32 = 13;
#[cfg(unix)]
const EIO: i32 = 5;
#[cfg(windows)]
const EACCES: i32 = 5; // ERROR_ACCESS_DENIED
#[cfg(windows)]
const EIO: i32 = 1117; // ERROR_IO_DEVICE
const EPERM: i32 = 1; // Windows: ERROR_INVALID_FUNCTION, which also means "fall back"

#[test]
fn vault_va_01() {
    let e = Vault::open(std::path::Path::new("/definitely/not/here")).err().unwrap();
    assert!(e.msg.contains("does not exist"), "{}", e.msg);
}

#[test]
fn vault_va_02() {
    let d = mini_vault();
    assert!(open(&d).read("Daily.md").unwrap().contains("Remember to check"));
}

#[test]
fn vault_va_03() {
    let d = mini_vault();
    assert!(open(&d).read("Ghost.md").unwrap_err().msg.contains("not found"));
}

#[test]
fn vault_va_04() {
    let d = mini_vault();
    assert!(open(&d).read("../outside.md").unwrap_err().msg.contains("escapes"));
}

#[test]
fn vault_va_05() {
    let d = mini_vault();
    let v = open(&d);
    v.write("Claude/Inbox/note.md", "hello", false, false).unwrap();
    assert_eq!(v.read("Claude/Inbox/note.md").unwrap(), "hello");
}

#[test]
fn vault_va_06() {
    let d = mini_vault();
    let e = open(&d).write("Projects/New.md", "x", false, false).unwrap_err();
    assert!(e.msg.contains("outside Claude/"));
    assert!(!d.path().join("Projects/New.md").exists());
}

#[test]
fn vault_va_07() {
    let d = mini_vault();
    open(&d).write("Projects/New.md", "x", false, true).unwrap();
    assert_eq!(read(&d, "Projects/New.md"), "x");
}

#[test]
fn vault_va_08() {
    let d = mini_vault();
    let v = open(&d);
    v.write("Claude/x.md", "a", false, false).unwrap();
    let e = v.write("Claude/x.md", "b", false, false).unwrap_err();
    assert!(e.msg.contains("already exists"));
    assert_eq!(e.kind, Kind::NoteExists);
    assert!(e.is(Kind::VaultError));
    assert_eq!(read(&d, "Claude/x.md"), "a");
}

#[test]
fn vault_va_09() {
    let d = mini_vault();
    let v = open(&d);
    v.write("Claude/x.md", "v1", false, false).unwrap();
    v.write("Claude/x.md", "v2", true, false).unwrap();
    assert_eq!(read(&d, "Claude/x.md"), "v2");
}

#[test]
fn vault_va_10() {
    let d = mini_vault();
    open(&d).write("Claude/Sessions/2026/deep.md", "x", false, false).unwrap();
    assert!(d.path().join("Claude/Sessions/2026/deep.md").is_file());
}

#[test]
fn vault_va_11() {
    let d = mini_vault();
    let v = open(&d);
    v.append("Claude/Inbox/2026-07-05.md", "- one\n", false).unwrap();
    v.append("Claude/Inbox/2026-07-05.md", "- two\n", false).unwrap();
    assert_eq!(read(&d, "Claude/Inbox/2026-07-05.md"), "- one\n- two\n");
}

#[test]
fn vault_va_12() {
    let d = mini_vault();
    assert!(open(&d).append("Daily.md", "x", false).unwrap_err().msg.contains("outside Claude/"));
}

#[test]
fn vault_va_13() {
    let d = mini_vault();
    let v = open(&d);
    let inside = |rel: &str| v.in_claude(&v.resolve(rel).unwrap());
    assert!(inside("Claude/Index.md"));
    assert!(inside("Claude/Sessions/x.md"));
    assert!(inside("Claude"));
    assert!(!inside("Daily.md"));
    assert!(!inside("ClaudeFake/x.md"));
    assert!(!inside("Claude/../Daily.md"));
    // POSIX compares exactly, so other spellings are outside Claude/; Windows ignores case (03 §3)
    assert_eq!(inside("claude/Sessions/x.md"), cfg!(windows));
    assert_eq!(inside("CLAUDE/Index.md"), cfg!(windows));
}

#[test]
fn vault_va_14() {
    let d = mini_vault();
    let v = open(&d);
    for e in [
        v.write("Claude/Sessions", "x", false, false).unwrap_err(),
        v.write("", "x", false, true).unwrap_err(),
        v.append("Claude/Inbox", "x", false).unwrap_err(),
        v.read("Claude/Sessions").unwrap_err(),
    ] {
        assert!(e.msg.contains("directory"), "{}", e.msg);
    }
}

#[test]
fn vault_va_15() {
    faults::reset();
    let d = mini_vault();
    faults::arm("read", Fault::Os(EACCES), 2);
    assert!(open(&d).read("Daily.md").unwrap().contains("Remember"));
    assert_eq!(faults::calls("read"), 3);
}

#[test]
fn vault_va_16() {
    let d = mini_vault();
    let v = open(&d);
    v.write("Claude/a.md", "1", false, false).unwrap();
    v.write("Claude/a.md", "2", true, false).unwrap();
    assert!(temp_writes(d.path()).is_empty());
}

#[test]
fn vault_va_17() {
    let d = mini_vault();
    let v = open(&d);
    let text = "line one\nline two\nline three\n";
    v.write("Claude/w.md", text, false, false).unwrap();
    v.append("Claude/a.md", text, false).unwrap();
    assert_eq!(fs::read(d.path().join("Claude/w.md")).unwrap(), text.as_bytes());
    assert_eq!(fs::read(d.path().join("Claude/a.md")).unwrap(), text.as_bytes());
}

#[test]
fn vault_va_18() {
    faults::reset();
    let d = mini_vault();
    let v = open(&d);
    faults::arm("publish", Fault::Os(EACCES), 2);
    v.write("Claude/c.md", "create", false, false).unwrap();
    assert_eq!(faults::calls("publish"), 3);
    faults::arm("publish", Fault::Os(EACCES), 2);
    v.write("Claude/c.md", "replace", true, false).unwrap();
    assert_eq!(faults::calls("publish"), 6);
    assert_eq!(read(&d, "Claude/c.md"), "replace");
    assert!(temp_writes(d.path()).is_empty());
}

#[test]
fn vault_va_19() {
    faults::reset();
    let d = mini_vault();
    let v = open(&d);
    faults::arm("publish", Fault::Os(EACCES), 5);
    let e = v.write("Claude/c.md", "create", false, false).unwrap_err();
    assert_eq!(e.kind, Kind::IoError);
    assert_eq!(faults::calls("publish"), 5);
    v.write("Claude/c.md", "old", false, false).unwrap();
    faults::arm("publish", Fault::Os(EACCES), 5);
    assert_eq!(v.write("Claude/c.md", "new", true, false).unwrap_err().kind, Kind::IoError);
    assert_eq!(read(&d, "Claude/c.md"), "old");
    assert!(temp_writes(d.path()).is_empty());
}

#[test]
fn vault_va_20() {
    let d = mini_vault();
    let root = d.path().to_path_buf();
    let gate = Arc::new(Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let (root, gate) = (root.clone(), gate.clone());
            std::thread::spawn(move || {
                faults::reset();
                faults::arm("publish", Fault::Os(EPERM), 1); // no hard links on this file system
                let v = Vault::open(&root).unwrap();
                let content = format!("writer {i}\nline two\n");
                gate.wait();
                let r = v.write("Claude/race.md", &content, false, false);
                (r.map(|_| content), faults::calls("publish"))
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // writers that see the winner's file early stop before publishing (0 calls)
    assert!(results.iter().all(|(_, calls)| *calls <= 1), "the no-hard-link condition is never retried");
    let winners: Vec<&String> = results.iter().filter_map(|(r, _)| r.as_ref().ok()).collect();
    assert_eq!(winners.len(), 1);
    assert!(results.iter().filter_map(|(r, _)| r.as_ref().err()).all(|e| e.kind == Kind::NoteExists));
    assert_eq!(&read(&d, "Claude/race.md"), winners[0]);
    assert!(temp_writes(d.path()).is_empty());
}

#[test]
fn vault_va_21() {
    let d = mini_vault();
    let v = open(&d);
    let rel = format!("Claude/{}.md", "a".repeat(237));
    v.write(&rel, "one", false, false).unwrap();
    v.write(&rel, "two", true, false).unwrap();
    assert_eq!(read(&d, &rel), "two");
}

#[test]
fn vault_va_22() {
    faults::reset();
    let d = mini_vault();
    faults::arm("temp_delete", Fault::Os(EIO), 1);
    open(&d).write("Claude/t.md", "kept", false, false).unwrap();
    assert_eq!(read(&d, "Claude/t.md"), "kept");
}

#[test]
fn vault_va_23() {
    faults::reset();
    let d = mini_vault();
    faults::arm("append_open", Fault::Os(EACCES), 2);
    open(&d).append("Claude/Inbox/c.md", "- captured\n", false).unwrap();
    assert_eq!(faults::calls("append_open"), 3);
    assert_eq!(read(&d, "Claude/Inbox/c.md"), "- captured\n");
}

#[test]
fn vault_va_24() {
    faults::reset();
    let d = mini_vault();
    faults::arm("append_open", Fault::Os(EACCES), 5);
    let e = open(&d).append("Claude/Inbox/c.md", "- captured\n", false).unwrap_err();
    assert_eq!(e.kind, Kind::IoError);
    assert_eq!(faults::calls("append_open"), 5);
}

#[test]
fn vault_va_25() {
    faults::reset();
    let d = mini_vault();
    faults::arm("append_write", Fault::Short(5), 1);
    faults::arm("append_write", Fault::Os(EIO), 1);
    assert!(open(&d).append("Claude/Inbox/c.md", "- captured\n", false).is_err());
    assert_eq!(faults::calls("append_open"), 1);
    assert_eq!(faults::calls("append_write"), 2);
    assert_eq!(read(&d, "Claude/Inbox/c.md"), "- cap");
}

// ------------------------------------------- symlinks (review finding 6)

#[test]
fn vault_symlinks_cannot_escape() {
    let d = mini_vault();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "SECRET").unwrap();
    symlink(outside.path().join("secret.txt"), &d.path().join("leak.md"));
    symlink(outside.path(), &d.path().join("Claude/out"));
    let v = open(&d);
    for e in [
        v.read("leak.md").unwrap_err(),
        v.write("leak.md", "x", true, true).unwrap_err(),
        v.append("leak.md", "x", true).unwrap_err(),
        v.read("Claude/out/secret.txt").unwrap_err(),
        v.write("Claude/out/x.md", "x", false, false).unwrap_err(),
        v.append("Claude/out/x.md", "x", false).unwrap_err(),
    ] {
        assert!(e.msg.contains("escapes the vault"), "{}", e.msg);
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1, "nothing was written outside");
    assert_eq!(fs::read_to_string(outside.path().join("secret.txt")).unwrap(), "SECRET");
}

#[test]
fn vault_symlink_into_human_area_is_quarantined() {
    let d = mini_vault();
    symlink("../Projects", &d.path().join("Claude/human"));
    let e = open(&d).write("Claude/human/x.md", "x", false, false).unwrap_err();
    assert!(e.msg.contains("outside Claude/"), "{}", e.msg);
    assert!(!d.path().join("Projects/x.md").exists());
}
