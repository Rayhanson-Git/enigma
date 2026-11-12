# 02b — MCP tools (continued)

Continuation of [02-tools.md](02-tools.md): tools 5.13 to 5.25. Sections 1–4
of that file (registration, instructions, result and error encoding,
profiles) apply here unchanged.

### 5.13 upsert_concept

Description: `Keep one lasting note per concept in Claude/Concepts/. A new name creates the note; a name that already exists (letter case ignored) gets a dated update appended, so nothing is duplicated or replaced. Search before calling.`

| Parameter | Type | Default |
|---|---|---|
| name | string | required |
| content | string | required |

Behaviour: 03 §7.5. Returns the rel path of the created or extended note,
e.g. `Claude/Concepts/DynDNS.md`; `upsert_concept("postgres", …)` on the mini
vault returns `Claude/Concepts/Postgres.md` (existing note extended).

### 5.14 write_note

Description: `Write a note at any path. Paths outside Claude/ are rejected unless confirm_outside_claude=true, and that flag is only for writes the user asked for in so many words. An existing note is kept unless overwrite=true.`

| Parameter | Type | Default |
|---|---|---|
| path | string | required |
| content | string | required |
| confirm_outside_claude | boolean | false |
| overwrite | boolean | false |

Behaviour: the vault write of 03 §5.2. Returns `path` exactly as the caller
passed it (not normalised). Errors, in check order: escape; outside Claude/
(message in 03 §3); directory; NoteExists
`'{path}' already exists. Pass overwrite=true to replace it.`; IoError after
the retry budget.

Example: `write_note(path="Projects/Injected.md", content="x")` → error text
`Error executing tool write_note: 'Projects/Injected.md' is outside Claude/. Pass confirm_outside_claude=true only when the user explicitly asked for this write.`;
with `confirm_outside_claude=true` it returns `Projects/Injected.md`.

### 5.15 add_task

Description: `Append an open checkbox task to Claude/Tasks.md, in the format the Obsidian Tasks plugin reads. due is optional and must be YYYY-MM-DD.`

| Parameter | Type | Default |
|---|---|---|
| content | string | required |
| due | string? | null |

Behaviour: 03 §8. Returns `Claude/Tasks.md`. Errors: InvalidArgument for a bad
due, e.g. `Error executing tool add_task: 'tomorrow' is not a date in the form YYYY-MM-DD`.

### 5.16 list_tasks

Description: `Collect checkbox tasks from every note in the vault, or from one folder. Finished tasks are left out unless include_done=true.`

| Parameter | Type | Default |
|---|---|---|
| include_done | boolean | false |
| folder | string? | null |

Behaviour: 03 §8. Returns a list of `{"path": rel, "text": string,
"done": boolean}`, one content block per task. Example block:

```json
{
  "path": "Claude/Tasks.md",
  "text": "deploy VM 📅 2026-07-10",
  "done": false
}
```

### 5.17 query_notes

Description: `Select notes by their frontmatter: project, tags, folder, or any mix of them. Each result is a note path with its frontmatter. With no filter it returns the notes that have any frontmatter at all.`

| Parameter | Type | Default |
|---|---|---|
| project | string? | null |
| tags | array of string? | null |
| folder | string? | null |
| limit | integer | 50 |

Behaviour: for each note of walk(folder) (04 §2), read leniently and let
meta = parse(text) (03 §6):

1. If project is not null (an empty string is a real filter): skip the note
   unless casefold(display(meta["project"])) equals casefold(project), where
   a missing `project` key counts as the empty string and a null value as
   `None` (edge case Q6).
2. If tags is a non-empty list: skip the note unless the casefolded tags are
   a subset of the casefolded str_list(meta["tags"]).
3. If project is null, tags is null or empty, and meta is empty: skip (a
   plain listing shows only notes with frontmatter).
4. Append `{"path": rel, "frontmatter": {key: json_safe(value)…}}` (03 §6),
   keys in frontmatter order.
5. After appending, stop if the number of results is ≥ limit (so limit ≤ 0
   still returns one row, edge case Q7).

Example (GQ-06): frontmatter `created: 2026-07-05`, `tags: [a, b]`,
`count: 3`, `done: true` gives

```json
{
  "path": "Notes/Typed.md",
  "frontmatter": {
    "created": "2026-07-05",
    "tags": [
      "a",
      "b"
    ],
    "count": 3,
    "done": true
  }
}
```

### 5.18 get_backlinks

Description: `Which notes link to this one with a [[wikilink]]? Useful for seeing what already depends on a topic before you add to it.`

| Parameter | Type | Default |
|---|---|---|
| path | string | required |

Behaviour:

1. target = path resolved (03 §2; may fail with the escape error). The target
   need not exist.
2. target stem = the target's final component with its last extension
   removed: everything before the last `.`, unless that dot is the first
   character or there is none (then the whole component). So `Postgres.md`
   → `Postgres` and `notes.v2.md` → `notes.v2`.
3. For each note of walk(null) in walk order whose path is not the target
   path, read leniently and find every occurrence of `[[` followed by one or
   more characters none of which is `]`, `|` or `#` (take the longest such
   run). For each run: strip it, split it on `/`, take the last piece,
   casefold it. If any equals the casefolded target stem, include the note's
   rel once.

Returns a list of rel strings, one content block each. Examples (GQ-04/05):
backlinks of `Claude/Concepts/Postgres.md` include notes containing
`[[postgres]]` and `[[Claude/Concepts/Postgres|the db]]`; a note never lists
itself; `[[Postgres.md]]` does not count (edge case Q9).

### 5.19 list_recent

Description: `The n notes changed most recently, latest first. A quick way to see what has been happening in the vault.`

| Parameter | Type | Default |
|---|---|---|
| n | integer | 10 |

Behaviour: for every note of walk(null) take its modification time (seconds,
64-bit float, following symlinks); sort by (mtime, rel) descending, i.e.
newest first and, for equal times, rel in reverse code-point order; take
`[:n]` with slice semantics (edge case Q7). Returns a list of
`{"path": rel, "modified": minute stamp of the mtime in local time}`, e.g.
`{"path": "Daily.md", "modified": "2026-07-05 14:30"}`.

### 5.20 onboard

Description: `Start every new session here. Returns the vault's constitution, its guide to where things go, a short list of the tools, and the state of the graph.`

No parameters. Returns:

| Key | Value |
|---|---|
| `instructions` | the server instructions of 02 §2 for the active profile |
| `constitution` | text of `Claude/README.md` (strict read); on any VaultError `(constitution not installed — run enigma-mcp install-conventions VAULT)` |
| `vault_guide` | text of `CLAUDE.md` at the vault root; on any VaultError `(vault guide not installed — run enigma-mcp install-conventions VAULT)` |
| `tools` | the 15 strings below, in order |
| `graph` | graph_stats object (5.24) if `graph.db` exists, else the string `not built yet — call index_brain` |

Dashes in these strings are U+2014. The cheat-sheet strings, exactly:

```text
search_brain(query, tags?, folder?, limit?) — find notes by words and by meaning
query_notes(project?, tags?, folder?) — select notes by frontmatter
read_note(path) / write_note(path, content) — writes stay inside Claude/ unless confirmed
log_session(title, content, project, tags) — file a work log when work ends
capture(content) — one line into today's Inbox note
upsert_concept(name, content) — lasting topic notes; search before calling
add_task(content, due?) / list_tasks(include_done?, folder?) — checkbox tasks
get_backlinks(path) / list_recent(n) — who links here; what changed lately
index_brain(force?) — add new and changed notes to the graph
find_entity(query, type?) / related_notes(path, hops?) / graph_stats() — read the graph
context_bundle(query, hops?, limit?) — search results with their entities and linked notes
recall_bundle(mode, project?, since?) — facts for a digest or a project resume
consolidate_graph(apply?) — merge entities that are the same thing; preview by default
organize_vault(apply?) / undo_move(path) — filing suggestions; preview by default
librarian_status() — the latest caretaker run and its health checks
```

A DecodeError while reading either file is not a VaultError and fails the
call. The cheat-sheet is the same in every profile.

### 5.21 index_brain

Description: `Read notes that are new or have changed and add their entities and relations to the graph, using the configured model CLI. The result includes a remaining count; repeat the call until it is 0. force=true processes every note again.`

| Parameter | Type | Default |
|---|---|---|
| force | boolean | false |

Behaviour: if force, the refused-apply check (01 §7). Then, always, take the
caretaker lock, build the extraction extractor (06; backend from
`ENIGMA_EXTRACTOR`; it sends the normative prompt of 06 §5.3) and run one indexer batch (06) with the default batch
size 25, the given force, embeddings precomputed. Returns the counts object
with keys in this order: `processed`, `entities_created`, `entities_merged`,
`mentions_added`, `relations_added`, `mentions_retracted`, `failed`,
`skipped`, `remaining` (all integers).

Example (SV-09, mini vault, extractor yields Orbital Freight for the
Lighthouse note):

```json
{
  "processed": 4,
  "entities_created": 1,
  "entities_merged": 0,
  "mentions_added": 1,
  "relations_added": 0,
  "mentions_retracted": 0,
  "failed": 0,
  "skipped": 0,
  "remaining": 0
}
```

Errors: refused apply; LockBusy; ExtractorError raised while building the
extractor (only `Unknown backend: {name}`); the run errors of 06 §4. Every
per-note failure, including a CLI missing from PATH (06 §5.4) or a note that
is not valid UTF-8, is counted in `failed`, not raised. Side effects: entity notes under
`Claude/Graph/`, `manifest.json`, `graph.db`, vector cache.

### 5.22 find_entity

Description: `Search the graph's entities by part of a name or alias, ignoring case. type optionally limits the result to person, organization, domain, topic, project or source.`

| Parameter | Type | Default |
|---|---|---|
| query | string | required |
| type | string? | null |

Behaviour: graph tool (needs `graph.db`). Every cached entity, in cache row
order, whose name or any alias contains casefold(query) after casefolding,
and, when type is non-null and non-empty, whose type equals it exactly (05).
Returns a list of entity objects (5.2), one content block each. Example
(SV-09): `find_entity("orbital")` first block has `"type": "organization"`.
Without a cache: `Error executing tool find_entity: Graph cache not built yet — run index_brain first.`

### 5.23 related_notes

Description: `Notes that share graph entities with the given note, directly or through up to hops steps. Strongest connections come first, at most limit results, and each one shows the chain of entities that links it.`

| Parameter | Type | Default |
|---|---|---|
| path | string | required |
| hops | integer | 2 |
| limit | integer | 20 |

Behaviour: graph tool; related-notes ranking of 05 (path with or without
`.md`; hops ≤ 1 uses only directly mentioned entities). Returns a list of
`{"path": rel with .md, "via": chain}` where chain is an entity name or
`{name} ({rel}) {name}…`. Errors: limit < 0 → InvalidArgument
`limit must be >= 0, got {limit}`.

Example block: `{"path": "Projects/Lighthouse Grid.md", "via": "Orbital Freight"}`.

### 5.24 graph_stats

Description: `How many entities, edges and mentions the graph holds.`

No parameters. Graph tool. Returns `{"entities": {type: count…},
"edges": n, "mentions": n}` with the type keys in ascending code-point
order. Example (SV-09):

```json
{
  "entities": {
    "organization": 1
  },
  "edges": 0,
  "mentions": 1
}
```

### 5.25 consolidate_graph

Description: `Ask a model which graph entities are the same thing under different names. The default is a preview that returns the suggested merges. Review them, then call with apply=true to fold each group into one canonical entity.`

| Parameter | Type | Default |
|---|---|---|
| apply | boolean | false |

Behaviour: build the consolidation extractor (06 §5.8; model
`ENIGMA_CONSOLIDATE_MODEL`; it sends the normative prompt of 07 §3.1). apply=false: run consolidation as a dry run
(entity vectors in memory only, no lock). apply=true: refused-apply check
(01 §7), take the caretaker lock, run with apply. Returns:

| Key | Value |
|---|---|
| `entities` | number of live entities considered |
| `proposed` | list of `{"type", "canonical", "duplicates": [names]}` |
| `applied` | true only if apply and at least one merge was applied |
| `merged_entities` | number of duplicates merged |
| `skipped_batches` | number of adjudication batches whose LLM call failed |

With no live entities the result is returned at once (entities 0) and the
backend is never called. Example (SV-11, after indexing one entity, backend
replies `{"merges": []}`):

```json
{
  "entities": 1,
  "proposed": [],
  "applied": false,
  "merged_entities": 0,
  "skipped_batches": 0
}
```

Errors: refused apply; LockBusy; ExtractorError while building the extractor.
Side effects (apply): canonical notes gain mentions, relations and aliases;
duplicates become redirect stubs; `graph.db` rebuilt; entity vector cache
written (07).
