//! acceptance/multi_agent.yaml
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::faults::{self, Fault};
use enigma_mcp::notes::{SEED, add_task, log_session, upsert_concept};
use enigma_mcp::vault::Vault;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

/// Runs f(i) on n threads released together; each opens its own Vault.
fn race<T: Send + 'static>(root: &Path, n: usize, f: fn(&Vault, usize) -> T) -> Vec<T> {
    let gate = Arc::new(Barrier::new(n));
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (root, gate): (PathBuf, _) = (root.to_path_buf(), gate.clone());
            std::thread::spawn(move || {
                let v = Vault::open(&root).unwrap();
                gate.wait();
                f(&v, i)
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

#[test]
fn multi_agent_ma_01() {
    let d = mini_vault();
    let results = race(d.path(), 16, |v, i| v.write("Claude/shared.md", &format!("writer {i}\n"), false, false));
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(read(&d, "Claude/shared.md").starts_with("writer "));
    assert!(temp_writes(d.path()).is_empty());
}

#[test]
fn multi_agent_ma_02() {
    let d = mini_vault();
    let v = open(&d);
    v.write("Claude/x.md", "a", false, false).unwrap();
    let e = v.write("Claude/x.md", "a", false, false).unwrap_err();
    assert!(e.is(Kind::VaultError) && e.msg.contains("already exists"));
}

#[test]
fn multi_agent_ma_03() {
    let d = mini_vault();
    let paths = race(d.path(), 8, |v, i| log_session(v, "Standup", &format!("body {i}"), "p", &[], &now(), "claude").unwrap());
    let mut uniq = paths.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 8);
    let all: String = paths.iter().map(|p| read(&d, p)).collect();
    for i in 0..8 {
        assert_eq!(all.matches(&format!("body {i}\n")).count(), 1);
    }
    assert_eq!(read(&d, "Claude/Index.md").matches("Standup").count(), 16);
}

#[test]
fn multi_agent_ma_04() {
    let state = tempfile::tempdir().unwrap();
    for (agent, want) in [(Some("scout"), "agent: scout"), (None, "agent: claude")] {
        let d = mini_vault();
        let mut env = vec![];
        if let Some(a) = agent {
            env.push(("ENIGMA_AGENT_NAME", a));
        }
        let mut m = Mcp::vault(d.path(), state.path(), &env);
        let r = m.call("log_session", json!({"title": "Who", "content": "c", "project": "p"}));
        let rel = texts(&r)[0].clone();
        assert!(read(&d, &rel).contains(want), "{rel}");
        m.close();
    }
}

#[test]
fn multi_agent_ma_05() {
    let d = mini_vault();
    let rel = upsert_concept(&open(&d), "Postgres", "Researcher notes.", &now(), "researcher").unwrap();
    let t = read(&d, &rel);
    assert!(t.contains("## Update 2026-07-05 (researcher)"));
    assert!(t.contains("Relational database used by the sync relay.") && t.contains("Researcher notes."));
}

#[test]
fn multi_agent_ma_06() {
    let d = mini_vault();
    let paths = race(d.path(), 8, |v, i| upsert_concept(v, "Shared Idea", &format!("from {i}"), &now(), "claude").unwrap());
    assert!(paths.iter().all(|p| p == "Claude/Concepts/Shared Idea.md"));
    let t = read(&d, &paths[0]);
    for i in 0..8 {
        assert!(t.contains(&format!("from {i}\n")));
    }
}

#[test]
fn multi_agent_ma_07() {
    faults::reset();
    let d = mini_vault();
    write_file(d.path(), "Claude/Concepts/Idea.md", "original idea\n");
    faults::arm("concept_lookup", Fault::Os(0), 2); // both lookups miss the file
    let rel = upsert_concept(&open(&d), "Idea", "new text", &now(), "claude").unwrap();
    assert_eq!(rel, "Claude/Concepts/Idea.md");
    let t = read(&d, &rel);
    assert!(t.starts_with("original idea\n") && t.ends_with("new text\n"));
}

#[test]
fn multi_agent_ma_08() {
    let d = mini_vault();
    race(d.path(), 16, |v, a| {
        for l in 0..25 {
            v.append("Claude/Inbox/shared.md", &format!("agent {a} line {l}\n"), false).unwrap();
        }
    });
    let t = read(&d, "Claude/Inbox/shared.md");
    assert_eq!(t.lines().count(), 400);
    for a in 0..16 {
        for l in 0..25 {
            assert_eq!(t.lines().filter(|x| *x == format!("agent {a} line {l}")).count(), 1);
        }
    }
}

#[test]
fn multi_agent_ma_09() {
    let d = mini_vault();
    race(d.path(), 8, |v, i| add_task(v, &format!("task {i}"), None).unwrap());
    let t = read(&d, "Claude/Tasks.md");
    assert!(t.starts_with(SEED));
    assert_eq!(t.matches("# Tasks").count(), 1);
    for i in 0..8 {
        assert!(t.contains(&format!("- [ ] task {i}\n")));
    }
}

const THINGS: &str = "---\nsheet: things\nfilename: \"{name}\"\nkey: [name]\ncolumns:\n  name: {type: string, required: true}\n---\nOne note per thing.\n";

fn things() -> tempfile::TempDir {
    let d = mini_vault();
    write_file(d.path(), "Things/_schema.md", THINGS);
    d
}

#[test]
fn multi_agent_ma_10() {
    let d = things();
    let state = tempfile::tempdir().unwrap();
    let mut m = Mcp::vault(d.path(), state.path(), &[("ENIGMA_AGENT_NAME", "scout")]);
    let r = m.call("sheet_upsert", json!({"sheet": "things", "fields": {"name": "Env"}}));
    let path = serde_json::from_str::<serde_json::Value>(&texts(&r)[0]).unwrap()["path"].as_str().unwrap().to_string();
    assert!(read(&d, &path).contains("agent: scout"));
    let r = m.call("sheet_upsert", json!({"sheet": "things", "fields": {"name": "Explicit"}, "agent": "cowork"}));
    let path = serde_json::from_str::<serde_json::Value>(&texts(&r)[0]).unwrap()["path"].as_str().unwrap().to_string();
    assert!(read(&d, &path).contains("agent: cowork"));
    m.close();
}

#[test]
fn multi_agent_ma_11() {
    let d = things();
    let results = race(d.path(), 8, |v, _| {
        let fields = vec![(enigma_mcp::yaml::Y::Str("name".into()), enigma_mcp::yaml::Y::Str("Same".into()))];
        enigma_mcp::sheets::upsert(v, "things", &fields, None, "claude", &now()).unwrap()["result"].as_str().unwrap().to_string()
    });
    assert_eq!(results.iter().filter(|r| *r == "created").count(), 1);
    assert_eq!(results.iter().filter(|r| *r == "updated").count(), 7);
    let mut names: Vec<String> = std::fs::read_dir(d.path().join("Things")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["Same.md", "_schema.md"]);
}

#[test]
fn multi_agent_ma_12() {
    let d = mini_vault();
    write_file(d.path(), "Things/_schema.md", &THINGS.replace("filename: \"{name}\"", "filename: \"Same\""));
    write_file(d.path(), "Things/Same.md", "---\nname: Other\n---\n\nKeep me.\n");
    let fields = vec![(enigma_mcp::yaml::Y::Str("name".into()), enigma_mcp::yaml::Y::Str("New".into()))];
    let r = enigma_mcp::sheets::upsert(&open(&d), "things", &fields, None, "claude", &now()).unwrap();
    assert_eq!(r["path"], "Things/Same 2.md");
    assert_eq!(read(&d, "Things/Same.md"), "---\nname: Other\n---\n\nKeep me.\n");
}
