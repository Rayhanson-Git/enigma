//! acceptance/tasks.yaml
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::notes::{SEED, add_task, list_tasks};
use serde_json::json;

#[test]
fn tasks_tk_01() {
    let d = mini_vault();
    assert_eq!(add_task(&open(&d), "review relay logs", None).unwrap(), "Claude/Tasks.md");
    let t = read(&d, "Claude/Tasks.md");
    assert!(t.starts_with("---\n") && t.contains("# Tasks") && t.contains("\u{60}\u{60}\u{60}tasks\n"));
    assert_eq!(t, format!("{SEED}- [ ] review relay logs\n"));
}

#[test]
fn tasks_tk_02() {
    let d = mini_vault();
    add_task(&open(&d), "deploy VM", Some("2026-07-10")).unwrap();
    assert!(read(&d, "Claude/Tasks.md").contains("\n- [ ] deploy VM \u{1F4C5} 2026-07-10\n"));
}

#[test]
fn tasks_tk_03() {
    let d = mini_vault();
    let e = add_task(&open(&d), "x", Some("tomorrow")).unwrap_err();
    assert_eq!(e.kind, Kind::InvalidArgument);
    assert_eq!(e.msg, "time data 'tomorrow' does not match format '%Y-%m-%d'");
    for (due, msg) in [
        ("2026-07-100", "unconverted data remains: 0"),
        ("2026-02-30", "day is out of range for month"),
        ("2026-7-5x", "unconverted data remains: x"),
    ] {
        assert_eq!(add_task(&open(&d), "x", Some(due)).unwrap_err().msg, msg);
    }
    add_task(&open(&d), "lenient", Some("2026-7-5")).unwrap();
}

#[test]
fn tasks_tk_04() {
    let d = mini_vault();
    let v = open(&d);
    add_task(&v, "first", None).unwrap();
    add_task(&v, "second", None).unwrap();
    let t = read(&d, "Claude/Tasks.md");
    assert!(t.contains("- [ ] first\n- [ ] second\n"));
}

#[test]
fn tasks_tk_05() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Inbox/todo.md", "- [ ] inbox task\n- [x] done task\n");
    let v = open(&d);
    add_task(&v, "open one", None).unwrap();
    let open_tasks = list_tasks(&v, false, None).unwrap();
    let texts: Vec<&str> = open_tasks.iter().map(|t| t["text"].as_str().unwrap()).collect();
    assert!(texts.contains(&"open one") && texts.contains(&"inbox task") && !texts.contains(&"done task"));
    let all = list_tasks(&v, true, None).unwrap();
    assert!(all.contains(&json!({"path": "Claude/Inbox/todo.md", "text": "done task", "done": true})));
}

#[test]
fn tasks_tk_06() {
    let d = mini_vault();
    write_file(d.path(), "Projects/todo.md", "- [ ] outside task\n");
    let v = open(&d);
    add_task(&v, "inside", None).unwrap();
    let tasks = list_tasks(&v, true, Some("Claude")).unwrap();
    assert!(!tasks.is_empty());
    assert!(tasks.iter().all(|t| t["path"].as_str().unwrap().starts_with("Claude/")));
}

#[test]
fn tasks_tk_07() {
    let d = mini_vault();
    add_task(&open(&d), "line one\nline two\t end", None).unwrap();
    assert_eq!(read(&d, "Claude/Tasks.md"), format!("{SEED}- [ ] line one line two end\n"));
}
