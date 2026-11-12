# 05 — Graph: vault queries, entity notes, graph cache

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance: `acceptance/graph.yaml`.

## 0. Conventions used in 05–15

Files 05–15 use the shared conventions of 00 §3 (lines, whitespace, casefold,
display() and quoted(), timestamps, lenient and strict reads), the error kinds
of 00 §5, and the exact JSON and YAML formats of 16. Additional terms:

- **File JSON** (16 §3): every JSON file Enigma writes in the state directory
  and every JSON a CLI command prints. "Compact" and "indented" select the two
  layouts of 16 §3.
- **float text** (16 §3.4): `1.0`, `0.5`, `0.667`, `1e-05`.
- **n-decimal**: the value rounded half-to-even on its exact binary value and
  printed with exactly n decimals (`0.85`, `1.00`). **rounded to 3 places**: the
  same rounding, kept as a float and printed as float text (`0.667`, `1.0`).
- **YAML dump** in *block style* or *flow-leaf style*, including the
  one-line frontmatter rule, scalar quoting, ASCII escaping and folding at
  width 80: defined only in 16 §2.3. The dump ends with LF; callers add
  fences.
- **truthy** (for frontmatter values): present and not null, false, 0, 0.0,
  empty string, empty list or empty mapping.
- Frontmatter helpers **parse**, **body**, **str_list** and **json_safe** are
  those of 03 §6; **walk** and note iteration order are those of 04 §2;
  **safe_filename** is 03 §7.1; the **caretaker lock** is 03 §10; the
  **lock retry rule** is 03 §5.1; the **state directory** is 01 §4.

Kinds used in 05–15 (all from 00 §5): VaultError, NoteExists, LockBusy,
ProtectedLink, SheetError, ExtractorError, InvalidArgument, DecodeError,
InvalidData, IoError, NotFound, EvalConfigError, ProvisionError, SyncError,
Internal. Where these files show `{Kind}: {message}` the kind's display name
from 00 §5 is used.

---

## 1. Vault metadata queries

`query_notes`, `get_backlinks` and `list_recent` read note frontmatter and
links directly, not the graph; they are specified in 02 §5.17–§5.19
(acceptance GQ-01…GQ-07). Nothing in this file changes them.

---

## 2. Entity notes (the markdown graph store)

The graph lives as markdown under `Claude/Graph/`. Markdown is the source of
truth; graph.db (§3) is a disposable cache.

### 2.1 Layout and names

| Entity type | Folder |
|---|---|
| person | `Claude/Graph/People` |
| organization | `Claude/Graph/Organizations` |
| domain | `Claude/Graph/Domains` |
| topic | `Claude/Graph/Topics` |
| project | `Claude/Graph/Projects` |
| source | `Claude/Graph/Sources` |

- *entity rel path* for (type, name) = `Claude/Graph/{Folder}/{safe_filename(name)}.md`
  (`safe_filename` from 03-vault-rules). `Bad:Name?` as person →
  `Claude/Graph/People/BadName.md`.
- *entity path* = a vault-relative path with a trailing `.md` removed (only if
  present). Mentions, relations, cache rows and redirects use entity paths.
- A note's *stem* is its file name without `.md`.
- A note is a *stub* when its frontmatter `merged_into` is truthy, and a
  *tombstone* when `retired` is truthy ("truthy": present and not null, false,
  0, empty string, empty list or empty mapping). Stubs and tombstones are
  never *live*.
- `REDIRECT_MAX_DEPTH` = 5.

### 2.2 New-note template

Created with a create-only write (03-vault-rules). Content:

1. `---` LF, then YAML dump (block style) of the mapping, in this order:
   `created` (local minute stamp), `agent` (always the literal `claude`; the
   `ENIGMA_AGENT_NAME` setting does not apply to graph notes), `entity` (the type),
   `aliases` (list, possibly empty), `tags` (one item `graph/{type}`); then
   `---` LF LF.
2. `# {name}` LF LF.
3. If the summary is non-empty: the summary LF LF.
4. `## Mentions` LF LF `## Relations` LF.

Example (ORBITAL, created 2026-07-05 14:30):

```
---
created: 2026-07-05 14:30
agent: claude
entity: organization
aliases:
- ORBF
tags:
- graph/organization
---

# Orbital Freight

A company.

## Mentions

## Relations
```

### 2.3 Line grammar and sections

- **Mention line** (regex, matched against a whitespace-trimmed line):
  `^- \[\[([^\]|]+)\|[^\]]+\]\](?:\s+[—-]\s+(.*))?$` — group 1 is the
  mentioning note's path without `.md`, group 2 the optional evidence.
  Written form: `- [[{note path without .md}|{note stem}]]`, plus ` — {evidence}`
  (space, U+2014, space) when evidence is non-empty.
- **Relation line**: `^- (\w+) \[\[([^\]|]+)\|[^\]]+\]\]$` — group 1 the relation
  name (Unicode word characters), group 2 the target entity path. Written
  form: `- {relation} [[{target entity path}|{target stem}]]`.
- **section(text, header)**: from the first occurrence of `header` anywhere in the
  text (plain substring search, not line-anchored) up to, not including, the
  next occurrence of LF followed by `## ` that starts after the header text;
  to end of text if none; empty when the header is absent. Headers are
  `## Mentions` and `## Relations`.
- **section lines** = the lines (00 §3.2) of a section that start with `- ` (untrimmed).
- **entity summary(text)**: take `body`; cut it at the first occurrence of
  `## Mentions`; drop every lines (00 §3.2) line beginning `# `; join the rest with LF;
  trim surrounding whitespace. `# Foo\n\nLine one.\n\n---\n\nLine two.\n\n## Mentions\n…`
  → `Line one.\n\n---\n\nLine two.`.

### 2.4 resolve_redirect(entity path)

Follow `merged_into` links from an entity path. Make at most
REDIRECT_MAX_DEPTH + 1 = 6 visits; at each visit of path P:

- P already visited → none (cycle).
- P + `.md` escapes the vault or is not a file → none.
- read leniently; `retired` truthy → none.
- `merged_into` falsy → **P** (the live end).
- otherwise continue with display() of `merged_into`.

After 6 visits without a live end → none. So a chain Old → Mid → Canonical
resolves, and a chain needing 7 visits does not.

### 2.5 find_entity_note(type, name)

Scan `*.md` files directly in the type's folder (not recursive) in ascending
file-name order (absent folder → none). A file matches when its stem
casefolds equal to `name`, or casefolds equal to `safe_filename(name)`, or
`name` casefolded equals one of its casefolded `str_list(aliases)`. The first
match wins:

- not a stub → its rel path `Claude/Graph/{Folder}/{file name}`;
- a stub → `resolve_redirect(display() of merged_into)` + `.md` when that resolves,
  else the stub's own rel path. Tombstones are returned as matches.

### 2.6 merge_aliases(rel, candidates) → bool

Read the note (strict). Current aliases = `str_list(aliases)`. *Known* = casefolds
of current aliases plus the casefolded stem of `rel`. *Added* = candidates, in
order, that are non-empty and whose casefold is not in Known (Known is not
updated while scanning, so two equal new candidates are both added). If Added
is empty → false. If the text has no LF + `---` after position 3 → false and the
file is untouched. Otherwise set `aliases` = current + Added in the parsed
mapping (keeping key order; a new key goes last) and overwrite the file with
`---` LF + YAML dump (flow-leaf style) of the mapping + `---` + everything after the
closing fence's four characters (LF `---`). Return true.

### 2.7 upsert_entity(entity) → (rel, created)

Entity = `{name, type, aliases, summary}`.

1. `find_entity_note(type, name)` is none → write the §2.2 template at the
   entity rel path (create-only) → (rel, true).
2. Found note is a tombstone → *revive*: aliases = entity aliases followed by
   the tombstone's aliases whose casefold is neither an entity alias casefold
   nor the entity name casefold; overwrite the note with a fresh §2.2 template
   (now as created) → (rel, true).
3. Otherwise candidates = entity aliases, plus the entity name when its
   casefold differs from the found note's stem casefold; `merge_aliases` →
   (rel, false). The summary is never rewritten.

### 2.8 Section insert, mentions, relations

**insert_line(rel, header, line, marker) → bool**: read the note (strict). If
`header` does not occur, the text becomes text with trailing whitespace
removed + LF LF + header + LF. Find the section (§2.3). If `marker` occurs in the
section → false (no write). Otherwise the insertion point is the start of the
next LF `## ` after the header, or the end of text; the new text is (text
before the insertion point with trailing whitespace removed) + LF + line + LF +
(the rest from the insertion point, only when a next header exists).
Overwrite → true.

| Operation | Marker | Line |
|---|---|---|
| add_mention(entity rel, note path, evidence) | `[[{note without .md}\|` | mention form (§2.3) |
| add_relation(src rel, relation, dst rel) | `- {relation} [[{dst without .md}\|` | relation form |

(In this table `\|` stands for one plain `|`; each marker ends with that character.)

**remove_mention(entity rel, note path) → bool**: read strictly; drop every
line (split keeping ends (00 §3.2)) that contains `[[{note without .md}|` anywhere;
none dropped → false; otherwise overwrite with the remaining lines → true.

### 2.9 apply(note path, extraction) → counts

Counts `{entities_created, entities_merged, mentions_added, relations_added}`
(that key order). A per-call memo maps (type, casefolded name) → rel; *ensure*
consults it and otherwise calls `upsert_entity`, counting created/merged.

1. For each extracted entity in order: ensure it (with its aliases and
   summary), then `add_mention(rel, note path, evidence = its summary)`;
   count true results.
2. For each relation in order: ensure `from` as `{name: from, type: from_type, aliases: [], summary: ""}`,
   ensure `to` likewise, then `add_relation(src, rel, dst)`; count true results.
   Relation evidence is not stored.

---

## 3. Graph cache (graph.db)

A SQLite file `graph.db` in the vault's state directory (01 §4; inventory in 16 §1).

### 3.1 Schema

The table definitions and column meanings are specified once, in 16 §4.
`aliases` holds a compact File JSON array. Queries read rows in insertion
(rowid) order.

### 3.2 rebuild(vault, db)

1. Create the parent directory. Delete `graph.tmp` beside the db if present; create
   it fresh with the schema of 16 §4.
2. For every `*.md` under `Claude/Graph/` (recursive, 04-search path order;
   nothing if the folder is absent), read leniently and skip stubs and
   tombstones. Insert an entity row:
   - path: entity path; type: display() of `entity`, or `topic` when falsy;
   - name: the first line anywhere in the text matching `^# (.+)$`, trimmed; else
     the stem;
   - aliases: `str_list(aliases)`;
   - summary: take the text after the first LF `# ` (the whole text if none);
     skip its first line; the first remaining lines (00 §3.2) line that is not blank
     and does not start with `#`, trimmed; else empty.
3. Mention rows from the `## Mentions` section: each trimmed line matching the
   mention regex → (entity path, group 1 trimmed, group 2 or empty).
4. Edge rows from the `## Relations` section: each trimmed line matching the
   relation regex → (entity path, group 1, group 2 trimmed).
5. Commit, close, atomically replace the db with `graph.tmp`.

### 3.3 Reads

- **note_entity_paths(db, note)**: strip `.md`; distinct `entity_path` values of
  mentions with that `note_path`, sorted ascending (code-point order).
- **entity record** for an entities row: `{name, type, path: path + ".md", summary, aliases, relations, mention_count}`
  where `relations` = `[{rel, to}]` over edges with that `src_path` (row order),
  `to` = the name of the entity row whose path is `dst_path` (last row wins on
  duplicates), else the last path component of `dst_path` minus its last extension;
  `mention_count` = number of mention rows for the entity.
- **find_entity(db, query, type?)**: for entity rows in order, keep those
  where the casefolded query is a substring of the casefolded name or of any
  casefolded alias, and (if `type` is a non-empty string) whose type equals it
  exactly. Return records.
- **entities_at(db, paths)**: the record for each path that has a row, in the
  given order.
- **stats(db)**: `{entities: {type: count}, edges: n, mentions: n}`; the type map
  lists types in ascending order.

### 3.4 related_notes(db, path, hops = 2, limit = 20)

1. `limit` < 0 → InvalidArgument `limit must be >= 0, got {limit}`.
2. lookup = path without `.md`. Seeds = `entity_path` of mention rows whose
   `note_path` = lookup, in row order. Each seed is *reached* at hop 1 with chain =
   its name (entity row name, else last path component minus extension).
3. Repeat `max(0, hops − 1)` times, hop increasing from 2: for each entity in
   the frontier (in order), list neighbours = edges where it is `src_path`
   (yielding (rel, dst_path)) united with edges where it is `dst_path` (yielding
   (rel, src_path)), as a de-duplicated set ordered ascending by (rel,
   other path). Each neighbour not yet reached becomes reached at this hop with
   chain `{parent chain} ({rel}) {neighbour name}` and joins the next frontier.
4. For each reached entity in reach order, for each mention row of it: skip
   note paths equal to lookup or starting `Claude/Graph/`. The result entry for
   `note + ".md"` is created on first sight with `via` = that entity's chain; its
   score increases by 1/hop; if this hop is smaller than the entry's best hop,
   `via` becomes this chain.
5. Sort by score descending, then path ascending; return the first `limit` as
   `[{path, via}]`.

---

## 4. Edge cases (intended)

1. **Substring section headers** (00 Q1). `section` finds the header by
   plain substring: `## Mentions` inside a summary or in a heading such as
   `### Mentions` or `## Mentions of x` is taken as the section start.
2. **Loose fences** (00 Q4) apply to entity notes as to every note.
3. **Cache name/summary heuristics.** The cache name is the first `# ` line
   anywhere; a note without an H1 takes its summary from the second line of
   the whole text (often a frontmatter line).
4. **Graph agent stamp.** Graph notes always say `agent: claude`.
5. **Duplicate new aliases** (00 Q10). `merge_aliases` adds the same new
   alias twice when it appears twice among the candidates.

## 5. Acceptance map

GQ-01…GQ-07 → 02 §5.17–§5.19; GC-01…GC-03 → §3; GS-01…GS-13 → §2.
