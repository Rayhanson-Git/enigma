# 11 — Recall bundles and the context block

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/recall.yaml`. Formats and error kinds: 05 §0. The MCP tool
`recall(mode, project?, since?)` and its argument checks are in 02-tools; the
CLI `recall` and the `hook session-start` caller are in 14-cli and 15.

Recall is read-only and makes no LLM calls: it packages what the vault
already holds. Every section degrades on its own.

---

## 1. Sections

A *section* runs one producer. Success → the producer's mapping with key
`status` = `ok` appended last. Any error → `{"status": "error", "error":
"{Kind}: {message}"}` (display name of 00 §5). One failing section never fails
the bundle (RC-06).

*notes since(cutoff, folder?)*: notes from 04-search iteration (scoped to
`folder` when given; an absent folder yields none) whose modification time,
as local time, is ≥ cutoff; each `{path, modified: minute stamp}`; sorted by
the `modified` text descending (stable, so notes with the same minute keep
iteration order).

## 2. Digest bundle: digest(since = null, now = current local time)

1. cutoff: `since` given (non-empty) → parse it as year-month-day (4-digit year,
   1–2 digit month and day, calendar-valid, nothing else); failure →
   VaultError `since must be YYYY-MM-DD: {quoted() of since}` (e.g.
   `since must be YYYY-MM-DD: 'last tuesday'`); cutoff is that day's local
   midnight. Not given → now − 7 days.
2. Bundle, keys in this order:

| Key | Value |
|---|---|
| `mode` | `digest` |
| `generated` | now as minute stamp |
| `since` | cutoff as day stamp |
| `librarian` | section `{report: <librarian status, 08 §2>}` |
| `recent_notes` | section `{notes: notes since(cutoff)}` |
| `inbox_captures` | section `{notes: notes since(cutoff, "Claude/Inbox")}` |
| `tasks` | section `{open, done_recently}` (below) |
| `proposals` | section `{pending, detail_note: "Claude/Organizer.md"}` |
| `new_entities` | section `{notes: notes since(cutoff, "Claude/Graph")}` |

- tasks: all tasks including done ones (03-vault-rules task listing); `open` =
  those not done; `done_recently` = done tasks whose `path` is among the paths
  of notes since(cutoff).
- proposals: `pending` = the librarian status's `health.pending_proposals`, or 0
  when there is no health block or no such key.

## 3. Resume bundle: resume(project, limit = 10)

q = casefolded project. Keys in this order: `mode` = `resume`, `project`, then
sections:

| Key | Producer |
|---|---|
| `sessions` | `{notes: [...]}`: for each note under `Claude/Sessions` (read leniently): hay = display() of frontmatter `project` (empty when absent) + space + the file stem, casefolded; keep when q is a substring of hay. Entry `{path, created, excerpt}`: created = display() of a truthy `created`, else the file's mtime minute stamp; excerpt = `body` with whitespace runs collapsed to single spaces (and trimmed), first 400 characters. Sort by `created` text descending (stable); first `limit`. |
| `decisions` | `{lines: [...]}`: `Claude/Decisions.md` absent → `[]`; else read strictly, lines (00 §3.2), keep lines starting `- ` whose casefold contains q. |
| `tasks` | `{tasks: [...]}`: open tasks whose casefolded `text` contains q. |
| `entities` | graph.db missing → `{entities: [], note: "graph cache not built"}` (status ok); else `find_entity(db, project)` (05 §3.3) mapped to `{name, type, path, summary}`. |

Session `created` values compare as text, so `2026-07-03 09:00` sorts after
`2026-07-01 09:00`, and a non-date value such as `July 1` sorts before both
(`J` > `2`) (RC-16).

## 4. Context block: context_block(project, budget = 2000)

1. project absent or empty → empty string.
2. bundle = resume(project, limit 10). parts, in order:
   - if sessions is ok: for its first 3 notes,
     `Session: {title} — {first 300 characters of excerpt}` where title is the
     last path segment with one trailing `.md` removed;
   - if tasks is ok: for its first 5 tasks, `Open task: {text}`;
   - if decisions is ok: its first 3 lines verbatim.
3. text = parts joined by LF. Length (in characters) ≤ budget → text.
   Otherwise cut = first `budget` characters; if cut contains a LF at an index
   > 0, return cut up to (not including) its last LF; else return cut.

No trailing LF is added. Entities are not part of the block.

## 5. CLI behaviour (summary; flags in 14-cli)

`enigma-mcp recall --vault PATH [--context] [--project P] [--budget N]` prints
the context block (no trailing newline) when `--context` is given, nothing
otherwise. Any runtime error is swallowed: no output, exit 0. Only argument
parsing errors exit 2. The session-start hook relies on this (15 §5).

## 6. Edge cases (intended)

1. A frontmatter `project:` with an empty value contributes `null` to the
   session haystack, so a query `none` matches it.
2. `done_recently` uses the modification time of the note that holds the task,
   not when the task was ticked.
3. A budget ≤ 0 yields an empty or near-empty block (character slicing of the
   reference); callers always pass a positive budget.

## 7. Acceptance map

RC-01…RC-16 → §§1–5.
