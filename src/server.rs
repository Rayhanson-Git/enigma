//! MCP server surface (02): tool table, profiles, argument handling, results.
use crate::config::{self, Profile};
use crate::embed::Embedder;
use crate::error::{Error, Kind, Result};
use crate::json::{mcp_text, structured};
use crate::vault::Vault;
use crate::extractor::Cli;
use crate::graph::{self, Graph};
use crate::{consolidate, indexer, librarian, lock, mover, notes, organizer, recall, search, sheets, text};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Ty {
    Str,
    Int,
    Bool,
    StrArray,
    Object,
}

struct Param {
    name: &'static str,
    ty: Ty,
    /// None = required; Some(Value::Null) = nullable, default null.
    default: Option<Value>,
}

#[derive(Clone, Copy, PartialEq)]
enum Out {
    Text,
    TextList,
    ObjList,
    Obj,
}

struct ToolDef {
    name: &'static str,
    desc: &'static str,
    params: Vec<Param>,
    out: Out,
    read_only: bool,
}

fn req(name: &'static str, ty: Ty) -> Param {
    Param { name, ty, default: None }
}
fn opt(name: &'static str, ty: Ty) -> Param {
    Param { name, ty, default: Some(Value::Null) }
}
fn def(name: &'static str, ty: Ty, v: Value) -> Param {
    Param { name, ty, default: Some(v) }
}

/// Every tool of 02 in registration order (02 §1, membership 02 §4).
fn tool_defs() -> Vec<ToolDef> {
    use Out::*;
    use Ty::*;
    let t = |name, desc, params, out, read_only| ToolDef { name, desc, params, out, read_only };
    vec![
        t("search_brain", "Find notes by exact words and by meaning at once: a keyword ranking and an embedding ranking are merged into one list. Narrow it with frontmatter tags or a folder. Each result is a note path with a short excerpt, most relevant first.",
          vec![req("query", Str), opt("tags", StrArray), opt("folder", Str), def("limit", Int, json!(20))], ObjList, true),
        t("context_bundle", "Search plus graph context in a single call. Returns the best-matching notes for the query, the entities those notes mention, and other notes reachable from those entities in up to hops steps.",
          vec![req("query", Str), def("hops", Int, json!(2)), def("limit", Int, json!(10))], Obj, true),
        t("organize_vault", "Suggest where loose notes belong by comparing each one with its most similar filed notes. The default is a preview that changes nothing and lists moved, proposals and skipped. With apply=true the suggestion list is refreshed and no more than ENIGMA_MAX_MOVES notes are moved (0 unless the owner raises it); every move is journaled and undo_move reverses it.",
          vec![def("apply", Bool, json!(false))], Obj, false),
        t("librarian_status", "Report from the most recent Librarian run: what each step did, the health checks, and how many proposals are waiting. Changes nothing; the Librarian itself is started on a schedule with enigma-mcp librarian.",
          vec![], Obj, true),
        t("recall_bundle", "Collect, without calling a model, the facts the recall skills build on. mode='digest' lists what changed after a date (7 days ago when omitted). mode='resume' gathers session logs, decisions, open tasks and graph entities whose project contains the given text, ignoring case. A section that fails does not stop the others.",
          vec![req("mode", Str), opt("project", Str), opt("since", Str)], Obj, true),
        t("sheet_upsert", "Add a row to a sheet, or update the row that is already there. A sheet is a folder the owner marked with a _schema.md file. Fields are checked against that schema, the existing row is located by its key and identity columns, and only the fields you pass are changed. agent is the name recorded on the row; it defaults to this server's ENIGMA_AGENT_NAME.",
          vec![req("sheet", Str), req("fields", Object), opt("body", Str), opt("agent", Str)], Obj, false),
        t("sheet_query", "Filter and sort the rows of a sheet. filters maps a column to {op: value}; the ops are eq, ne, lt, lte, gt, gte, contains, missing, in and nin, and every filter must hold. sort is {by, dir}. limit caps the number of rows.",
          vec![req("sheet", Str), opt("filters", Object), opt("sort", Object), def("limit", Int, json!(50))], ObjList, true),
        t("sheet_schema", "Describe sheets. Called with no sheet, it lists every sheet with its folder and number of rows. Called with a sheet, it returns that sheet's columns, types, key, identity columns and filing instructions.",
          vec![opt("sheet", Str)], Obj, true),
        t("undo_move", "Put a note the organizer moved back where it was. Pass the note's present path inside the vault; the latest move of that note is reversed, including the links that were rewritten and the note's indexing record.",
          vec![req("path", Str)], Obj, false),
        t("read_note", "Return the text of one note. path is relative to the vault root, for example 'Claude/Index.md'.", vec![req("path", Str)], Text, true),
        t("log_session", "Save a session log as a new note in Claude/Sessions/ and list it in Claude/Index.md. Call it when a meaningful piece of work ends, and say what happened, what was learned and what was decided.",
          vec![req("title", Str), req("content", Str), req("project", Str), opt("tags", StrArray)], Text, false),
        t("capture", "Add one timestamped line to today's note in Claude/Inbox/, for thoughts worth keeping that do not need a note of their own yet.", vec![req("content", Str)], Text, false),
        t("upsert_concept", "Keep one lasting note per concept in Claude/Concepts/. A new name creates the note; a name that already exists (letter case ignored) gets a dated update appended, so nothing is duplicated or replaced. Search before calling.",
          vec![req("name", Str), req("content", Str)], Text, false),
        t("write_note", "Write a note at any path. Paths outside Claude/ are rejected unless confirm_outside_claude=true, and that flag is only for writes the user asked for in so many words. An existing note is kept unless overwrite=true.",
          vec![req("path", Str), req("content", Str), def("confirm_outside_claude", Bool, json!(false)), def("overwrite", Bool, json!(false))], Text, false),
        t("add_task", "Append an open checkbox task to Claude/Tasks.md, in the format the Obsidian Tasks plugin reads. due is optional and must be YYYY-MM-DD.",
          vec![req("content", Str), opt("due", Str)], Text, false),
        t("list_tasks", "Collect checkbox tasks from every note in the vault, or from one folder. Finished tasks are left out unless include_done=true.",
          vec![def("include_done", Bool, json!(false)), opt("folder", Str)], ObjList, true),
        t("query_notes", "Select notes by their frontmatter: project, tags, folder, or any mix of them. Each result is a note path with its frontmatter. With no filter it returns the notes that have any frontmatter at all.",
          vec![opt("project", Str), opt("tags", StrArray), opt("folder", Str), def("limit", Int, json!(50))], ObjList, true),
        t("get_backlinks", "Which notes link to this one with a [[wikilink]]? Useful for seeing what already depends on a topic before you add to it.",
          vec![req("path", Str)], TextList, true),
        t("list_recent", "The n notes changed most recently, latest first. A quick way to see what has been happening in the vault.",
          vec![def("n", Int, json!(10))], ObjList, true),
        t("onboard", "Start every new session here. Returns the vault's constitution, its guide to where things go, a short list of the tools, and the state of the graph.",
          vec![], Obj, true),
        t("index_brain", "Read notes that are new or have changed and add their entities and relations to the graph, using the configured model CLI. The result includes a remaining count; repeat the call until it is 0. force=true processes every note again.",
          vec![def("force", Bool, json!(false))], Obj, false),
        t("find_entity", "Search the graph's entities by part of a name or alias, ignoring case. type optionally limits the result to person, organization, domain, topic, project or source.",
          vec![req("query", Str), opt("type", Str)], ObjList, true),
        t("related_notes", "Notes that share graph entities with the given note, directly or through up to hops steps. Strongest connections come first, at most limit results, and each one shows the chain of entities that links it.",
          vec![req("path", Str), def("hops", Int, json!(2)), def("limit", Int, json!(20))], ObjList, true),
        t("graph_stats", "How many entities, edges and mentions the graph holds.", vec![], Obj, true),
        t("consolidate_graph", "Ask a model which graph entities are the same thing under different names. The default is a preview that returns the suggested merges. Review them, then call with apply=true to fold each group into one canonical entity.",
          vec![def("apply", Bool, json!(false))], Obj, false),
    ]
}

const INSTRUCTIONS: [&str; 4] = [
    "Enigma is the long-term memory this vault's owner shares with their AI agents: one Obsidian vault that all of them read and write.",
    "First step in any new session: call \u{60}onboard\u{60}. Before your first write, read Claude/README.md; it is the constitution and its rules bind every agent.",
    "Where things live: agents write only inside Claude/. Sessions/ holds work logs, Concepts/ holds lasting topic notes, Inbox/ holds quick captures, Tasks.md holds checkbox tasks, Decisions.md is a decision log that is only ever appended to, and Graph/ holds the entity graph. The rest of the vault is the owner's. Read any of it, but write outside Claude/ only on an explicit request; without confirm_outside_claude such a write is rejected.",
    "Working habits: look before you conclude something is unknown (search_brain, query_notes, related_notes, find_entity). When a meaningful piece of work ends, call log_session. Use capture for passing thoughts, add_task for follow-ups, and upsert_concept for lasting knowledge, after searching, so that you extend a concept instead of duplicating it.",
];

/// Server instructions for a profile (02 §2).
pub fn instructions(p: Profile) -> String {
    let mut s = INSTRUCTIONS.join("\n\n");
    match p {
        Profile::Full => {}
        Profile::ReadOnly => s.push_str("\n\nThis session is read-only (ENIGMA_READONLY): the tools that write, index, organize, undo or consolidate are not offered."),
    }
    s
}

const CHEAT_SHEET: [&str; 15] = [
    "search_brain(query, tags?, folder?, limit?) \u{2014} find notes by words and by meaning",
    "query_notes(project?, tags?, folder?) \u{2014} select notes by frontmatter",
    "read_note(path) / write_note(path, content) \u{2014} writes stay inside Claude/ unless confirmed",
    "log_session(title, content, project, tags) \u{2014} file a work log when work ends",
    "capture(content) \u{2014} one line into today's Inbox note",
    "upsert_concept(name, content) \u{2014} lasting topic notes; search before calling",
    "add_task(content, due?) / list_tasks(include_done?, folder?) \u{2014} checkbox tasks",
    "get_backlinks(path) / list_recent(n) \u{2014} who links here; what changed lately",
    "index_brain(force?) \u{2014} add new and changed notes to the graph",
    "find_entity(query, type?) / related_notes(path, hops?) / graph_stats() \u{2014} read the graph",
    "context_bundle(query, hops?, limit?) \u{2014} search results with their entities and linked notes",
    "recall_bundle(mode, project?, since?) \u{2014} facts for a digest or a project resume",
    "consolidate_graph(apply?) \u{2014} merge entities that are the same thing; preview by default",
    "organize_vault(apply?) / undo_move(path) \u{2014} filing suggestions; preview by default",
    "librarian_status() \u{2014} the latest caretaker run and its health checks",
];

fn ty_schema(ty: Ty) -> Value {
    match ty {
        Ty::Str => json!({"type": "string"}),
        Ty::Int => json!({"type": "integer"}),
        Ty::Bool => json!({"type": "boolean"}),
        Ty::StrArray => json!({"type": "array", "items": {"type": "string"}}),
        Ty::Object => json!({"type": "object", "additionalProperties": true}),
    }
}

fn to_tool(d: &ToolDef) -> Tool {
    let mut props = Map::new();
    let mut required = vec![];
    for p in &d.params {
        let schema = match &p.default {
            None => {
                required.push(json!(p.name));
                ty_schema(p.ty)
            }
            Some(Value::Null) => json!({"anyOf": [ty_schema(p.ty), {"type": "null"}], "default": null}),
            Some(v) => {
                let mut s = ty_schema(p.ty);
                s["default"] = v.clone();
                s
            }
        };
        props.insert(p.name.into(), schema);
    }
    let mut input = Map::new();
    input.insert("type".into(), json!("object"));
    input.insert("properties".into(), Value::Object(props));
    input.insert("required".into(), Value::Array(required));
    let mut tool = Tool::new(d.name, d.desc, input);
    let result = match d.out {
        Out::Text => Some(json!({"type": "string"})),
        Out::TextList => Some(json!({"type": "array", "items": {"type": "string"}})),
        Out::ObjList => Some(json!({"type": "array", "items": {"type": "object", "additionalProperties": true}})),
        Out::Obj => None,
    };
    if let Some(r) = result
        && let Value::Object(o) = json!({"type": "object", "properties": {"result": r}, "required": ["result"]})
    {
        tool.output_schema = Some(Arc::new(o));
    }
    tool
}

// --------------------------------------------------------------- arguments

fn coerce(ty: Ty, v: &Value) -> std::result::Result<Value, String> {
    let bad = || format!("expected {}, got {v}", match ty {
        Ty::Str => "a string",
        Ty::Int => "an integer",
        Ty::Bool => "a boolean",
        Ty::StrArray => "an array of strings",
        Ty::Object => "an object",
    });
    match (ty, v) {
        (Ty::Str, Value::String(_)) => Ok(v.clone()),
        (Ty::Int, Value::Number(n)) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Ok(json!(i)),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9.2e18 => Ok(json!(f as i64)),
            _ => Err(bad()),
        },
        (Ty::Int, Value::String(s)) => s.trim().parse::<i64>().map(|i| json!(i)).map_err(|_| bad()),
        (Ty::Bool, Value::Bool(_)) => Ok(v.clone()),
        (Ty::Bool, Value::Number(n)) if n.as_i64() == Some(0) || n.as_i64() == Some(1) => Ok(json!(n.as_i64() == Some(1))),
        (Ty::Bool, Value::String(s)) => match s.to_lowercase().as_str() {
            "true" | "yes" | "on" | "t" | "y" | "1" => Ok(json!(true)),
            "false" | "no" | "off" | "f" | "n" | "0" => Ok(json!(false)),
            _ => Err(bad()),
        },
        (Ty::StrArray, Value::Array(a)) if a.iter().all(Value::is_string) => Ok(v.clone()),
        (Ty::Object, Value::Object(_)) => Ok(v.clone()),
        (Ty::StrArray | Ty::Object, Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(parsed) if !parsed.is_string() => coerce(ty, &parsed).map_err(|_| bad()),
            _ => Err(bad()),
        },
        _ => Err(bad()),
    }
}

struct Args(HashMap<&'static str, Value>);

impl Args {
    fn parse(d: &ToolDef, raw: &Map<String, Value>) -> std::result::Result<Args, String> {
        let mut out = HashMap::new();
        for p in &d.params {
            let v = match (raw.get(p.name), &p.default) {
                (None, None) => return Err(format!("missing required argument '{}'", p.name)),
                (None, Some(dv)) | (Some(Value::Null), Some(dv @ Value::Null)) => dv.clone(),
                (Some(v), _) => coerce(p.ty, v).map_err(|e| format!("'{}': {e}", p.name))?,
            };
            out.insert(p.name, v);
        }
        Ok(Args(out))
    }
    fn s(&self, k: &str) -> &str {
        self.0.get(k).and_then(Value::as_str).unwrap_or_default()
    }
    fn os(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(Value::as_str)
    }
    fn i(&self, k: &str) -> i64 {
        self.0.get(k).and_then(Value::as_i64).unwrap_or_default()
    }
    fn b(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or_default()
    }
    fn list(&self, k: &str) -> Option<Vec<String>> {
        self.0.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
    }
}

enum Ret {
    Str(String),
    List(Vec<Value>),
    Obj(Value),
}

fn refuse_apply(what: &str, cli: &str) -> Result<()> {
    if config::truthy("ENIGMA_NO_TOOL_APPLY") {
        return Err(Error::vault(format!(
            "{what} is disabled for MCP calls on this server (ENIGMA_NO_TOOL_APPLY). Ask the human to run \u{60}{cli}\u{60} after reviewing the dry-run."
        )));
    }
    Ok(())
}

fn not_implemented(name: &str) -> Result<Ret> {
    Err(Error::new(Kind::Internal, format!("{name} is not implemented in this build")))
}

#[derive(Clone)]
pub struct Server {
    profile: Profile,
    emb: Arc<dyn Embedder>,
}

impl Server {
    pub fn new(profile: Profile, emb: Arc<dyn Embedder>) -> Self {
        Server { profile, emb }
    }

    fn defs(&self) -> Vec<ToolDef> {
        tool_defs()
            .into_iter()
            .filter(|d| match self.profile {
                Profile::Full => true,
                Profile::ReadOnly => d.read_only,
            })
            .collect()
    }

    pub fn tools(&self) -> Vec<Tool> {
        self.defs().iter().map(to_tool).collect()
    }

    fn run(&self, name: &str, a: &Args) -> Result<Ret> {
        let v = Vault::from_env()?;
        let state = || config::state_dir(Some(&v.root));
        let now = text::now();
        Ok(match name {
            "search_brain" => {
                let tags = a.list("tags");
                Ret::List(search::hybrid(&v, &state()?, self.emb.as_ref(), a.s("query"), tags.as_deref(), a.os("folder"), a.i("limit"))?)
            }
            "read_note" => Ret::Str(v.read(a.s("path"))?),
            "log_session" => {
                let tags = a.list("tags").unwrap_or_default();
                Ret::Str(notes::log_session(&v, a.s("title"), a.s("content"), a.s("project"), &tags, &now, &config::agent_name())?)
            }
            "capture" => Ret::Str(notes::capture(&v, a.s("content"), &now)?),
            "upsert_concept" => Ret::Str(notes::upsert_concept(&v, a.s("name"), a.s("content"), &now, &config::agent_name())?),
            "write_note" => {
                v.write(a.s("path"), a.s("content"), a.b("overwrite"), a.b("confirm_outside_claude"))?;
                Ret::Str(a.s("path").to_string())
            }
            "add_task" => Ret::Str(notes::add_task(&v, a.s("content"), a.os("due"))?),
            "list_tasks" => Ret::List(notes::list_tasks(&v, a.b("include_done"), a.os("folder"))?),
            "query_notes" => {
                let tags = a.list("tags");
                Ret::List(search::query_notes(&v, a.os("project"), tags.as_deref(), a.os("folder"), a.i("limit"))?)
            }
            "get_backlinks" => Ret::List(search::backlinks(&v, a.s("path"))?.into_iter().map(Value::String).collect()),
            "list_recent" => Ret::List(search::list_recent(&v, a.i("n"))?),
            "onboard" => Ret::Obj(self.onboard(&v)?),
            "graph_stats" => Ret::Obj(Graph::load(&graph::require_db(&state()?)?)?.stats()),
            "find_entity" => Ret::List(Graph::load(&graph::require_db(&state()?)?)?.find(a.s("query"), a.os("type"))),
            "related_notes" => Ret::List(Graph::load(&graph::require_db(&state()?)?)?.related(a.s("path"), a.i("hops"), a.i("limit"))?),
            "context_bundle" => Ret::Obj(self.context_bundle(&v, &state()?, a)?),
            "index_brain" => {
                if a.b("force") {
                    refuse_apply("index_brain(force=true)", "enigma-mcp index <vault> --force")?;
                }
                let st = state()?;
                let _lock = lock::acquire(&st)?;
                let ex = Cli::extraction(None)?;
                let o = indexer::Options { force: a.b("force"), ..Default::default() };
                Ret::Obj(serde_json::to_value(indexer::run(&v, &st, &ex, Some(self.emb.as_ref()), &o, &now)?).unwrap_or_default())
            }
            "consolidate_graph" => {
                let ex = Cli::consolidation(None)?;
                let st = state()?;
                let apply = a.b("apply");
                let _lock = if apply {
                    refuse_apply("consolidate_graph(apply=true)", "enigma-mcp consolidate <vault> --apply")?;
                    Some(lock::acquire(&st)?)
                } else {
                    None
                };
                Ret::Obj(consolidate::run(&v, &st, &ex, apply, self.emb.as_ref(), &now)?)
            }
            "sheet_upsert" => {
                let fields = a.0.get("fields").and_then(Value::as_object).map(sheets::fields_from).unwrap_or_default();
                let agent = a.os("agent").map_or_else(config::agent_name, String::from);
                Ret::Obj(sheets::upsert(&v, a.s("sheet"), &fields, a.os("body"), &agent, &now)?)
            }
            "sheet_query" => {
                let (filters, sort) = (a.0.get("filters").and_then(Value::as_object), a.0.get("sort").and_then(Value::as_object));
                Ret::List(sheets::query(&v, a.s("sheet"), filters, sort, a.i("limit"))?)
            }
            "sheet_schema" => Ret::Obj(sheets::schema_info(&v, a.os("sheet"))?),
            "organize_vault" => {
                let st = state()?;
                let apply = a.b("apply");
                let _lock = if apply {
                    refuse_apply("organize_vault(apply=true)", "enigma-mcp organize <vault>")?;
                    Some(lock::acquire(&st)?)
                } else {
                    None
                };
                // an agent's apply never moves notes unless the owner set a cap; dry runs stay uncapped
                let cap = apply.then(|| config::var("ENIGMA_MAX_MOVES").and_then(|m| m.trim().parse::<i64>().ok()).unwrap_or(0).max(0));
                Ret::Obj(organizer::run(&v, &st, self.emb.as_ref(), apply, cap, &now)?)
            }
            "undo_move" => Ret::Obj(mover::undo_move(&v, &state()?, a.s("path"), &now)?),
            "librarian_status" => Ret::Obj(librarian::status(&state()?)?),
            "recall_bundle" => match a.s("mode") {
                "digest" => Ret::Obj(recall::digest(&v, &state()?, a.os("since"), &now)?),
                "resume" => match a.os("project").filter(|p| !p.is_empty()) {
                    Some(p) => Ret::Obj(recall::resume(&v, &state()?, p, 10)),
                    None => return Err(Error::vault("mode='resume' requires project")),
                },
                other => return Err(Error::vault(format!("mode must be 'digest' or 'resume', got {}", text::quote_str(other)))),
            },
            _ => return not_implemented(name),
        })
    }

    /// context_bundle (02 §5.2).
    fn context_bundle(&self, v: &Vault, st: &std::path::Path, a: &Args) -> Result<Value> {
        let hits = search::hybrid(v, st, self.emb.as_ref(), a.s("query"), None, None, a.i("limit"))?;
        let db = st.join("graph.db");
        if !db.exists() {
            return Ok(json!({"hits": hits, "entities": [], "related_notes": []}));
        }
        let g = Graph::load(&db)?;
        let mut paths = std::collections::BTreeSet::new();
        let mut related: Vec<Value> = Vec::new();
        for h in &hits {
            let path = h["path"].as_str().unwrap_or_default();
            paths.extend(g.note_entity_paths(path));
            for row in g.related(path, a.i("hops"), 20)? {
                if !related.iter().any(|r| r["path"] == row["path"]) {
                    related.push(row);
                }
            }
        }
        related.truncate(20);
        let paths: Vec<String> = paths.into_iter().collect();
        Ok(json!({"hits": hits, "entities": g.entities_at(&paths), "related_notes": related}))
    }

    fn onboard(&self, v: &Vault) -> Result<Value> {
        let read_or = |rel: &str, fallback: &str| match v.read(rel) {
            Ok(t) => Ok(t),
            Err(e) if e.is(Kind::VaultError) => Ok(fallback.to_string()),
            Err(e) => Err(e),
        };
        let db = config::state_dir(Some(&v.root))?.join("graph.db");
        let graph = if db.exists() { Graph::load(&db)?.stats() } else { json!("not built yet \u{2014} call index_brain") };
        Ok(json!({
            "instructions": instructions(self.profile),
            "constitution": read_or("Claude/README.md", "(constitution not installed \u{2014} run enigma-mcp install-conventions VAULT)")?,
            "vault_guide": read_or("CLAUDE.md", "(vault guide not installed \u{2014} run enigma-mcp install-conventions VAULT)")?,
            "tools": CHEAT_SHEET,
            "graph": graph,
        }))
    }

    /// One tools/call, rendered per 02 §3.
    pub fn call(&self, name: &str, args: &Map<String, Value>) -> CallToolResult {
        let defs = self.defs();
        let Some(d) = defs.iter().find(|d| d.name == name) else {
            return CallToolResult::error(vec![ContentBlock::text(format!("Unknown tool: {name}"))]);
        };
        let fail = |msg: String| CallToolResult::error(vec![ContentBlock::text(format!("Error executing tool {name}: {msg}"))]);
        let a = match Args::parse(d, args) {
            Ok(a) => a,
            Err(detail) => return fail(format!("invalid arguments: {detail}")),
        };
        let ret = match self.run(name, &a) {
            Ok(r) => r,
            Err(e) => return fail(e.msg),
        };
        let (content, value) = match ret {
            Ret::Str(s) => (vec![ContentBlock::text(s.clone())], Value::String(s)),
            Ret::List(items) => {
                let blocks = items
                    .iter()
                    .map(|i| match i {
                        Value::String(s) => ContentBlock::text(s.clone()),
                        other => ContentBlock::text(mcp_text(other)),
                    })
                    .collect();
                (blocks, Value::Array(items))
            }
            Ret::Obj(o) => return CallToolResult::success(vec![ContentBlock::text(mcp_text(&o))]),
        };
        let mut r = CallToolResult::success(content);
        r.structured_content = Some(json!({"result": structured(&value)}));
        r
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("enigma", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(instructions(self.profile));
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let me = self.clone();
        let args = request.arguments.unwrap_or_default();
        let res = tokio::task::spawn_blocking(move || me.call(&request.name, &args))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(res.into())
    }
}
