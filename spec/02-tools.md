# 02 — MCP tools

Enigma's MCP server exposes 25 tools. This file fixes, for each tool, its
parameters, types and defaults, return shape (with an example), errors, side
effects and profile membership. Algorithms live in the files referenced; this
file is the surface contract. Tools 5.13 onward are in
[02-tools-b.md](02-tools-b.md).

Acceptance: `server.yaml` and the cases referenced per tool.

## 1. Registration order

`tools/list` returns the tools of the active profile (§4) in this order:

1. `search_brain` 2. `context_bundle` 3. `organize_vault`
4. `librarian_status` 5. `recall_bundle` 6. `sheet_upsert` 7. `sheet_query`
8. `sheet_schema` 9. `undo_move` 10. `read_note` 11. `log_session`
12. `capture` 13. `upsert_concept` 14. `write_note` 15. `add_task`
16. `list_tasks` 17. `query_notes` 18. `get_backlinks` 19. `list_recent`
20. `onboard` 21. `index_brain` 22. `find_entity` 23. `related_notes`
24. `graph_stats` 25. `consolidate_graph`

Each tool's `inputSchema` is a JSON Schema object whose `properties` are
exactly the parameters listed for it, with types as listed (nullable
parameters allow `null`), and whose `required` array lists exactly the
parameters without a default. Each tool's `description` is the text given in
its section, verbatim.

## 2. Server instructions

The `instructions` string sent in the MCP `initialize` result (and returned by
`onboard`) is the following four paragraphs joined by `\n\n`, each paragraph a
single line, with no trailing newline:

```text
Enigma is the long-term memory this vault's owner shares with their AI agents: one Obsidian vault that all of them read and write.

First step in any new session: call `onboard`. Before your first write, read Claude/README.md; it is the constitution and its rules bind every agent.

Where things live: agents write only inside Claude/. Sessions/ holds work logs, Concepts/ holds lasting topic notes, Inbox/ holds quick captures, Tasks.md holds checkbox tasks, Decisions.md is a decision log that is only ever appended to, and Graph/ holds the entity graph. The rest of the vault is the owner's. Read any of it, but write outside Claude/ only on an explicit request; without confirm_outside_claude such a write is rejected.

Working habits: look before you conclude something is unknown (search_brain, query_notes, related_notes, find_entity). When a meaningful piece of work ends, call log_session. Use capture for passing thoughts, add_task for follow-ups, and upsert_concept for lasting knowledge, after searching, so that you extend a concept instead of duplicating it.
```

(The backticks around `onboard` are literal characters; dashes are U+2014.)
The active profile appends one more paragraph, preceded by `\n\n`:

| Profile | Extra paragraph |
|---|---|
| full | none |
| read-only | `This session is read-only (ENIGMA_READONLY): the tools that write, index, organize, undo or consolidate are not offered.` |

## 3. Results and errors on the MCP surface

### 3.1 Successful results

A tool returns a string, an object or a list. The `tools/call` result has
`isError: false` and `content` built as follows:

| Return value | `content` |
|---|---|
| string | one text block whose text is the string |
| object | one text block whose text is the object in MCP JSON (16 §3.1) |
| list | one text block per element, in order: a string element as is, an object element in MCP JSON; an empty list gives `content: []` |

So `search_brain` with two hits yields two text blocks, each a
pretty-printed `{"path", "excerpt"}` object, and `get_backlinks` yields one
text block per path string.

Structured output is normative:

- **String- and list-returning tools** (`search_brain`, `sheet_query`,
  `read_note`, `log_session`, `capture`, `upsert_concept`, `write_note`,
  `add_task`, `list_tasks`, `query_notes`, `get_backlinks`, `list_recent`,
  `find_entity`, `related_notes`): the `tools/list` entry carries an
  `outputSchema` that is a JSON Schema object with exactly one property
  `result`, listed in `required` (type `string`, an array of strings, or an
  array of objects, matching the return value; `title` keywords are
  implementation-defined). Every successful `tools/call` result carries
  `structuredContent` = `{"result": <return value>}` in addition to the text
  content.
- **Object-returning tools** (`context_bundle`, `organize_vault`,
  `librarian_status`, `recall_bundle`, `sheet_upsert`, `sheet_schema`,
  `undo_move`, `onboard`, `index_brain`, `graph_stats`,
  `consolidate_graph`): no `outputSchema` and no `structuredContent`; the text
  block is the only result.
- Error results (§3.2) never carry `structuredContent`.

The text content is what tests compare; `structuredContent`
holds the same value as JSON data.

Non-finite floats (a frontmatter or sheet value `.nan`, `.inf`, `-.inf` that
reaches a result through `query_notes` or `sheet_query`) are written in text
content as the bare tokens `NaN`, `Infinity`, `-Infinity` (16 §3.1) and in
`structuredContent` as `null`.

In MCP JSON, YAML date values that reach a result unconverted (sheet rows,
10) are written as `"YYYY-MM-DD"` and date-times as `"YYYY-MM-DDTHH:MM:SS"`
(ISO 8601 with `T`, fraction and offset when present).

### 3.2 Errors

When a tool fails with any error kind (00 §5), the `tools/call` result is
`isError: true` with exactly one text block:

```text
Error executing tool {tool}: {message}
```

Example: `Error executing tool write_note: 'Projects/Injected.md' is outside Claude/. Pass confirm_outside_claude=true only when the user explicitly asked for this write.`

Other failures:

| Situation | Result |
|---|---|
| tool name not registered in the active profile | `isError: true`, text `Unknown tool: {name}` |
| arguments fail validation (missing required, wrong type) | `isError: true`, text `Error executing tool {tool}: invalid arguments: {detail}`, detail implementation-defined, one line |

### 3.3 Argument handling

- Unknown extra arguments are ignored.
- Integer parameters accept JSON integers, floats with a zero fraction, and
  strings holding an optionally signed decimal integer.
- Boolean parameters accept JSON booleans, the integers 0 and 1, and the
  strings (case-insensitive) `true`, `false`, `yes`, `no`, `on`, `off`, `t`,
  `f`, `y`, `n`, `1`, `0`.
- String parameters accept JSON strings only.
- Array and object parameters accept JSON arrays / objects, and also a JSON
  string whose content parses as an array / object of the right kind.
- Nullable parameters accept `null`, which means the same as omitting them.

## 4. Profiles, locks and refusals

Profiles are chosen at start-up (01 §6). "Lock" means the caretaker lock
(03 §10); "Refusable" means the call is refused when `ENIGMA_NO_TOOL_APPLY`
is truthy (01 §7).

| # | Tool | Full | Read-only | Writes | Lock | Refusable |
|---|---|---|---|---|---|---|
| 1 | search_brain | ✓ | ✓ | state dir (vector cache) | — | — |
| 2 | context_bundle | ✓ | ✓ | state dir (vector cache) | — | — |
| 3 | organize_vault | ✓ | — | vault + state when apply | apply | apply=true |
| 4 | librarian_status | ✓ | ✓ | — | — | — |
| 5 | recall_bundle | ✓ | ✓ | — | — | — |
| 6 | sheet_upsert | ✓ | — | vault | — | — |
| 7 | sheet_query | ✓ | ✓ | — | — | — |
| 8 | sheet_schema | ✓ | ✓ | — | — | — |
| 9 | undo_move | ✓ | — | vault + state | — | — |
| 10 | read_note | ✓ | ✓ | — | — | — |
| 11 | log_session | ✓ | — | vault | — | — |
| 12 | capture | ✓ | — | vault | — | — |
| 13 | upsert_concept | ✓ | — | vault | — | — |
| 14 | write_note | ✓ | — | vault | — | — |
| 15 | add_task | ✓ | — | vault | — | — |
| 16 | list_tasks | ✓ | ✓ | — | — | — |
| 17 | query_notes | ✓ | ✓ | — | — | — |
| 18 | get_backlinks | ✓ | ✓ | — | — | — |
| 19 | list_recent | ✓ | ✓ | — | — | — |
| 20 | onboard | ✓ | ✓ | — | — | — |
| 21 | index_brain | ✓ | — | vault + state | always | force=true |
| 22 | find_entity | ✓ | ✓ | — | — | — |
| 23 | related_notes | ✓ | ✓ | — | — | — |
| 24 | graph_stats | ✓ | ✓ | — | — | — |
| 25 | consolidate_graph | ✓ | — | vault + state when apply | apply | apply=true |

Totals: full 25, read-only 15.

Shared errors: every tool needs the vault, so every tool can fail with the
vault configuration errors of 01 §3 (`ENIGMA_VAULT_PATH is not set;
point it at the vault folder.`, `Vault root does not exist: {root}`). Tools
that compute the state dir can fail with IoError if it cannot be created.
Tools that parse frontmatter can fail with edge case Q5. These are not repeated
below. "Graph tools" (find_entity, related_notes, graph_stats) need
`<state dir>/graph.db` and otherwise fail with VaultError
`Graph cache not built yet — run index_brain first.`

## 5. Tool reference

Notation: parameter tables list name, JSON type, default ("required" when
none). `string?` means string or null.

### 5.1 search_brain

Description: `Find notes by exact words and by meaning at once: a keyword ranking and an embedding ranking are merged into one list. Narrow it with frontmatter tags or a folder. Each result is a note path with a short excerpt, most relevant first.`

| Parameter | Type | Default |
|---|---|---|
| query | string | required |
| tags | array of string? | null |
| folder | string? | null |
| limit | integer | 20 |

Behaviour: hybrid(query, tags, folder, limit) (04 §7). Returns a list of
`{"path": string, "excerpt": string}`.

Example (mini vault): `search_brain(query="ingestion pipeline")` → first
content block:

```json
{
  "path": "Projects/Lighthouse Grid.md",
  "excerpt": "Grid outage ingestion pipeline with a queue-free architecture."
}
```

Errors: folder escapes (`Path escapes the vault: {folder}`); embedder
failures (IoError / Internal). Side effects: may write
`fallback_embeddings.json` (04 §5). Limit: slice semantics (Q7).

### 5.2 context_bundle

Description: `Search plus graph context in a single call. Returns the best-matching notes for the query, the entities those notes mention, and other notes reachable from those entities in up to hops steps.`

| Parameter | Type | Default |
|---|---|---|
| query | string | required |
| hops | integer | 2 |
| limit | integer | 10 |

Behaviour:

1. hits = hybrid(query, no tags, no folder, limit) (04 §7).
2. If `<state dir>/graph.db` does not exist, return
   `{"hits": hits, "entities": [], "related_notes": []}`.
3. Otherwise, for each hit in rank order: add note_entity_paths(hit.path) to
   a set of entity paths; then for each row of related_notes(hit.path, hops,
   limit 20) (05) append it to the merged list unless a row with the same
   `path` was already appended.
4. Truncate the merged list to its first 20 rows.
5. entities = entities_at(sorted entity paths) (05): the find_entity-shaped
   object for each path that exists in the cache, in sorted order.
6. Return `{"hits": [{"path", "excerpt"}…], "entities": [entity…],
   "related_notes": [{"path", "via"}…]}`.

Entity object (05): `{"name", "type", "path", "summary", "aliases",
"relations": [{"rel", "to"}…], "mention_count"}`.

Example (after indexing, SV-14):

```json
{
  "hits": [
    {
      "path": "Projects/Lighthouse Grid.md",
      "excerpt": "Grid outage ingestion pipeline with a queue-free architecture."
    }
  ],
  "entities": [
    {
      "name": "Orbital Freight",
      "type": "organization",
      "path": "Claude/Graph/Organizations/Orbital Freight.md",
      "summary": "A company.",
      "aliases": [],
      "relations": [],
      "mention_count": 1
    }
  ],
  "related_notes": []
}
```

Errors: as search_brain. Limit: slice semantics for hits (Q7).

### 5.3 organize_vault

Description: `Suggest where loose notes belong by comparing each one with its most similar filed notes. The default is a preview that changes nothing and lists moved, proposals and skipped. With apply=true the suggestion list is refreshed and no more than ENIGMA_MAX_MOVES notes are moved (0 unless the owner raises it); every move is journaled and undo_move reverses it.`

| Parameter | Type | Default |
|---|---|---|
| apply | boolean | false |

Behaviour: apply=false runs the organizer sweep as a dry run (09) with
vectors computed in memory only (no cache write, no lock). apply=true:
refused-apply check (01 §7), then take the caretaker lock, then run the
sweep with apply and `max_moves` = `ENIGMA_MAX_MOVES` (01 §2; default 0, so
an agent's call moves nothing unless the owner set a cap). Returns:

| Key | Value |
|---|---|
| `moved` | list of `{"from", "to_folder", "share", "neighbors"}` |
| `proposals` | list of `{"path", "suggested", "share", "neighbors", "reason"}` |
| `skipped` | list of `{"path", "reason"}` |
| `cache_rebuilt` | boolean |

`share` is rounded to 3 decimals; `neighbors` has at most 3 paths. A dry run
reports the moves it would make in `moved` and moves nothing.

Example (SV-15, dry run):

```json
{
  "moved": [
    {
      "from": "Loose.md",
      "to_folder": "02 - Orbit",
      "share": 1.0,
      "neighbors": [
        "02 - Orbit/Orbits.md"
      ]
    }
  ],
  "proposals": [],
  "skipped": [],
  "cache_rebuilt": false
}
```

Errors: refused apply (01 §7); LockBusy; organizer errors (09). Side effects
(apply): file moves, link rewrites, journal and `Claude/Organizer.md` entries,
cache rebuild, vector cache write.

### 5.4 librarian_status

Description: `Report from the most recent Librarian run: what each step did, the health checks, and how many proposals are waiting. Changes nothing; the Librarian itself is started on a schedule with enigma-mcp librarian.`

No parameters. Returns one of:

| Situation | Result |
|---|---|
| no `librarian_state.json` in the state dir | `{"status": "no sweep yet"}` |
| the file exists but cannot be read or parsed as JSON | `{"status": "state file unreadable", "error": message, "path": absolute path of the file}` (message implementation-defined, one line) |
| otherwise | the state object as stored (16 §7), after the load defaults of 08 |

Example after a sweep: an object whose `last_sweep` is
`"2026-07-09 12:00:00"` and which also has `steps`, `health`, `errors` and
`consolidation`.

### 5.5 recall_bundle

Description: `Collect, without calling a model, the facts the recall skills build on. mode='digest' lists what changed after a date (7 days ago when omitted). mode='resume' gathers session logs, decisions, open tasks and graph entities whose project contains the given text, ignoring case. A section that fails does not stop the others.`

| Parameter | Type | Default |
|---|---|---|
| mode | string | required |
| project | string? | null |
| since | string? | null |

Behaviour:

- `mode == "digest"`: the digest bundle for since (11); project is ignored.
- `mode == "resume"`: if project is null or empty, VaultError
  `mode='resume' requires project`; otherwise the resume bundle for project
  (11); since is ignored.
- any other mode: VaultError
  `mode must be 'digest' or 'resume', got {quoted(mode)}`, e.g.
  `mode must be 'digest' or 'resume', got 'weekly'`.

Digest keys: `mode`, `generated`, `since`, `librarian`, `recent_notes`,
`inbox_captures`, `tasks`, `proposals`, `new_entities`. Resume keys: `mode`,
`project`, `sessions`, `decisions`, `tasks`, `entities`. Each section object
carries `"status": "ok"` or `{"status": "error", "error": "<Kind>: <message>"}`
(11). A malformed since fails the whole call with VaultError
`since must be YYYY-MM-DD: {quoted(since)}`.

### 5.6 sheet_upsert

Description: `Add a row to a sheet, or update the row that is already there. A sheet is a folder the owner marked with a _schema.md file. Fields are checked against that schema, the existing row is located by its key and identity columns, and only the fields you pass are changed. agent is the name recorded on the row; it defaults to this server's ENIGMA_AGENT_NAME.`

| Parameter | Type | Default |
|---|---|---|
| sheet | string | required |
| fields | object | required |
| body | string? | null |
| agent | string? | null |

Behaviour: sheet upsert (10). agent null or empty means the agent name
(01 §5). Returns `{"result": "created" | "updated", "path": rel,
"changed": {column: {"from": old, "to": new}…}}`. On create, every passed
field appears in changed with `"from": null`.

Example (SV-20): `sheet_upsert(sheet="things", fields={"name": "First",
"status": "New"}, agent="cowork")` →

```json
{
  "result": "created",
  "path": "Things/First.md",
  "changed": {
    "name": {
      "from": null,
      "to": "First"
    },
    "status": {
      "from": null,
      "to": "New"
    }
  }
}
```

(the path depends on the sheet's folder and filename template). Errors:
SheetError (unknown sheet, validation, ambiguous match, body on update — 10);
NoteExists only in races the suffix logic cannot absorb. Side effects: writes
a row note (rows live outside `Claude/`; sheets pass the confirmation).

### 5.7 sheet_query

Description: `Filter and sort the rows of a sheet. filters maps a column to {op: value}; the ops are eq, ne, lt, lte, gt, gte, contains, missing, in and nin, and every filter must hold. sort is {by, dir}. limit caps the number of rows.`

| Parameter | Type | Default |
|---|---|---|
| sheet | string | required |
| filters | object? | null |
| sort | object? | null |
| limit | integer | 50 |

Behaviour: sheet query (10). Returns a list of row objects
`{"path": rel, <frontmatter keys…>}`, one content block per row. Frontmatter
values are emitted as parsed (dates per §3.1). Limit: slice semantics (Q7).
Errors: SheetError (unknown sheet, unknown operator, ordering on a column that
is not date/number).

### 5.8 sheet_schema

Description: `Describe sheets. Called with no sheet, it lists every sheet with its folder and number of rows. Called with a sheet, it returns that sheet's columns, types, key, identity columns and filing instructions.`

| Parameter | Type | Default |
|---|---|---|
| sheet | string? | null |

Returns (10): without sheet `{name: {"folder", "rows"}…}` plus
`"invalid": [{"folder", "error"}…]` when some schema fails to load; with
sheet `{"sheet", "folder", "filename", "key", "identity", "columns":
{name: {"type", "required", "values", "max_length"}…}, "instructions"}`.
Errors: SheetError (unknown sheet, duplicate sheet name).

### 5.9 undo_move

Description: `Put a note the organizer moved back where it was. Pass the note's present path inside the vault; the latest move of that note is reversed, including the links that were rewritten and the note's indexing record.`

| Parameter | Type | Default |
|---|---|---|
| path | string | required |

Returns `{"restored": original rel, "was": rel it was moved to}`. Errors
(VaultError): `No undoable move found for: {path}`,
`Cannot undo: file no longer at {to}`,
`Cannot undo: original location occupied: {from}`. Not locked. Side effects:
moves the file back, reverses link rewrites, renames the manifest entry,
marks the journal entry undone, appends to `Claude/Organizer.md` (09).

### 5.10 read_note

Description: `Return the text of one note. path is relative to the vault root, for example 'Claude/Index.md'.`

| Parameter | Type | Default |
|---|---|---|
| path | string | required |

Returns the note text (strict read, 03 §4) as a string. Errors: escape,
`'{path}' is a directory, not a note.`, `Note not found: {path}`,
DecodeError.

### 5.11 log_session

Description: `Save a session log as a new note in Claude/Sessions/ and list it in Claude/Index.md. Call it when a meaningful piece of work ends, and say what happened, what was learned and what was decided.`

| Parameter | Type | Default |
|---|---|---|
| title | string | required |
| content | string | required |
| project | string | required |
| tags | array of string? | null |

Behaviour: 03 §7.3 (null tags = empty list). Returns the new note's rel path,
e.g. `Claude/Sessions/2026-07-05 Relay setup.md`.

### 5.12 capture

Description: `Add one timestamped line to today's note in Claude/Inbox/, for thoughts worth keeping that do not need a note of their own yet.`

| Parameter | Type | Default |
|---|---|---|
| content | string | required |

Behaviour: 03 §7.4. Returns e.g. `Claude/Inbox/2026-07-05.md`.
