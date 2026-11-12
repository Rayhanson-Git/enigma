# 16 — File formats

Exact on-disk formats: the state dir inventory, the YAML parse and emission
rules used for every frontmatter Enigma reads or writes, the two JSON styles,
the SQLite cache, vector caches, manifest, Librarian state, journals, history
and locks. Algorithms that fill these files are in the files referenced.

## 1. State dir inventory

The state dir is `~/.enigma-mcp/<digest>/` or `ENIGMA_STATE_DIR` (01 §4).

| File | Format | Written by | Atomic | Section |
|---|---|---|---|---|
| `graph.db` | SQLite 3 | cache rebuild (05) | yes (rename from `graph.tmp`) | §4 |
| `fallback_embeddings.json` | compact File JSON | note vectors (04 §5) | yes (temp + rename) | §5 |
| `entity_vectors.json` | compact File JSON | consolidation blocking (07) | yes (temp + rename) | §5 |
| `manifest.json` | indented File JSON | indexer (06), cleanup (12), mover (09) | no (written in place) | §6 |
| `librarian_state.json` | indented File JSON | Librarian (08) | yes (via `librarian_state.json.tmp`) | §7 |
| `organizer_journal.jsonl` | JSON Lines (compact) | organizer, undo (09) | append; undo rewrites it through a temp file and a rename | §8 |
| `organizer_suggestions.json` | indented File JSON | organizer (09 §3.1) | no (written in place) | §8.1 |
| `eval_history.jsonl` | JSON Lines (compact) | evals (13) | append | §9 |
| `caretaker.lock` | empty file, OS lock | caretakers (03 §10) | — | §10 |

All text files are UTF-8 with LF line endings on every OS. Everything except
the journal, the Librarian state and the organizer suggestions state (which
holds the acknowledged suggestions) is derived and may be deleted.

## 2. YAML

Frontmatter is YAML 1.1 with the "safe" type set. Enigma's parser and emitter
MUST behave as below; a general YAML 1.2 library does not (for example it
reads `yes` as a string).

### 2.1 Parsing

- One document. Supported node kinds: block and flow mappings and sequences,
  plain, single-quoted, double-quoted, literal (`|`) and folded (`>`)
  scalars, anchors and aliases, the merge key `<<` (in mappings), comments.
- Quoted and block scalars are strings. Plain scalars are typed by implicit
  resolution (§2.2). Explicit standard tags (`!!str`, `!!int`, `!!float`,
  `!!bool`, `!!null`, `!!timestamp`, `!!binary`, `!!set`, `!!omap`,
  `!!pairs`, `!!seq`, `!!map`) are honoured.
- Construction errors (any other tag, a plain `=` scalar, a plain `<<` used
  as a value, a mapping or sequence used as a mapping key) count as YAML
  errors: frontmatter parses to `{}` (03 §6).
- Duplicate mapping keys: the last one wins.
- Values of types other than null, bool, int, float, string, date,
  date-time, list and mapping (binary, set, omap, pairs) are rare; Enigma
  MAY render them with an implementation-defined display().

### 2.2 Implicit resolution of plain scalars

A plain scalar is tested against these patterns in this order; the first full
match decides its type. Patterns are anchored at both ends; spaces shown in
the patterns are not part of them.

| Type | Pattern (whole scalar) |
|---|---|
| bool | `yes Yes YES no No NO true True TRUE false False FALSE on On ON off Off OFF` (one of these words) |
| float | `[-+]?[0-9][0-9_]*\.[0-9_]*([eE][-+][0-9]+)?`, or `\.[0-9][0-9_]*([eE][-+][0-9]+)?`, or `[-+]?[0-9][0-9_]*(:[0-5]?[0-9])+\.[0-9_]*`, or `[-+]?\.(inf|Inf|INF)`, or `\.(nan|NaN|NAN)` |
| int | `[-+]?0b[0-1_]+`, or `[-+]?0[0-7_]+`, or `[-+]?(0|[1-9][0-9_]*)`, or `[-+]?0x[0-9a-fA-F_]+`, or `[-+]?[1-9][0-9_]*(:[0-5]?[0-9])+` |
| merge | `<<` |
| null | `~`, `null`, `Null`, `NULL`, or the empty scalar |
| timestamp | `[0-9]{4}-[0-9]{2}-[0-9]{2}` (date), or `[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}([Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(\.[0-9]*)?([ \t]*(Z|[-+][0-9]{1,2}(:[0-9]{2})?))?` (date-time) |
| value | `=` |
| string | anything else |

Conversions:

- **bool**: `yes`, `true`, `on` (any listed case) are true; the others false.
- **int**: remove `_`; take the sign; `0` is 0; `0b…` binary; `0x…`
  hexadecimal; any other leading `0` octal (`010` is 8); a value containing
  `:` is base 60 (`1:30` is 90); otherwise decimal. `09` matches no int
  pattern and is a string.
- **float**: remove `_`, lowercase; `.inf` / `-.inf` infinity; `.nan` NaN;
  `:` base 60; otherwise the decimal value. `1e5` and `1.0e5` (no exponent
  sign) are strings; `1.0e+5` is a float; `1.10` is 1.1.
- **timestamp**: date-only form → a date. Date-time form → a date-time with
  the fraction's first 6 digits as microseconds (right-padded with zeros), a
  `Z` suffix meaning UTC, `±HH[:MM]` a fixed offset, no suffix a value
  without offset. Impossible values raise InvalidArgument (03 §6, edge case Q5).

### 2.3 Emission

Enigma emits YAML in two styles. This section is their only definition;
every other file names them ("YAML dump, block style", "YAML dump,
flow-leaf style") and points here.

- **block style** — every collection in block style (lists as `- item`
  lines, mappings as `key: value` lines), except that empty collections are
  written `[]` / `{}`. Used by agent frontmatter (03 §7.2), the graph note
  template (05 §2.2), consolidation stubs (07 §4) and sheet field lines
  (10 §6).
- **flow-leaf style** — a collection whose items (for mappings: keys and
  values) are all scalars is written in flow style (`aliases: [a, b]`); any
  other collection in block style. Used when graph and cleanup code rewrite
  an existing frontmatter (05 §2.6, 12 §4.2, 12 §5).
- **One-line frontmatter rule** (a consequence of flow-leaf style, stated
  because tests pin it): when every value of the top-level mapping is a
  scalar, the whole dump is the single line `{k1: v1, k2: v2}` followed by
  `\n` (scalars quoted per the rules below, inside a flow collection), so the
  rewritten note starts `---\n{k1: v1, k2: v2}\n---`. As soon as one value is
  a list or mapping (even an empty one, which is written `[]` / `{}`), the
  top level is a block mapping and only the all-scalar leaves are flow
  collections.

Common settings: keys in insertion order (never sorted), indent step 2, line
width 80, ASCII-only output (non-ASCII is escaped, see below), LF line breaks,
no document markers, and the output ends with `\n`.

**Continuation indent.** When a scalar wraps, the new line is indented to
the indent column of the collection that directly contains the scalar plus
2: a value of a top-level mapping or an item of a top-level block sequence
continues at column 2; an item of a flow list that is a top-level mapping
value continues at column 4.

**Layout.** A block mapping writes one `key: value` per line at its indent.
When the value is a non-empty block collection, the line ends after `key:`.
A block sequence that is a mapping value is written at the same indent as
the key (`- item` lines directly under `tags:`); a nested block mapping is
indented by 2. A sequence item that is a mapping starts on the `- ` line
(`- k: v`, following keys indented by 2). Flow collections are `[a, b]` and
`{k: v, k2: v2}`: items separated by `, `; before each item, if the current
column is beyond 80, a line break and the indent (2 more than the enclosing
block's) are written instead of the space.

**Scalar representation.**

| Value | Text |
|---|---|
| null | `null` |
| bool | `true` / `false` |
| int | decimal |
| float | `.nan`, `.inf`, `-.inf`; otherwise the float text of §3.4 lowercased, with `.0` inserted before `e` when it has an exponent but no `.` (`1e-05` → `1.0e-05`, `1e+16` → `1.0e+16`) |
| date | `YYYY-MM-DD` |
| date-time | `YYYY-MM-DD HH:MM:SS`, plus `.ffffff` if microseconds are non-zero, plus `+HH:MM` / `-HH:MM` if it has an offset |
| string | per the style choice below |

All non-string scalars are written plain.

**String style choice.** Analyse the string s (n characters):

1. *Indicators.* For each position i with character ch, let "followed by
   space" mean i is the last position or the next character is one of space,
   tab, CR, LF, U+0085, U+2028, U+2029, NUL; "preceded by space" means i is
   0 or the previous character is one of those.
   - s starts with `---` or `...`: flow and block indicator.
   - i = 0: ch in `# , [ ] { } & * ! | > ' " % @` or backtick → flow and block
     indicator; ch in `? :` → flow indicator, and block indicator if followed
     by space; ch = `-` followed by space → flow and block indicator.
   - i > 0: ch in `, ? [ ] { }` → flow indicator; ch = `:` → flow indicator,
     and block indicator if followed by space; ch = `#` preceded by space →
     flow and block indicator.
2. *Characters.* line break characters are LF, U+0085, U+2028, U+2029. A
   character is **special** if it is not LF and not in U+0020..U+007E
   (this includes tab, CR and every non-ASCII character, because output is
   ASCII-only).
3. *Spaces.* leading space / trailing space: s starts / ends with a space.
   leading break / trailing break: s starts / ends with a line break.
   break-space: a space directly after a line break. space-break: a line
   break directly after a space.
4. *Permissions* start all allowed (plain-in-block, plain-in-flow,
   single-quoted), then: leading or trailing space or break → no plain; a
   break-space → no plain, no single-quoted; a space-break or any special
   character → no plain, no single-quoted; any line break → no plain; a flow
   indicator → no plain-in-flow; a block indicator → no plain-in-block.
   The empty string allows plain but is never written plain (step 5).
5. *Choice.* Write **plain** if the relevant plain permission holds (flow when
   inside a flow collection, block otherwise) and s, read back as a plain
   scalar, would resolve to string (§2.2 — so `yes`, `123`, `1.5`, `null`,
   `~`, the empty string, `2026-07-05`, `=`, `<<` are never plain) and, for a
   mapping key, s has no line break. Otherwise write **single-quoted** if
   permitted (and, for a key, s has no line break). Otherwise
   **double-quoted**.

**Writing plain** (values, not keys, may wrap): write a space after the
preceding indicator, then the text. Walk the text as alternating runs of
non-space characters and spaces. When a run of exactly one space ends and the
current output column (characters already on this line) is greater than 80,
write a line break plus the continuation indent instead of that space;
otherwise write the spaces unchanged. Runs of two or more spaces are always
written unchanged.

**Writing single-quoted** (values may wrap): write ` '`, then the text with
every `'` doubled (`''`), then `'`. Spaces wrap as for plain, except that a
space at the very start or very end of the text never wraps. A run of line
breaks is written as: one extra line break if the run starts with LF, then
one line break per character of the run, then the continuation indent. So
`a\nb` as a mapping value is written `'a`, an empty line, then `  b'`.

**Writing double-quoted** (values may wrap): write ` "`, the escaped text,
`"`. Characters U+0020..U+007E other than `"` and `\` are written as is;
others are escaped: NUL `\0`, U+0007 `\a`, U+0008 `\b`, tab `\t`, LF `\n`,
U+000B `\v`, U+000C `\f`, CR `\r`, U+001B `\e`, `"` `\"`, `\` `\\`, U+0085
`\N`, U+00A0 `\_`, U+2028 `\L`, U+2029 `\P`; any other character `\xHH`
(below U+0100), `\uHHHH` (below U+10000) or `\UHHHHHHHH`, **uppercase** hex.
Wrapping: at each position p with 0 < p < n − 1, if (the character at p is a
space, or an escape was just written) and the current column plus the length
of the pending unwritten text exceeds 80, write the pending text followed by
`\`, a line break and the continuation indent; if the character at p is a
space, write `\` right after the indent (the space itself follows).

**Examples** (block mode, as the value of key `k` at top level):

| String | Emitted line |
|---|---|
| `lighthouse` | `k: lighthouse` |
| empty | `k: ''` |
| `2026-07-05 14:30` | `k: 2026-07-05 14:30` (no seconds, so not a timestamp) |
| `2026-07-05` (a string) | `k: '2026-07-05'` |
| `yes`, `123`, `1.5`, `null` | `k: 'yes'`, `k: '123'`, `k: '1.5'`, `k: 'null'` |
| `grid: outage [pipeline]` | `k: 'grid: outage [pipeline]'` |
| `b:c` | `k: b:c` (in a flow list: `'b:c'`) |
| `odd"tag` | `k: odd"tag` |
| `it's` | `k: it's` |
| `#x`, `- x`, ` lead` | `k: '#x'`, `k: '- x'`, `k: ' lead'` |
| `don't: stop` | `k: 'don''t: stop'` |
| `café` | `k: "caf\xE9"` |
| `a—b` (U+2014) | `k: "a\u2014b"` |
| `a\tb` | `k: "a\tb"` |

A list value in block mode: `tags:` then `- grid` lines; empty `tags: []`. In
flow-leaf mode: `aliases: [ORBF, Orbital Inc]`.

## 3. JSON

### 3.1 MCP JSON

Used for tool results (02 §3). Equivalent to Rust `serde_json`'s pretty
printer with a 2-space indent:

- Objects: `{`, newline, members `"key": value` each on its own line indented
  2 more than the enclosing line, separated by `,` at line end, newline,
  closing `}` at the enclosing indent. Arrays likewise with `[` `]`. Empty
  object `{}`, empty array `[]`. No trailing newline.
- Strings: `"` and `\` escaped as `\"` `\\`; U+0008 `\b`, U+000C `\f`, LF
  `\n`, CR `\r`, tab `\t`; other characters below U+0020 as `\u00hh`
  (lowercase hex); everything else, including non-ASCII, written raw as
  UTF-8. `/` is not escaped.
- Integers in decimal. Floats in the shortest round-trip form produced by
  serde_json's default formatter (`1.0`, `0.5`, `0.667`, `1e-5`, `1e16`).
  Non-finite floats are written as the bare tokens `NaN`, `Infinity` and
  `-Infinity` (not strict JSON; kept on purpose in
  text content). In `structuredContent` (02 §3.1)
  they are `null`.
- Dates `"YYYY-MM-DD"`; date-times ISO 8601 with `T` (02 §3.1).

### 3.2 File JSON, compact

Used for vector caches, journals and history lines, and CLI one-line outputs:

- Separators `, ` between items and `: ` between key and value; no newlines.
- Strings: `\"`, `\\`, `\b`, `\f`, `\n`, `\r`, `\t`; other characters below
  U+0020 as `\u00hh`; every non-ASCII character as `\uhhhh` with lowercase
  hex, characters above U+FFFF as a UTF-16 surrogate pair of two such escapes.
  U+007F and `/` are written raw.
- Integers decimal; floats in the float text of §3.4 (`1.0`, `1e-05`);
  NaN / infinities as `NaN`, `Infinity`, `-Infinity`.

Example: `{"rebuilt": true, "db": "/home/u/.enigma-mcp/0a1b2c3d4e5f/graph.db"}`

### 3.3 File JSON, indented

Same scalars and escaping as §3.2, laid out with a 2-space indent: each
member or element on its own line, `,` at line end (no space before the
newline), `": "` between key and value, empty containers `{}` / `[]` on one
line, no trailing newline unless the writer adds one.

```json
{
  "hashes": {
    "Daily.md": "3b5d…"
  },
  "failures": {}
}
```

### 3.4 Float text

The shortest decimal digit string that reads back as the same 64-bit float.
With that digit string as d₁d₂…dₖ × 10^e (d₁ ≠ 0, e the exponent of the
first digit): if −4 ≤ e < 16, write it in positional notation with at least
one digit after the decimal point (`1.0`, `0.0001`, `123.456`,
`1234567890123456.0`); otherwise write d₁, then
`.` and the remaining digits if k > 1, then `e`, the exponent sign (`+` or
`-`) and at least two exponent digits (`1e-05`, `1.5e-05`, `1e+16`). Zero is
`0.0` (`-0.0` for negative zero). In display() (00 §3.4) infinities and NaN
read `inf`, `-inf`, `nan`.

### 3.5 File JSON, indented, literal UTF-8

Used only by `enigma-mcp sheets VAULT --check` (10 §9, 14 §2). Identical to
§3.3 except that non-ASCII characters are written raw as UTF-8 instead of
`\uhhhh` escapes (control characters below U+0020, `"` and `\` are still
escaped as in §3.2). Example: a row path `Hiring/Roles/Café - Chef.md` is
printed as `"Hiring/Roles/Café - Chef.md"`.

## 4. graph.db

SQLite 3 database. Created only by the cache rebuild (05), which:

1. creates the state dir if needed;
2. deletes `graph.tmp` (the db path with its extension replaced by `.tmp`) if
   present;
3. creates `graph.tmp` with exactly these statements (this section is the
   normative schema; 05 §3 refers to it):

```sql
CREATE TABLE entities (path TEXT, name TEXT, type TEXT, summary TEXT, aliases TEXT);
CREATE TABLE edges (src_path TEXT, rel TEXT, dst_path TEXT);
CREATE TABLE mentions (entity_path TEXT, note_path TEXT, evidence TEXT);
```

4. inserts rows (content and order defined in 05 §3.2), commits, closes;
5. atomically renames `graph.tmp` over `graph.db`.

No indexes, primary keys or other tables. Column meanings:

| Table.column | Content |
|---|---|
| entities.path | entity note rel path without `.md`, e.g. `Claude/Graph/Organizations/Orbital Freight` |
| entities.name | first `# ` heading text of the note, else the stem |
| entities.type | `entity` frontmatter value rendered with display(), default `topic` |
| entities.summary | first summary line (05), possibly empty |
| entities.aliases | compact File JSON array of strings, e.g. `["ORBF"]` |
| edges.src_path / dst_path | entity paths without `.md` |
| edges.rel | relation word (`works_at` …) |
| mentions.entity_path | entity path without `.md` |
| mentions.note_path | mentioning note rel path without `.md` |
| mentions.evidence | evidence text or empty string |

Readers check that `graph.db` exists before opening it and never create it.

## 5. Vector caches

Both caches are one compact File JSON object:

```json
{"Daily.md": {"hash": "<64 lowercase hex>", "vec": [0.0123, -0.5, 1.0]}}
```

| File | Key | `hash` |
|---|---|---|
| `fallback_embeddings.json` | note rel path (with `.md`) | SHA-256 of the note text (04 §5) |
| `entity_vectors.json` | entity path (without `.md`) | SHA-256 of the entity identity text (07) |

Saving writes a temp file in the state dir named `<file name>.<8 random
characters from a-z, 0-9 and _>.tmp` (e.g.
`fallback_embeddings.json.k3j9_x0a.tmp`) and renames it over the file. A
missing or unparseable file loads as `{}` (04 §5). Readers must accept any
valid JSON number formatting for the vector values.

## 6. manifest.json

This section is the normative on-disk format; load, save and rename
semantics are in 06 §2. Indented File JSON (§3.3), keys `hashes` then
`failures`, written in place (not atomic), no trailing newline:

| Key | Value |
|---|---|
| `hashes` | object: note rel path → lowercase hex SHA-256 of the note's raw bytes when last indexed successfully |
| `failures` | object: note rel path → `{"error": message (at most 300 characters), "attempts": integer}` |

Both maps keep insertion order. How a missing, partial, legacy or invalid
file is loaded is specified once, in 06 §2.

## 7. librarian_state.json

This section is the normative on-disk format and save mechanism; the
default state, the blocks, load and `status()` rules are in 08 §2 only.
Indented File JSON (§3.3), no trailing newline. Saved by writing
`librarian_state.json.tmp` in the state dir and atomically renaming it over
the file, so an interrupted save never leaves a truncated state. Example after
one sweep:

```json
{
  "last_sweep": "2026-07-09 12:00:00",
  "steps": {
    "index": {"processed": 3, "failed": 0, "remaining": 0},
    "organize": {"moved": 0, "proposals": 0, "skipped": 0},
    "cache": {"rebuilt": true, "by": "index"},
    "cleanup": {"retracted_notes": 0, "...": "..."},
    "consolidate": {"ran": true, "reason": "2 unchecked", "proposed": 0, "skipped_batches": 0}
  },
  "health": {"stale_embeddings": 0, "...": "..."},
  "errors": {},
  "consolidation": {
    "checked_hash": {"Claude/Graph/Organizations/Acme": "<sha256 hex>"},
    "cursor": null,
    "pending_proposals": [],
    "last_pass": "2026-07-09 12:00:00",
    "backstop_last_advance": "2026-07-09 12:00:00"
  },
  "cleanup": {"pending_retirements": [], "last_pass": "2026-07-09 12:00:00"}
}
```

(Shown compacted; the real file uses the indented layout of §3.3 throughout,
and the `"...": "..."` members stand for the full key sets of 08.)

## 8. organizer_journal.jsonl

One compact File JSON object per line, each line ending in `\n`. Moves are
appended; an undo rewrites the whole file with the entry's `undone` set to
true. Blank lines are ignored when reading; a missing file means no entries.

| Key | Value |
|---|---|
| `ts` | second stamp of the move |
| `from` | original rel path |
| `to` | new rel path |
| `share` | vote share as an unrounded float |
| `neighbors` | up to 3 rel paths |
| `rewrites` | list of `{"path": rel, "count": integer}` — notes whose links were rewritten |
| `undone` | boolean |

Example line:
`{"ts": "2026-07-09 12:00:01", "from": "Loose Orbit Note.md", "to": "02 - Orbit/Loose Orbit Note.md", "share": 0.85, "neighbors": ["02 - Orbit/Probe Ops.md"], "rewrites": [], "undone": false}`

### 8.1 organizer_suggestions.json

Indented File JSON, no trailing newline:
`{"open": [{"path": rel, "suggested": folder, "reason": text, "proposed": day stamp}], "acknowledged": [{"path": rel, "suggested": folder, "at": second stamp}]}`.
`open` mirrors the items of the suggestions block in `Claude/Organizer.md`;
`acknowledged` pairs are never proposed again (09 §3.1).

## 9. eval_history.jsonl

One compact File JSON object per eval run, appended, line ending `\n`. Keys in
order: `ts` (ISO second stamp), `git` (short commit id or null), `vault`,
`golden` (paths as given), then the scorecard keys of 13 (`success_at_5`,
`success_at_10`, `recall_at_5`, `recall_at_10`, `mrr`, `skipped`,
`n_queries`, `queries`).

## 10. Locks

**caretaker.lock** — an empty file in the state dir used only as an OS lock
target (03 §10). Created on first use; never written, truncated or deleted.

**enigma-index.lock** — a directory `$TMPDIR/enigma-index.lock` (`/tmp` when
`TMPDIR` is unset) used by `enigma-mcp index-nightly` (14). Created with an
atomic directory create; it holds one file `pid` containing the owner's
process id in decimal followed by `\n`; removed when the run exits. A run that
finds the directory reads `pid`; if that process is alive and its command
line contains `index-nightly` the run skips, otherwise it removes the stale
directory and tries once more.

## 11. Temporary files

| Name | Where | Used by |
|---|---|---|
| `.<16 hex>.tmp-write` | target note's folder | vault write (03 §5.2) |
| `<note name>.tmp-move` | target note's folder | link rewrites during moves (09) |
| `<cache name>.<8 chars>.tmp` | state dir | vector caches (§5) |
| `librarian_state.json.tmp` | state dir | Librarian state (§7) |
| `graph.tmp` | state dir | cache rebuild (§4) |

None remains after a successful operation.

## 12. Smart Connections files (read only)

`<vault>/.smart-env/multi/*.ajson`: UTF-8 text, one `"key": {object},`
fragment per line, later lines superseding earlier ones for the same key.
Enigma never writes them. Parsing rules: 04 §6.

## 13. Vault files Enigma maintains

| Path | Created / changed by | Spec |
|---|---|---|
| `Claude/Sessions/<day> <title>[ n].md` | log_session | 03 §7.3 |
| `Claude/Index.md` | log_session (append) | 03 §7.3 |
| `Claude/Inbox/<day>.md` | capture (append) | 03 §7.4 |
| `Claude/Concepts/<name>.md` | upsert_concept | 03 §7.5 |
| `Claude/Tasks.md` | add_task | 03 §8 |
| `Claude/Graph/<Folder>/<name>.md` | indexer, consolidation, cleanup | 05, 07, 12 |
| `Claude/Organizer.md` | organizer (move log, suggestions block), undo | 09 |
| `Claude/Librarian.md` | Librarian sweep reports | 08 |
| `Claude/README.md`, `Claude/Decisions.md`, `CLAUDE.md`, `AGENTS.md` | conventions install | 15 |
| `Claude/Evals.md` | evals `--init-live` | 13 |
| sheet rows `<sheet folder>/<rendered name>.md` | sheet_upsert | 10 |
