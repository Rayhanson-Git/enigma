//! acceptance/server.yaml: listing, profiles, Phase 1 and Phase 2 tools.
//! Phase 3 cases: SV-15..SV-18 and SV-20.
mod common;
use common::*;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn mv_server(extra: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir, Mcp) {
    let d = mini_vault();
    let state = tempfile::tempdir().unwrap();
    let m = Mcp::vault(d.path(), state.path(), extra);
    (d, state, m)
}

fn names(extra: &[(&str, &str)]) -> BTreeSet<String> {
    let (_d, _s, mut m) = mv_server(extra);
    let n = m.tool_names().into_iter().collect();
    m.close();
    n
}

fn text(r: &Value) -> String {
    texts(r).join("\n")
}

#[test]
fn server_sv_01() {
    let (_d, _s, mut m) = mv_server(&[]);
    let order = m.tool_names();
    assert_eq!(
        order,
        [
            "search_brain", "context_bundle", "organize_vault", "librarian_status", "recall_bundle", "sheet_upsert",
            "sheet_query", "sheet_schema", "undo_move", "read_note", "log_session", "capture", "upsert_concept",
            "write_note", "add_task", "list_tasks", "query_notes", "get_backlinks", "list_recent", "onboard",
            "index_brain", "find_entity", "related_notes", "graph_stats", "consolidate_graph"
        ]
    );
    m.close();
}

#[test]
fn server_sv_02() {
    assert_eq!(
        names(&[("ENIGMA_READONLY", "1")]),
        set(&[
            "search_brain", "read_note", "list_tasks", "query_notes", "get_backlinks", "list_recent", "find_entity",
            "related_notes", "graph_stats", "onboard", "context_bundle", "librarian_status", "recall_bundle", "sheet_query",
            "sheet_schema"
        ])
    );
}

#[test]
fn server_sv_03() {
    // ENIGMA_TOOL_PROFILE is ignored: no restricted profile, no start-up error
    for value in ["restricted", "weird", "full"] {
        assert_eq!(names(&[("ENIGMA_TOOL_PROFILE", value)]).len(), 25, "{value}");
    }
    assert_eq!(names(&[("ENIGMA_TOOL_PROFILE", "restricted"), ("ENIGMA_READONLY", "1")]), names(&[("ENIGMA_READONLY", "1")]));
    let (_d, _s, mut m) = mv_server(&[("ENIGMA_TOOL_PROFILE", " Restricted ")]);
    assert!(!is_error(&m.call("write_note", json!({"path": "Claude/x.md", "content": "x"}))));
    m.close();
}

#[test]
fn server_sv_04() {
    let state = tempfile::tempdir().unwrap();
    let (mut m, _) = Mcp::start(&[("ENIGMA_STATE_DIR", state.path().to_str().unwrap())]);
    for (tool, args) in [("read_note", json!({"path": "Daily.md"})), ("capture", json!({"content": "x"})), ("list_recent", json!({}))] {
        let r = m.call(tool, args);
        assert!(is_error(&r));
        assert!(text(&r).contains("ENIGMA_VAULT_PATH"), "{}", text(&r));
    }
    m.close();
}

#[test]
fn server_sv_05() {
    let (_d, _s, mut m) = mv_server(&[]);
    let r = m.call("search_brain", json!({"query": "ingestion pipeline"}));
    assert_eq!(
        objects(&r)[0],
        json!({"path": "Projects/Lighthouse Grid.md", "excerpt": "Grid outage ingestion pipeline with a queue-free architecture."})
    );
    assert_eq!(r["structuredContent"]["result"][0]["path"], "Projects/Lighthouse Grid.md");
    assert_eq!(objects(&m.call("search_brain", json!({"query": "e", "limit": 1}))).len(), 1);
    // integers given as strings are accepted
    assert_eq!(objects(&m.call("search_brain", json!({"query": "e", "limit": "1"}))).len(), 1);
    m.close();
}

#[test]
fn server_sv_06() {
    let (_d, _s, mut m) = mv_server(&[]);
    assert!(text(&m.call("read_note", json!({"path": "Daily.md"}))).contains("Remember to check"));
    let rel = text(&m.call("log_session", json!({"title": "Probe", "content": "session body", "project": "p", "tags": ["t"]})));
    assert!(text(&m.call("read_note", json!({"path": rel}))).contains("session body"));
    let rel = text(&m.call("capture", json!({"content": "captured thought"})));
    assert!(text(&m.call("read_note", json!({"path": rel}))).contains("captured thought"));
    let rel = text(&m.call("upsert_concept", json!({"name": "Probe Concept", "content": "concept body"})));
    assert_eq!(rel, "Claude/Concepts/Probe Concept.md");
    assert!(text(&m.call("read_note", json!({"path": rel}))).contains("concept body"));
    m.close();
}

#[test]
fn server_sv_07() {
    let (d, _s, mut m) = mv_server(&[]);
    let r = m.call("write_note", json!({"path": "Projects/Injected.md", "content": "x"}));
    assert!(is_error(&r));
    assert!(text(&r).starts_with("Error executing tool write_note: ") && text(&r).contains("outside Claude/"));
    let r = m.call("write_note", json!({"path": "Projects/Injected.md", "content": "x", "confirm_outside_claude": true}));
    assert!(!is_error(&r));
    assert_eq!(text(&r), "Projects/Injected.md");
    assert_eq!(read(&d, "Projects/Injected.md"), "x");
    m.close();
}

#[test]
fn server_sv_08() {
    let (_d, _s, mut m) = mv_server(&[]);
    assert_eq!(text(&m.call("add_task", json!({"content": "torture the tests", "due": "2026-07-09"}))), "Claude/Tasks.md");
    let tasks = objects(&m.call("list_tasks", json!({})));
    assert!(tasks.iter().any(|t| t["text"].as_str().unwrap().starts_with("torture the tests")));
    let notes = objects(&m.call("query_notes", json!({"tags": ["grid"]})));
    assert!(notes.iter().any(|n| n["path"] == "Projects/Lighthouse Grid.md"));
    m.call("capture", json!({"content": "see [[Postgres]] note"}));
    let r = m.call("get_backlinks", json!({"path": "Claude/Concepts/Postgres.md"}));
    assert!(texts(&r).iter().any(|p| p.starts_with("Claude/Inbox/")));
    assert!(!objects(&m.call("list_recent", json!({"n": 3}))).is_empty());
    m.close();
}

#[test]
fn server_sv_12() {
    let state = tempfile::tempdir().unwrap();
    let (m, init) = Mcp::start(&[("ENIGMA_STATE_DIR", state.path().to_str().unwrap())]);
    let s = init["result"]["instructions"].as_str().unwrap();
    assert!(s.contains("Claude/") && s.contains("search_brain") && s.contains("outside Claude/"));
    m.close();
}

#[test]
fn server_sv_13() {
    let (d, s, mut m) = mv_server(&[]);
    let r = m.call("onboard", json!({}));
    let o: Value = serde_json::from_str(&text(&r)).unwrap();
    assert!(o["constitution"].as_str().unwrap().contains("not installed"));
    assert!(o["vault_guide"].as_str().unwrap().contains("not installed"));
    m.close();
    write_file(d.path(), "Claude/README.md", "# Constitution\n");
    write_file(d.path(), "CLAUDE.md", "# Guide\n");
    let mut m = Mcp::vault(d.path(), s.path(), &[]);
    let o: Value = serde_json::from_str(&text(&m.call("onboard", json!({})))).unwrap();
    assert_eq!(o["constitution"], "# Constitution\n");
    assert_eq!(o["vault_guide"], "# Guide\n");
    assert!(o["tools"].as_array().unwrap().iter().any(|t| t.as_str().unwrap().contains("search_brain")));
    assert_eq!(o["graph"], "not built yet \u{2014} call index_brain");
    assert_eq!(o.as_object().unwrap().keys().collect::<Vec<_>>(), ["instructions", "constitution", "vault_guide", "tools", "graph"]);
    m.close();
}

#[test]
fn server_sv_19() {
    // the model loads before serving: without it the server exits before answering
    let empty = tempfile::tempdir().unwrap();
    let out = spawn(&[("HF_HUB_CACHE", empty.path().to_str().unwrap())]).wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.starts_with("enigma-mcp: cannot load embedding model TaylorAI/bge-micro-v2: "), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn server_sv_20() {
    let d = mini_vault();
    write_file(
        d.path(),
        "Things/_schema.md",
        "---\nsheet: things\nfilename: \"{name}\"\nkey: [name]\ncolumns:\n  name: {type: string, required: true}\n  when: {type: date}\n  status: {type: enum, values: [New]}\n---\nFile one note per thing.\n",
    );
    let state = tempfile::tempdir().unwrap();
    let mut m = Mcp::vault(d.path(), state.path(), &[]);
    let r = m.call("sheet_upsert", json!({"sheet": "things", "fields": {"name": "First", "status": "New"}, "agent": "cowork"}));
    let created: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(created["result"], "created");
    let rows = objects(&m.call("sheet_query", json!({"sheet": "things", "filters": {"when": {"missing": true}}})));
    assert_eq!(rows[0]["name"], "First");
    let listing: Value = serde_json::from_str(&text(&m.call("sheet_schema", json!({})))).unwrap();
    assert!(listing.get("things").is_some());
    let row = read(&d, created["path"].as_str().unwrap());
    assert!(row.contains("(agent: cowork)") && row.contains("## Log"));
    let tools = m.request("tools/list", json!({}));
    let t = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "sheet_upsert").unwrap().clone();
    assert!(t["inputSchema"]["properties"]["agent"].is_object());
    m.close();
}

#[test]
fn server_sv_21_refusals() {
    let (_d, _s, mut m) = mv_server(&[("ENIGMA_NO_TOOL_APPLY", "1")]);
    for (tool, args) in [
        ("organize_vault", json!({"apply": true})),
        ("consolidate_graph", json!({"apply": true})),
        ("index_brain", json!({"force": true})),
    ] {
        let r = m.call(tool, args);
        assert!(is_error(&r) && text(&r).contains("ENIGMA_NO_TOOL_APPLY"), "{}", text(&r));
    }
    m.close();
}

// ------------------------------------------------- surface details (02 §1, §3)

fn spec_descriptions() -> HashMap<String, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("spec");
    let mut out = HashMap::new();
    let mut current = None;
    for f in ["02-tools.md", "02-tools-b.md"] {
        for line in std::fs::read_to_string(dir.join(f)).unwrap().lines() {
            if let Some(h) = line.strip_prefix("### 5.") {
                current = h.split_whitespace().nth(1).map(String::from);
            } else if let Some(d) = line.strip_prefix("Description: \u{60}")
                && let Some(name) = current.take()
            {
                out.insert(name, d.trim_end_matches('\u{60}').to_string());
            }
        }
    }
    out
}

#[test]
fn server_descriptions_and_schemas_match_spec() {
    let want = spec_descriptions();
    assert_eq!(want.len(), 25);
    let (_d, _s, mut m) = mv_server(&[]);
    let r = m.request("tools/list", json!({}));
    for t in r["result"]["tools"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        assert_eq!(t["description"].as_str().unwrap(), want[name], "{name}");
        assert_eq!(t["inputSchema"]["type"], "object");
    }
    let tool = |n: &str| r["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == n).unwrap().clone();
    assert_eq!(tool("write_note")["inputSchema"]["required"], json!(["path", "content"]));
    assert_eq!(tool("read_note")["outputSchema"]["properties"]["result"]["type"], "string");
    assert!(tool("onboard").get("outputSchema").is_none());
    m.close();
}

#[test]
fn server_unknown_tool_and_invalid_arguments() {
    let (_d, _s, mut m) = mv_server(&[]);
    let r = m.call("nope", json!({}));
    assert!(is_error(&r));
    assert_eq!(text(&r), "Unknown tool: nope");
    let r = m.call("read_note", json!({}));
    assert!(is_error(&r) && text(&r).starts_with("Error executing tool read_note: invalid arguments: "), "{}", text(&r));
    let r = m.call("list_recent", json!({"n": "lots"}));
    assert!(is_error(&r) && text(&r).contains("invalid arguments"));
    let r = m.call("list_tasks", json!({"include_done": "yes", "folder": null}));
    assert!(!is_error(&r));
    m.close();
}

// ------------------------------------------------------- Phase 2 (graph tools)

/// An MV server whose extractor is the fake codex CLI.
fn graph_server(extra: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir, Mcp) {
    let fake = fake_codex_dir();
    let path = path_with(fake.path());
    let mut env = vec![("PATH", path.as_str()), ("ENIGMA_EXTRACTOR", "codex")];
    env.extend_from_slice(extra);
    let (d, s, m) = mv_server(&env);
    (d, s, fake, m)
}

fn obj(r: &Value) -> Value {
    assert!(!is_error(r), "{}", text(r));
    serde_json::from_str(&text(r)).unwrap()
}

#[test]
fn server_sv_09() {
    let (_d, _s, _f, mut m) = graph_server(&[]);
    let counts = obj(&m.call("index_brain", json!({})));
    assert!(counts["processed"].as_i64().unwrap() > 0);
    assert_eq!(counts["entities_created"], 1);
    assert_eq!(
        counts.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["processed", "entities_created", "entities_merged", "mentions_added", "relations_added", "mentions_retracted", "failed", "skipped", "remaining"]
    );
    assert_eq!(objects(&m.call("find_entity", json!({"query": "orbital"})))[0]["type"], "organization");
    let r = m.call("related_notes", json!({"path": "Projects/Lighthouse Grid.md"}));
    assert!(!is_error(&r) && r["structuredContent"]["result"].is_array());
    assert_eq!(obj(&m.call("graph_stats", json!({}))), json!({"entities": {"organization": 1}, "edges": 0, "mentions": 1}));
    let r = m.call("related_notes", json!({"path": "Projects/Lighthouse Grid.md", "limit": -1}));
    assert_eq!(text(&r), "Error executing tool related_notes: limit must be >= 0, got -1");
    m.close();
}

#[test]
fn server_sv_10() {
    let (_d, _s, mut m) = mv_server(&[]);
    for tool in ["find_entity", "graph_stats"] {
        let r = m.call(tool, json!({"query": "x"}));
        assert!(is_error(&r));
        assert_eq!(text(&r), format!("Error executing tool {tool}: Graph cache not built yet \u{2014} run index_brain first."));
    }
    m.close();
}

#[test]
fn server_sv_11() {
    let (_d, _s, _f, mut m) = graph_server(&[]);
    obj(&m.call("index_brain", json!({})));
    let r = obj(&m.call("consolidate_graph", json!({})));
    assert_eq!(r, json!({"entities": 1, "proposed": [], "applied": false, "merged_entities": 0, "skipped_batches": 0}));
    m.close();
}

#[test]
fn server_sv_14() {
    let (_d, _s, _f, mut m) = graph_server(&[]);
    let cold = obj(&m.call("context_bundle", json!({"query": "ingestion pipeline"})));
    assert!(!cold["hits"].as_array().unwrap().is_empty());
    assert_eq!((cold["entities"].clone(), cold["related_notes"].clone()), (json!([]), json!([])));
    obj(&m.call("index_brain", json!({})));
    let b = obj(&m.call("context_bundle", json!({"query": "ingestion pipeline"})));
    assert_eq!(b["hits"][0]["path"], "Projects/Lighthouse Grid.md");
    assert!(b["entities"].as_array().unwrap().iter().any(|e| e["name"] == "Orbital Freight"));
    assert!(b["related_notes"].is_array());
    m.close();
}

#[test]
fn server_sv_21_dry_run() {
    let (_d, _s, _f, mut m) = graph_server(&[("ENIGMA_NO_TOOL_APPLY", "1")]);
    obj(&m.call("index_brain", json!({})));
    assert_eq!(obj(&m.call("consolidate_graph", json!({})))["applied"], false);
    m.close();
}

#[test]
fn server_concurrent_writes_never_interleave() {
    let (d, _s, mut m) = mv_server(&[]);
    let bodies: Vec<String> = (0..6).map(|i| format!("{}\n", char::from(b'a' + i).to_string().repeat(512 * 1024))).collect();
    let ids: Vec<i64> = bodies
        .iter()
        .map(|b| {
            m.submit("tools/call", json!({"name": "write_note", "arguments": {"path": "Claude/hot.md", "content": b, "overwrite": true}}))
        })
        .collect();
    for id in ids {
        let r = m.response(id);
        assert!(!is_error(&r["result"]), "{r}");
    }
    let final_text = read(&d, "Claude/hot.md");
    assert!(bodies.contains(&final_text), "the note mixes writers' content");
    assert!(temp_writes(d.path()).is_empty());
    m.close();
}

// ------------------------------------------------------- Phase 3 (caretakers)

#[test]
fn server_sv_15() {
    let d = vault_with(&[("02 - Orbit/Orbits.md", "orbit orbits"), ("Loose.md", "orbit loose note")]);
    let state = tempfile::tempdir().unwrap();
    let mut m = Mcp::vault(d.path(), state.path(), &[]);
    let r: Value = serde_json::from_str(&text(&m.call("organize_vault", json!({})))).unwrap();
    assert!(r["moved"].as_array().unwrap().iter().any(|x| x["from"] == "Loose.md" && x["to_folder"] == "02 - Orbit"), "{r}");
    assert!(d.path().join("Loose.md").is_file() && !d.path().join("02 - Orbit/Loose.md").exists());
    m.close();
}

#[test]
fn server_sv_22() {
    for (env, moves) in [(vec![], false), (vec![("ENIGMA_MAX_MOVES", "x")], false), (vec![("ENIGMA_MAX_MOVES", "1")], true)] {
        let d = vault_with(&[("02 - Orbit/Orbits.md", "orbit orbits"), ("Loose.md", "orbit loose note")]);
        let state = tempfile::tempdir().unwrap();
        let mut m = Mcp::vault(d.path(), state.path(), &env);
        let r: Value = serde_json::from_str(&text(&m.call("organize_vault", json!({"apply": true})))).unwrap();
        assert_eq!(d.path().join("02 - Orbit/Loose.md").is_file(), moves, "{r}");
        assert_eq!(d.path().join("Loose.md").is_file(), !moves, "{r}");
        assert_eq!(r["skipped"].as_array().unwrap().iter().any(|x| x["path"] == "Loose.md" && x["reason"] == "move cap reached"), !moves, "{r}");
        m.close();
    }
}

#[test]
fn server_sv_23() {
    // undo_move waits its turn: refused while another maintenance run holds the lock
    let (_d, state, mut m) = mv_server(&[]);
    let held = enigma_mcp::lock::acquire(state.path()).unwrap();
    let r = m.call("undo_move", json!({"path": "02 - Orbit/Nope.md"}));
    assert!(is_error(&r) && text(&r).contains("another maintenance run"), "{}", text(&r));
    drop(held);
    assert!(text(&m.call("undo_move", json!({"path": "02 - Orbit/Nope.md"}))).contains("No undoable move"));
    m.close();
}

#[test]
fn server_sv_16() {
    let (_d, _s, mut m) = mv_server(&[]);
    let r = m.call("undo_move", json!({"path": "02 - Orbit/Nope.md"}));
    assert!(is_error(&r) && text(&r).contains("No undoable move"), "{}", text(&r));
    m.close();
}

#[test]
fn server_sv_17() {
    let (_d, st, mut m) = mv_server(&[]);
    assert_eq!(serde_json::from_str::<Value>(&text(&m.call("librarian_status", json!({})))).unwrap(), json!({"status": "no sweep yet"}));
    std::fs::write(st.path().join("librarian_state.json"), r#"{"last_sweep": "2026-07-09 12:00:00", "steps": {}}"#).unwrap();
    let s: Value = serde_json::from_str(&text(&m.call("librarian_status", json!({})))).unwrap();
    assert_eq!(s["last_sweep"], "2026-07-09 12:00:00");
    m.close();
}

#[test]
fn server_sv_18() {
    let (_d, _s, mut m) = mv_server(&[]);
    let b: Value = serde_json::from_str(&text(&m.call("recall_bundle", json!({"mode": "digest"})))).unwrap();
    assert_eq!(b["mode"], "digest");
    let r = m.call("recall_bundle", json!({"mode": "resume"}));
    assert!(is_error(&r) && text(&r).contains("requires project"));
    let r = m.call("recall_bundle", json!({"mode": "weekly"}));
    assert_eq!(text(&r), "Error executing tool recall_bundle: mode must be 'digest' or 'resume', got 'weekly'");
    m.close();
}
