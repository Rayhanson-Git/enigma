//! acceptance/notes.yaml
mod common;
use common::*;
use enigma_mcp::notes::{capture, frontmatter, log_session, safe_filename, upsert_concept};
use enigma_mcp::vault::{get, parse, str_list};
use enigma_mcp::yaml::Y;

#[test]
fn notes_no_01() {
    assert_eq!(safe_filename("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
}

#[test]
fn notes_no_02() {
    assert_eq!(safe_filename("///"), "untitled");
}

#[test]
fn notes_no_03() {
    assert_eq!(safe_filename("Fix ]] the [[bug"), "Fix the bug");
    assert_eq!(safe_filename("line1\nline2\ttab"), "line1 line2 tab");
}

#[test]
fn notes_no_04() {
    let fm = frontmatter("lighthouse", &["grid".into()], &now(), "claude");
    assert_eq!(fm, "---\ncreated: 2026-07-05 14:30\nagent: claude\nproject: lighthouse\ntags:\n- grid\n---\n\n");
}

#[test]
fn notes_no_05() {
    let d = mini_vault();
    let rel = log_session(&open(&d), "Relay setup", "We configured the relay.", "enigma", &["infra".into()], &now(), "claude").unwrap();
    assert_eq!(rel, "Claude/Sessions/2026-07-05 Relay setup.md");
    let note = read(&d, &rel);
    assert!(note.contains("agent: claude") && note.contains("We configured the relay."));
    let index = read(&d, "Claude/Index.md");
    assert!(index.contains("[[2026-07-05 Relay setup]]") && index.contains("enigma"));
    assert!(index.ends_with("- [[2026-07-05 Relay setup]] \u{2014} enigma: Relay setup\n"));
}

#[test]
fn notes_no_06() {
    let d = mini_vault();
    let v = open(&d);
    assert_eq!(capture(&v, "check bucket pricing", &now()).unwrap(), "Claude/Inbox/2026-07-05.md");
    assert_eq!(read(&d, "Claude/Inbox/2026-07-05.md"), "- 14:30 check bucket pricing\n");
    capture(&v, "check bucket pricing", &now()).unwrap();
    assert_eq!(read(&d, "Claude/Inbox/2026-07-05.md").matches("- 14:30").count(), 2);
}

#[test]
fn notes_no_07() {
    let d = mini_vault();
    let rel = upsert_concept(&open(&d), "DynDNS", "Free dynamic DNS.", &now(), "claude").unwrap();
    assert_eq!(rel, "Claude/Concepts/DynDNS.md");
    let t = read(&d, &rel);
    assert!(t.starts_with("---\n") && t.contains("# DynDNS") && t.contains("Free dynamic DNS."));
}

#[test]
fn notes_no_08() {
    let d = mini_vault();
    let rel = upsert_concept(&open(&d), "Postgres", "Used by the relay.", &now(), "claude").unwrap();
    assert_eq!(rel, "Claude/Concepts/Postgres.md");
    let t = read(&d, &rel);
    assert!(t.contains("Relational database used by the sync relay."));
    assert!(t.ends_with("\n## Update 2026-07-05\n\nUsed by the relay.\n"));
}

#[test]
fn notes_no_09() {
    let d = mini_vault();
    let v = open(&d);
    let a = log_session(&v, "Standup", "one", "p", &[], &now(), "claude").unwrap();
    let b = log_session(&v, "Standup", "two", "p", &[], &now(), "claude").unwrap();
    assert_eq!(a, "Claude/Sessions/2026-07-05 Standup.md");
    assert_eq!(b, "Claude/Sessions/2026-07-05 Standup 2.md");
    let index = read(&d, "Claude/Index.md");
    assert!(index.contains("[[2026-07-05 Standup]]") && index.contains("[[2026-07-05 Standup 2]]"));
}

#[test]
fn notes_no_10() {
    let d = mini_vault();
    let rel = upsert_concept(&open(&d), "postgres", "Lower-case update.", &now(), "claude").unwrap();
    assert_eq!(rel, "Claude/Concepts/Postgres.md");
    assert!(read(&d, &rel).contains("Lower-case update."));
    let same: Vec<_> = std::fs::read_dir(d.path().join("Claude/Concepts"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_lowercase() == "postgres.md")
        .collect();
    assert_eq!(same.len(), 1);
}

#[test]
fn notes_no_11() {
    let tags = vec!["odd\"tag".to_string(), "b:c".to_string()];
    let fm = frontmatter("grid: outage [pipeline]", &tags, &now(), "claude");
    let meta = parse(&fm).unwrap();
    assert_eq!(get(&meta, "project"), Some(&Y::Str("grid: outage [pipeline]".into())));
    assert_eq!(str_list(get(&meta, "tags")), tags);
    assert_eq!(get(&meta, "created"), Some(&Y::Str("2026-07-05 14:30".into())));
}
