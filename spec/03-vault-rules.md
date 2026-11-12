# 03 — Vault rules

The vault access layer is the only way Enigma touches notes. It enforces two
rules in code: no path may leave the vault, and writes outside `Claude/` need
explicit confirmation. This file also specifies frontmatter parsing, the
agent-note helpers behind `log_session`, `capture` and `upsert_concept`, the
task helpers, the multi-agent guarantees and the caretaker lock.

Acceptance: `vault.yaml`, `notes.yaml`, `tasks.yaml`, `multi_agent.yaml`,
`locking.yaml`.

## 1. Vault root

Opening a vault resolves the given folder to its canonical absolute path
(symbolic links resolved). If that path is not an existing directory, opening
fails with VaultError `Vault root does not exist: {root}` (root = the resolved
path). See 01 §3 for how the server obtains the folder.

## 2. Path resolution

Every operation takes a **rel path** string as given by the caller and
resolves it:

1. Join it onto the vault root. If the given string is itself absolute, the
   join yields that absolute path (it then normally escapes).
2. Resolve the result: collapse `.` and `..`, follow symbolic links for the
   components that exist, keep a non-existent tail lexically.
3. The resolved path must be the vault root itself or lie inside it
   (component-wise prefix; on Windows compared case-insensitively). Otherwise
   fail with VaultError `Path escapes the vault: {rel}`, where rel is the string
   exactly as the caller gave it.

Consequences: `../outside.md` escapes; a symlink inside the vault that points
outside escapes; the empty string resolves to the root itself.

## 3. The agent-owned folder (quarantine)

A rel path is **inside Claude/** when its resolved path (§2) equals
`<root>/Claude` or starts with `<root>/Claude` followed by the OS path
separator. On Windows both sides are lowercased and `/` is treated as `\`
before comparing; on POSIX the comparison is exact.

| rel | inside Claude/ |
|---|---|
| `Claude/Index.md`, `Claude/Sessions/x.md`, `Claude` | yes |
| `Daily.md`, `ClaudeFake/x.md`, `Claude/../Daily.md` | no |
| `claude/Sessions/x.md`, `CLAUDE/Index.md` | yes on Windows only |

Writes and appends to a path that is not inside Claude/ require
`confirm_outside_claude=true`; otherwise they fail with VaultError:

```text
'{rel}' is outside Claude/. Pass confirm_outside_claude=true only when the user explicitly asked for this write.
```

(rel as given; the single quotes and `True` are literal.) Callers may pass
the confirmation only when the human explicitly asked for that write. Sheets
(10) pass it for rows in registered sheet folders.

## 4. Read

Reading rel:

1. Resolve (§2).
2. If the path is a directory: VaultError `'{rel}' is a directory, not a note.`
3. If it is not a regular file (absent, or any other type):
   VaultError `Note not found: {rel}`
4. Read the bytes, decode strictly as UTF-8 (DecodeError on failure) and
   normalise line endings (00 §3.1). The open-and-read step follows the lock
   retry rule (§5.1).

## 5. Write and append

### 5.1 Lock retry rule

Sync tools, antivirus and Obsidian briefly lock files, mainly on Windows. A
**lock error** is an OS error of the permission-denied class (POSIX `EACCES`
or `EPERM`; Windows access denied, sharing violation, lock violation), except
in the hard-link step of §5.2, where the table there decides and POSIX
`EPERM` means "no hard links" (fall back, never retried). A step
covered by this rule runs up to 5 times: after failed attempt k (k = 1..4) it
sleeps 0.2 × k seconds (0.2, 0.4, 0.6, 0.8 s) and tries again; the fifth
failure surfaces as IoError. Errors that are not lock errors are never
retried. Steps covered: the read in §4, each publish step and each temp-file
deletion in §5.2, the open in §5.3, and file moves in 09.

### 5.2 Write

Inputs: rel, content (text), `overwrite` (default false),
`confirm_outside_claude` (default false). Checks run in this order and the
first failure wins:

1. Resolve (§2) — escape error.
2. Quarantine (§3) — outside-Claude error.
3. If the path is a directory: VaultError `'{rel}' is a directory, not a note.`
4. If the path exists (following symlinks) and `overwrite` is false:
   NoteExists `'{rel}' already exists. Pass overwrite=true to replace it.`

Then:

5. Create missing parent folders.
6. Write the UTF-8 bytes of content, unchanged, to a temp file in the target's
   folder named `.` + 16 random lowercase hex characters + `.tmp-write` (for
   example `.3f9a0c1d22b4e6f7.tmp-write`). The short fixed-length name keeps
   long note names within the 255-byte component limit; the leading dot keeps
   Obsidian from indexing it.
7. Publish:
   - `overwrite=true`: atomically rename the temp file over the target
     (replacing it), under the retry rule.
   - `overwrite=false` (create-only): create a hard link at the target pointing
     to the temp file, under the retry rule. If the target exists at that
     moment: NoteExists (same message as step 4). If the file system cannot
     hard-link (table below), fall back to an **exclusive create**: open the
     target with create-new semantics (the open is under the retry rule); if
     it already exists: NoteExists; otherwise write all bytes; if writing
     fails, delete the partially written target and surface the error. The
     no-hard-link condition itself is never retried.
8. Always (success or failure) delete the temp file, under the retry rule;
   any failure to delete is ignored and never changes the outcome. If writing
   the temp file in step 6 fails, Enigma SHOULD also try to delete it.

The write returns the resolved path internally; tools decide what to return.

Hard-link failure classification:

| OS | Error from creating the link | Meaning |
|---|---|---|
| any | target already exists | NoteExists |
| POSIX | `EPERM` | no hard links on this file system: fall back |
| POSIX | `EACCES` | lock error: retry, then surface |
| POSIX | anything else (e.g. `ENOTSUP`, `EXDEV`, `EMLINK`) | no hard links: fall back |
| Windows | permission-class error | lock error: retry, then surface |
| Windows | anything else (e.g. invalid function, not supported) | no hard links: fall back |

Guarantees: exactly one of any number of concurrent create-only writers of one
path wins, the rest get NoteExists, the file holds exactly the winner's bytes
(with the exclusive-create fallback a reader may briefly see it half written),
and no `*.tmp-write` file remains afterwards.

### 5.3 Append

Inputs: rel, content, `confirm_outside_claude` (default false).

1. Resolve (§2); quarantine (§3); directory check (VaultError
   `'{rel}' is a directory, not a note.`).
2. Create missing parent folders.
3. Open the file for appending, creating it if absent, under the retry rule.
   POSIX: write-only with `O_APPEND | O_CREAT`, mode 0666 before umask.
   Windows: a handle with only `FILE_APPEND_DATA` access, sharing read, write
   and delete, disposition "open always", so every write lands at the end of
   file.
4. Write the UTF-8 bytes of content in one write request, looping only to
   finish a short write. This step is **not** retried: if it fails part-way,
   the error surfaces and the bytes already written stay in the file.
5. Close.

Concurrent appends of whole lines never interleave or overwrite each other.

## 6. Frontmatter

**Fence detection.** A note has frontmatter when its text starts with `---`
and the substring `\n---` occurs at or after index 3. Let end be the index of
the first such occurrence. (Edge case Q4: the first line may be `---x`, and
`\n----` closes.)

**parse(text)** returns a mapping:

1. No frontmatter: `{}`.
2. Parse the characters from index 3 up to end (this includes the remainder
   of the opening line) as one YAML 1.1 document using the safe type set and
   implicit resolution of 16 §2.
3. YAML syntax or construction error: `{}`. A document that is not a
   mapping (scalar, list, empty): `{}`.
4. A scalar that matches the timestamp pattern but is not a valid calendar
   value fails the whole calling operation with InvalidArgument (edge case Q5):
   `that day does not exist in that month`, `the month must be between 1 and 12`,
   `the year must be 1 or later`, `the hour must be between 0 and 23`,
   `the minute must be between 0 and 59` or `the second must be between 0 and 59`.
5. Duplicate keys: the last occurrence wins.

**body(text)**: if the note has frontmatter, the text after the 4 characters
`\n---` (so the rest of the closing fence line, usually `\n`, is kept);
otherwise the whole text. Examples:
body(`---\ntags: [x]\n---\n\n# T\n\nBody.\n`) is `\n\n# T\n\nBody.\n`; a text
without frontmatter or with an unclosed fence is returned unchanged.

**str_list(v)**: null → `[]`; a list → each item rendered with display()
(00 §3.4); any other value → a one-item list of display(v). Examples:
`null` → `[]`, `one` → `["one"]`, `[a, 2]` → `["a", "2"]`, `[true]` →
`["True"]`.

**json_safe(v)** (how frontmatter values appear in tool results): null,
strings, integers, floats and booleans unchanged; lists element-wise; mappings
with every key rendered by display() and values recursively; every other value
(dates, date-times, binary, sets) rendered by display().

## 7. Agent notes

### 7.1 safe_filename(title)

1. Delete every occurrence of these characters: `\ / : * ? " < > | [ ] # ^`
2. Collapse whitespace (00 §3.3).
3. If the result is empty, use `untitled`.

Examples: `a/b\c:d*e?f"g<h>i|j` → `abcdefghij`; `///` → `untitled`;
`Fix ]] the [[bug` → `Fix the bug`; `line1\nline2\ttab` → `line1 line2 tab`.

### 7.2 Agent frontmatter

frontmatter(project, tags, created, agent) is the YAML block-style emission
(16 §2.3) of this mapping, keys in this order:

| Key | Value |
|---|---|
| `created` | minute stamp of created (string) |
| `agent` | agent (default: the agent name, 01 §5) |
| `project` | project (string, default empty) |
| `tags` | list of strings (default empty list) |

wrapped as `---\n` + YAML + `---\n\n`. Example for project `lighthouse`, tags
`[grid]`, created 2026-07-05 14:30, agent `claude`:

```text
---
created: 2026-07-05 14:30
agent: claude
project: lighthouse
tags:
- grid
---

```

An empty project emits `project: ''`; empty tags emit `tags: []`. Values that
need quoting follow 16 §2.3 (e.g. project `grid: outage [pipeline]` emits
`project: 'grid: outage [pipeline]'`), and the result always re-parses to the
same values.

### 7.3 log_session

Inputs: title, content, project, tags (default empty).

1. now = current time. base = day(now) + ` ` + safe_filename(title).
2. stem = base, n = 2. Repeat: rel = `Claude/Sessions/{stem}.md`; create-only
   write (§5.2) of frontmatter(project, tags, now, agent name) + content +
   `\n`. On NoteExists set stem = base + ` ` + n, increment n, and repeat; any
   other error surfaces.
3. Append to `Claude/Index.md` the line
   `- [[{stem}]] — {project}: {title}\n` (title raw, not sanitised; the dash is
   U+2014 EM DASH with a space on each side).
4. Return rel.

Example: title `Relay setup` on 2026-07-05 returns
`Claude/Sessions/2026-07-05 Relay setup.md`; a second `Standup` the same day
returns `Claude/Sessions/2026-07-05 Standup 2.md`.

### 7.4 capture

Input: content. rel = `Claude/Inbox/{day(now)}.md`. Append
`- {clock(now)} {content}\n` (content raw, newlines included) and return rel.
Example: `- 14:30 check bucket pricing\n`.

### 7.5 upsert_concept

Inputs: name, content.

1. filename = safe_filename(name) + `.md`.
2. **Lookup**: if `Claude/Concepts` resolves to a directory, scan its direct
   entries in path order and return `Claude/Concepts/{entry}` for the first
   entry whose name casefolds equal to filename casefolded; otherwise none.
3. If lookup found none: rel = `Claude/Concepts/{filename}`; create-only write
   of frontmatter(project empty, tags `[concept]`, now, agent name) +
   `# {name}\n\n{content}\n` (name raw). On success return rel. On NoteExists
   (another agent won the race), existing = lookup again, or rel if lookup
   still finds none (for example when the file name differs only by Unicode
   normalisation); continue with step 4. Never overwrite.
4. **Extend** existing: heading = `## Update {day(now)}`, plus
   ` ({agent name})` when the agent name is not `claude`. Append
   `\n{heading}\n\n{content}\n` and return existing.

Example: with agent `researcher` the heading is
`## Update 2026-07-05 (researcher)`.

## 8. Tasks

Constants: tasks file `Claude/Tasks.md`; due marker U+1F4C5 (📅). Seed
content, byte for byte (shown inside a tilde fence; the seed ends with
`## Open` followed by one empty line, i.e. the bytes `## Open\n\n`):

~~~~text
---
agent: claude
tags: [tasks]
---

# Tasks

## Live view (all open tasks, vault-wide)

```tasks
not done
group by filename
```

## Open

~~~~

**add_task(content, due)**:

1. If the tasks file does not exist, create-only write the seed; ignore
   NoteExists (another agent seeded it whole).
2. line = `- [ ] ` + collapse_whitespace(content).
3. If due is non-null and non-empty, validate it (below) and append
   ` 📅 {due}` (space, U+1F4C5, space, due exactly as given) to line.
4. Append line + `\n` to the tasks file. Return `Claude/Tasks.md`.

Step 1 happens before validation, so an invalid due still creates the seed.

**Due validation** (InvalidArgument on failure). due must match, from its
first character, the pattern `YYYY-M-D` where YYYY is exactly 4 ASCII digits,
M is one of `10`–`12`, `01`–`09`, `1`–`9` (tried in that order), D is one of
`30`–`31`, `10`–`29`, `01`–`09`, `1`–`9`, or a space followed by `1`–`9`
(tried in that order), and must then be a real calendar date. Messages:

| Case | Message |
|---|---|
| no match at the start | `{quoted(due)} is not a date in the form YYYY-MM-DD` |
| match leaves characters | `unexpected text after the date: {rest}` |
| day beyond month length | `that day does not exist in that month` |
| year 0000 | `the year must be 1 or later` |

Examples: `2026-07-10` and `2026-7-5` are valid; `tomorrow` gives
`'tomorrow' is not a date in the form YYYY-MM-DD`; `2026-07-100` gives
`unexpected text after the date: 0`; `2026-02-30` gives
`that day does not exist in that month`.

**list_tasks(include_done, folder)**: walk notes (04 §2, scoped to folder when
given, which is resolved per §2 and may escape-fail), read each leniently, split
into lines, and match each line against: optional leading whitespace, `-` or
`*`, one space, `[`, one of space / `x` / `X`, `]`, one space, then at least
one character to the end of the line. For each match emit
`{"path": rel, "text": strip(rest), "done": state is x or X}`, skipping done
tasks unless include_done is true. Order: walk order, then line order.

## 9. Multi-agent guarantees

Several agents run separate Enigma processes on one vault at once. With the
rules above:

- 16 concurrent create-only writes of one path: one winner, 15 NoteExists.
- 8 concurrent `log_session` calls with one title: 8 distinct paths
  (`…Standup.md`, `…Standup 2.md`, …), every body present once, every one
  indexed in `Claude/Index.md`.
- 8 concurrent `upsert_concept` calls of one name: one note; every text
  present (one create, the rest extensions).
- Concurrent appends (16 agents × 25 lines): every line present exactly once.
- 8 concurrent first `add_task` calls: the seed appears once; all 8 lines
  present.
- Each note records its writer in `agent:` (01 §5).

## 10. Caretaker lock

Caretakers (00 §2) serialise through one OS file lock per vault.

- Lock file: `caretaker.lock` in the state dir (01 §4). It is opened for
  reading and appending, created if absent, never truncated and never deleted.
- Acquisition is non-blocking and exclusive. POSIX: `flock` with
  `LOCK_EX | LOCK_NB` on the whole file (so two acquisitions in the same
  process through two opens also conflict). Windows: a non-blocking lock of
  one byte at offset 0.
- If the lock is held elsewhere, fail at once with LockBusy:

```text
this vault is being written by another maintenance run (Librarian, indexer, organizer or consolidation); retry once it has finished ({path})
```

  where path is the lock file's absolute path.
- Release: when the guarded work ends, successfully or not (explicit unlock on
  Windows; closing the file on POSIX). The OS also releases it when the
  process dies, so a killed caretaker never leaves a stale lock.

Who takes the lock:

| Operation | Lock |
|---|---|
| `index_brain` (both `force` values) | always |
| `organize_vault(apply=true)`, `consolidate_graph(apply=true)` | yes |
| `organize_vault` / `consolidate_graph` dry runs | no |
| `enigma-mcp index` (including `--rebuild-only`) | always |
| `enigma-mcp librarian` | unless `--dry-run` |
| `enigma-mcp organize` | unless `--dry-run` |
| `enigma-mcp consolidate --apply` | yes |
| `enigma-mcp cleanup --apply-repairs` / `--apply-retirements` | yes |

Note-writing tools (`log_session`, `capture`, `write_note` …) never take it;
they rely on §5. The refused-apply check (01 §7) runs before the lock is
taken. A LockBusy from a tool reaches the client as an ordinary tool error.
