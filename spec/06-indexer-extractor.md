# 06 — Incremental indexer and LLM extractor

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/indexer.yaml`, `acceptance/extractor.yaml`. Shared conventions:
05 §0; formats: 16; error kinds: 00 §5.

The indexer turns new or changed notes into graph facts: it asks an LLM CLI
(the *extractor*) for entities and relations, writes them into entity notes
(05 §2.9), and rebuilds the graph cache (05 §3).

---

## 1. State directory

Every per-vault artefact lives in the state directory of 01 §4. The
inventory of its files, their formats and owners is in 16 §1.

## 2. Manifest

`manifest.json` records what has been extracted. Its on-disk format (keys,
indented File JSON, written in place) is in 16 §6. This section is the only
normative statement of its load, save and rename rules.

- **Load**: file absent → both maps empty. Present → parse JSON; an unparseable
  file fails the operation with InvalidData (never silently reset). A missing or null `hashes` /
  `failures` key becomes an empty map. A failure value that is a plain string
  `s` becomes `{"error": s, "attempts": 1}`. Entries for the *caretaker notes*
  `Claude/Organizer.md` and `Claude/Librarian.md` are dropped from both maps.
- **Save**: written to `manifest.json.tmp` beside it, then renamed over it (the
  rename under the lock retry rule of 03 §5.1), in the format of 16 §6, so a
  crash never leaves a half-written manifest. Map
  entries keep insertion order.
- **rename_manifest_entry(old, new)**: in each map that has `old`, move its value
  to `new` (removed from its old position, appended at the end). Save only if
  something moved.

## 3. Scanning and pending work

Constants: `DEFAULT_IGNORE` = (`copilot`), `DEFAULT_BATCH` = 25, `MAX_ATTEMPTS` = 3,
caretaker notes = {`Claude/Organizer.md`, `Claude/Librarian.md`}.

**scan_notes(vault)** → ordered map path → hash. Walk notes in 04-search
iteration order (which already skips `.obsidian`, `.trash`, `.git`) and drop:
paths starting `Claude/Graph/`, the caretaker notes, and paths whose first
segment is an ignored folder. Hash = lowercase hex SHA-256 of the raw bytes.
A walked path whose bytes cannot be read (permission denied, a directory named
`*.md`, a file that vanished) stays in the map with the empty hash `""`, which
never equals a stored hash; it is therefore pending and its read error is
recorded per note by run (§4).

**pending_notes(manifest, scan)** → (pending list, benched count). For each
scanned path in scan order: if it has a failure record with attempts ≥ 3 it is
*benched* (counted, not pending); otherwise it is pending when its hash differs
from `hashes` (or is absent there) or it has any failure record.

## 4. run(vault, extractor, batch = 25, force = false, retry_failures = false, precompute_embeddings = true)

1. Load the manifest. If `retry_failures`, empty the `failures` map.
2. Scan. If `force`: pending = every scanned path, skipped = 0. Otherwise
   (pending, skipped) = pending_notes.
3. todo = first `batch` pending paths; remaining = max(0, pending − batch).
4. Counts (this key order): `processed, entities_created, entities_merged,
   mentions_added, relations_added, mentions_retracted, failed, skipped,
   remaining` — all 0 except skipped and remaining.
5. Before the first todo path, when graph.db exists, load it once for the
   retraction below. If it cannot be loaded, rebuild it from the markdown
   (05 §3) and load it again; only a failure of that rebuild is a run error.
6. For each todo path in order (*per-note work*):
   - Read the note (strict, 03-vault-rules) and call the extractor with
     (path, text).
   - Any error in the per-note work (the read, the extractor, the retraction
     or apply below) is a **per-note failure**: failures[path] = `{error:
     first 300 characters of E, attempts: previous attempts + 1}` (previous =
     0 when none), where E is the message for an ExtractorError and
     `{Kind}: {message}` (00 §5) for every other kind, e.g.
     `DecodeError: invalid UTF-8: byte 0xff at offset 0: not a valid first byte`;
     failed += 1; next path. Errors never stop the run: the note is retried
     on later runs (pending_notes, §3) until it is benched at `MAX_ATTEMPTS`,
     and `retry_failures` clears the records. Work already done for that
     note (for example mentions retracted before apply failed) is not rolled
     back; the retry redoes it.
   - Success → *retract stale mentions*: when graph.db exists, for each entity
     path from `note_entity_paths(db, path)`, `remove_mention(entity path + ".md",
     path)`, counting true results into mentions_retracted; a VaultError from a
     missing entity note is ignored. Then `apply(path, extraction)` (05 §2.9)
     and add its four counts. Record hashes[path] = the scanned hash, remove
     any failure record, processed += 1.
7. Save the manifest (always).
8. If processed > 0 or graph.db does not exist → rebuild the cache.
9. If `precompute_embeddings` → compute note vectors with persistence
   (04-search `get_note_vectors`), using the process's embedder, for every
   walked note whose text can be read (04 §5 lenient read); notes that cannot
   be read are left out, not an error (their failure is in the manifest when
   they were in this batch, and later runs reach them).
10. Return the counts.

**Run errors.** Only these stop a run, because continuing could lose or
corrupt data; each propagates to the caller (CLI exit 1, MCP tool error,
Librarian step error):

| Error | Why it stops the run |
|---|---|
| the vault root is missing or not a directory (01 §3) | there is nothing to index |
| the caretaker lock is held (LockBusy, taken by the caller) | another writer owns the vault |
| `manifest.json` exists but cannot be read or does not parse (§2) | continuing would overwrite the failure and hash history |
| listing the vault fails (the walk of 04 §2 cannot read a folder) | the scan would be incomplete |
| graph.db cannot be loaded and rebuilding it fails (step 5), or the rebuild of step 8 fails | the cache that keeps mentions consistent is unusable |
| saving the manifest fails (step 7) | progress cannot be recorded |
| the embedding model fails while embedding (step 9) | a model fault, not a note fault |

When a run error happens after step 6 began, the manifest is saved first
(when possible), so the work of earlier notes in the batch is kept.

Because retraction runs before apply and uses the cache built by the previous
run, a note whose new extraction no longer names an entity loses that mention
(IX-17), while unchanged mentions are removed and re-added.

The MCP tool `index_brain` and the CLI `index` (14-cli) call `run` under the
caretaker lock. The Librarian drains it in rounds (08 §3.1); when 40 rounds
do not drain it, that step fails with kind Internal and the message
`index did not drain after 40 rounds; {remaining} notes still pending`
(00 §5).

---

## 5. Extractor

### 5.1 Vocabularies

- Entity types: `person`, `organization`, `domain`, `topic`, `project`, `source`.
- Relations: `mentions`, `works_at`, `part_of`, `operates_in`, `about`, `related_to`.

### 5.2 Coercion of a reply object

An *extraction* is `{entities: [...], relations: [...]}`.

Entities (from the reply's `entities`; null/absent/false → none; items that are
not objects are skipped):
- name = display() of `name` (null/false/0/empty → empty), trimmed; empty → drop.
- type = display() of `type`, trimmed, lower-cased; not a known type → `topic`.
- aliases = each element of `aliases` (null/absent → none) as display(), trimmed,
  empties dropped. A string value is iterated character by character (edge case).
- summary = display() of `summary` (falsy → empty), trimmed.
Result order: `{name, type, aliases, summary}`.

Relations (from `relations`, same container rules):
- from / to = display(), trimmed; either empty → drop.
- rel = display(), trimmed, lower-cased; unknown → `related_to`.
- from_type / to_type = display(), trimmed, lower-cased; unknown → `topic`.
- evidence = display(), trimmed.
Result order: `{from, from_type, rel, to, to_type, evidence}`.

### 5.3 Extraction prompt

The prompt is UTF-8 text sent on stdin, never on the command line. The text
in the fence below is **normative byte for byte**:
it is sent as shown, LF line ends, with
`{path}` replaced by the note's rel path and `{content}` by the note text,
and no trailing LF. Each placeholder is replaced once, at its own position,
so braces or placeholder names inside the path or the note are sent as they
are. It contains one non-ASCII character, the em dash U+2014 in
guidance item 4. The list that follows explains what the wording
guarantees (EX-01 checks the lower-cased prompt contains "file path",
"file name" or "filename", "canonical" and "alias"; EX-21 checks the whole
text); it adds no further requirement.

1. The task: turn one note into knowledge-graph data (entities and how they
   are connected).
2. The entity types and relation types of §5.1, each listed in full, with
   any other value declared invalid.
3. The reply format: one JSON object of the shown shape and nothing else.
4. Guidance: keep specific named things; never a file path, file name, note
   title or web address; one entity per real thing under its canonical name
   with variants as aliases; what each type means; one-sentence summaries;
   short evidence; relations only between listed entities.
5. The literal block: `Note path: {path}` LF `Note content:` LF `---` LF
   `{content}` LF `---`.

Normative text:

```text
Task: turn one note from a personal notes vault into knowledge-graph data.
List the things the note is really about and how those things are connected.

Allowed entity types: person, organization, domain, topic, project, source.
Allowed relation types: mentions, works_at, part_of, operates_in, about, related_to.
A type outside these two lists is invalid.

Output a single JSON object and nothing else (no explanation, no code fence), in this shape:
{"entities": [{"name": "...", "type": "...", "aliases": ["..."], "summary": "..."}],
 "relations": [{"from": "...", "from_type": "...", "rel": "...", "to": "...", "to_type": "...", "evidence": "..."}]}

Guidance:
1. Keep named, specific things that matter to this note. Leave out everyday words and things mentioned only in passing.
2. A file path, file name, note title or web address is never an entity, even when the note discusses that file.
3. One entity per real thing. Give it its canonical name, meaning the fullest standard form, and put every other spelling, abbreviation or longer variant in its aliases instead of adding a second entity.
4. Typing: person for a named human; organization for a company, team, product, service or software tool; domain for a field or industry; project for a named piece of work; topic for an idea or theme; source for a cited work such as a paper, book, talk, interview or dataset — the work itself, never the file that holds it.
5. summary is one sentence. evidence is a few words quoted or paraphrased from the note. Both ends of every relation must be entities from your own list.

Note path: {path}
Note content:
---
{content}
---
```

### 5.4 Backend and command line

- Backend = the explicit backend argument, else `ENIGMA_EXTRACTOR` when it is
  set (an empty value is a value here, not unset: 01 §2), else `codex`. Only
  `codex` and `claude` exist; anything else, including the empty name, →
  ExtractorError `Unknown backend: {backend}` when the extractor is created.
  This is the only error raised at creation.
- Base commands: codex → `codex exec`; claude → `claude -p`.
- The command line below is built at **each invocation** (§5.5), not when the
  extractor is created. Resolve the executable name on `PATH` (on Windows
  honouring `PATHEXT`) at that moment. Not found → ExtractorError
  `{backend} CLI not found on PATH` for that invocation, nothing is run and
  nothing is retried. So a missing CLI is an ordinary per-note failure in the
  indexer (§4: recorded in `failures`, the run continues and ends normally;
  later runs retry the note automatically until it is benched at 3 attempts,
  `retry_failures` resets the count, and a later success clears the record,
  IX-21)
  and a skipped batch in consolidation (07 §3.3), never a run or step error.
- argv by the resolved file's extension (case-insensitive):
  `.cmd`/`.bat` → `["cmd", "/c", <path>, <base args>]`; `.ps1` →
  `["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", <path>, <base args>]`;
  otherwise `[<path>, <base args>]`.
- Then, only for claude with a non-empty model: append `--model`, `<model>`.
  codex never gets `--model`.
- Then, unless `ENIGMA_EXTRACTOR_NO_TOOLS` is explicitly off (trimmed,
  lower-cased value in {`0`, `false`, `no`, `off`}): claude appends `--tools`, an empty argument,
  and `--strict-mcp-config` (never `--mcp-config`); codex appends `--sandbox`,
  `read-only`. Note text is untrusted input, so an unset, empty or
  unrecognised value denies the tools.

### 5.5 Invocation

Run argv with the prompt on stdin, capture stdout and stderr (decoded as
UTF-8, invalid bytes replaced), timeout 120 s.

| Outcome | Result |
|---|---|
| timeout (process is killed) | ExtractorError `{backend} timed out after 120s` |
| spawn failure | ExtractorError `failed to run {backend}: {OS error text}` |
| exit code ≠ 0 | ExtractorError `{backend} exited {code}: {stderr summary}` (a signal death reports the negative signal number) |
| exit 0 | stdout (empty string if none) |

**stderr summary**: trim; empty → empty string. Otherwise the last line (00 §3.2)
containing the word `ERROR` or `FATAL` (case-insensitive, word boundaries) —
trimmed, first 300 characters; if none, the last 300 characters of the trimmed
text.

### 5.6 Reply parsing

Let FENCE be three backtick characters.

1. Search for the first match of: FENCE, optional `json` (lower-case only),
   optional whitespace, then the shortest text that starts with `{` and ends
   with a `}` followed by optional whitespace and FENCE (the regex
   FENCE`(?:json)?\s*(\{.*?\})\s*`FENCE with "dot matches newline"). If found,
   JSON-decode the captured text; a failure is a *decode error*.
2. Otherwise, for every `{` in the text in order, try to decode one JSON value
   starting there (trailing text allowed); the first value that is an object
   wins.
3. Otherwise ExtractorError `no JSON object in extractor output`.

JSON decoding accepts `NaN`, `Infinity` and `-Infinity` and keeps the last of
duplicate keys.

### 5.7 complete_json(prompt)

Invoke; parse. If parsing fails (ExtractorError or decode error), invoke again
with prompt + LF LF + `Your previous reply was not valid JSON. Reply with ONLY
the JSON object.` and parse again: a decode error now →
ExtractorError `invalid JSON after retry: {decoder detail}`; an ExtractorError
from the second parse propagates unchanged. Invocation errors are never
retried. So exactly one retry happens (EX-07).

**extract(path, content)** = coerce(complete_json(extraction prompt)).

### 5.8 Factories

| Factory | Model |
|---|---|
| extraction extractor | `ENIGMA_EXTRACT_MODEL`, else `haiku` |
| consolidation extractor | `ENIGMA_CONSOLIDATE_MODEL`, else `sonnet` |

An unset variable gives the default. An empty value is kept as the model,
so claude then gets no `--model` (§5.4) (01 §2).

Tests inject the process runner and the PATH lookup.

---

## 6. Edge cases (intended)

1. A string `aliases` value is split into single characters; a null alias
   element becomes `null`.
2. A note listed in failures is pending even when its hash is unchanged, until
   it is benched at 3 attempts.
3. The manifest save is not atomic.

(The earlier rule "errors other than ExtractorError abort the whole run
without saving the manifest" was dropped after v1: see §4, run errors.)

## 7. Acceptance map

IX-01…IX-24 → §§1–4 (IX-19…IX-22 also 14 §2); EX-01…EX-21 → §5.
