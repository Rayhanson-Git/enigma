# 10 — Structured sheets

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/sheets.yaml` (plus MA-10…MA-12 in `multi_agent.yaml`, SV-20 in
`server.yaml`). Shared conventions (quoted(), display(), YAML dump, File JSON …): 05 §0
and 16; error kinds: 00 §5. Every error in this file has kind **SheetError** unless stated.

A folder outside `Claude/` becomes an agent-writable *sheet* when the human
puts a `_schema.md` in it. `sheet_upsert` is the only agent write path into a
sheet and validates every write; raw `write_note` into the folder stays
quarantined (SH-22, SH-37).

---

## 1. Constants

| Name | Value |
|---|---|
| schema file | `_schema.md` |
| standard columns | `created`, `agent`, `project`, `tags` (always allowed) |
| column types | `string`, `enum`, `date`, `bool`, `url`, `number` |
| date pattern | `^\d{4}-\d{2}-\d{2}$` (shape only, no calendar check) |
| tracking query keys | `^(utm_.*\|ref\|src\|gh_src\|lever-origin)$` (each `\|` here is a plain `|`) |
| filename illegal characters | `\ / : * ? " < >`, the vertical bar, and U+0000–U+001F |
| query operators | `eq ne lt lte gt gte contains missing in nin` |
| ordered column types | `date`, `number` |

## 2. Schema

`{folder}/_schema.md` frontmatter (example):

```yaml
sheet: roles
filename: "{org} - {title}"
key: [org, title]
identity: [posting_id, posting_url]
columns:
  org:        {type: string, required: true, max_length: 120}
  stage:      {type: enum, required: true, values: [Found, Sent, Silent]}
  sent_on:    {type: date}
  visa:       {type: bool}
  posting_url: {type: url, max_length: 500}
```

The body after the frontmatter is the sheet's *instructions*.

**load_schema(folder)**, checks in order:

1. `{folder}/_schema.md` not a file → `No _schema.md in '{folder}' — not a sheet.`
2. Read strictly, parse frontmatter. For each of `sheet`, `filename`, `key`,
   `columns` absent → `{folder}/_schema.md: missing '{key}'.`
3. For each column (mapping order): the spec is not a mapping, or its `type` is
   not a column type → `{folder}/_schema.md: column '{name}' has invalid type
   '{display() of type, null when absent}' (allowed: ['bool', 'date', 'enum',
   'number', 'string', 'url']).` An enum without truthy `values` →
   `{folder}/_schema.md: enum column '{name}' needs 'values'.`
4. `key` not a list, or `identity` present and not a list →
   `{folder}/_schema.md: field '{key|identity}' expected list, got {quoted()}.`
5. Result: name = display()(sheet), folder, filename = display()(filename), key and
   identity = display() of each item (identity [] when absent); per column
   `{type, required: truthiness of required (default false), values: display() of
   each value or null, max_length: as given or null}`.

**discovery**: every `_schema.md` in the vault (recursive, path order), skipping
paths with a `.obsidian`, `.trash` or `.git` component and folders inside
`Claude/` (03-vault-rules `in_claude`; a schema at the vault root has folder
`""`). Each loads independently: a SheetError puts `{folder, error}` in the
*invalid* list and the folder is skipped. Two valid schemas with the same name
→ `Duplicate sheet name '{name}': registered in both '{first folder}' and
'{second folder}'.` (this breaks every sheet tool until fixed). Registry =
name → folder in discovery order.

**get_schema(name)**: not registered → `Unknown sheet '{name}'. Registered:
{quoted() of the sorted names, or none}.` (e.g. `Registered: ['roles'].`,
`Registered: none.`). **is_sheet_folder(folder)**: `{folder}/_schema.md` is a file.

## 3. Normalisation

- **norm_str(v)**: display()(v), whitespace runs → one space, trimmed, casefolded.
- **normalize_link(url)**: split display()(url) trimmed into scheme, host, path,
  query, fragment (RFC 3986 generic split); keep query pairs (blank values
  kept, `+` and percent-escapes decoded) whose key does not match the tracking
  pattern; sort pairs by (key, value); rebuild as scheme lower-cased + `://` +
  host lower-cased + (path without trailing `/`, or `/` if that leaves it empty)
  + (`?` + form-encoded pairs, if any); the fragment is dropped.
  `HTTPS://Careers.Example.org/r/77/?utm_medium=x&q=2#top` →
  `https://careers.example.org/r/77?q=2`.
- **normalize value**: a YAML date → `YYYY-MM-DD`; a date-time →
  `YYYY-MM-DDTHH:MM:SS` (ISO); anything else unchanged.

## 4. Field validation

**check(name, column, value)** (value non-null):

| Type | Accepts | Error text |
|---|---|---|
| enum | value equal to one of `values` (exact, strings only) | `Field '{n}': expected one of {quoted() values}, got '{display() value}'.` |
| date | a YAML date, or a string matching the date pattern | `Field '{n}': expected date YYYY-MM-DD, got '{display() value}'.` |
| bool | a boolean | `Field '{n}': expected bool, got '{quoted() value}'.` |
| number | integer or float, not boolean | `Field '{n}': expected number, got '{quoted() value}'.` |
| url | non-empty string without whitespace | `Field '{n}': expected a URL with no whitespace, got '{quoted() value}'.` |
| url | scheme `http`/`https` and a host | `Field '{n}': expected an http(s) URL with a host, got '{quoted() value}'.` |
| url, string | length ≤ max_length (when truthy) | `Field '{n}': exceeds max_length {m} ({len} chars).` |
| string | a string | `Field '{n}': expected string, got '{quoted() value}'.` |

Lengths count Unicode scalar values. Note the quoted() forms are wrapped in an
extra pair of single quotes (`got ''yes''`).

**validate_fields(schema, fields, require_required)** → fields unchanged:
for each field in order, skip standard columns; an undeclared name →
`Field '{n}' is not declared in sheet '{sheet}' (columns: {quoted() of sorted
column names}; standard: ['agent', 'created', 'project', 'tags']).`; null
values are skipped (absent); others are checked. When `require_required`:
required columns (column order) whose field is absent or null →
`Missing required field(s): {quoted() list}.`

## 5. Rows and matching

- **rows**: `*.md` files directly in the sheet folder (not sub-folders), path
  order, excluding `_schema.md`; each read leniently (lock-retry) → (path
  `{folder}/{file name}`, frontmatter).
- **render_filename(schema, fields)**: substitute `{column}` placeholders of
  the filename pattern with display() of the field (empty when absent) for every
  declared column (a placeholder naming anything else is an error); replace
  each illegal character with `-`; collapse whitespace runs to one space; trim;
  cut to 120 characters; trim trailing whitespace.
  `{org: "X/Y: Ltd?", title: "Ops|Lead"}` → `X-Y- Ltd- - Ops-Lead`.
- **identity value(meta)**: the first identity column whose value is neither
  null nor `""` → (column, normalize_link(value) for url columns else
  norm_str(value)); none → none.
- **match_row(fields)** → (row path or none, backfill):
  1. candidates = rows whose norm_str of every key column equals the incoming
     norm_str (absent = `""`).
  2. Incoming identity present: a candidate with equal identity value →
     (it, {}). Else, if exactly one candidate exists and it has no identity →
     (it, {identity column: incoming value}) — a backfill. Else (none, {}).
  3. No incoming identity: one candidate → (it, {}); none → (none, {});
     several → `Ambiguous match: multiple rows share this key — supply
     {quoted() identity list} to disambiguate. Candidates: {quoted() list of
     candidate paths}` (no final full stop).

## 6. Writing rows

**Frontmatter lines**: a string value matching the date pattern is written
`{key}: {value}` unquoted; any other value is the YAML dump (block style, 16 §2.3) of the
one-entry mapping, trimmed (so lists become `key:` followed by `- item` lines).

**Status log**: when the changes include `status`, the line is
`- {day stamp} status: {from, or (new)} → {to} (agent: {agent})` (display() values).
*append to Log*: find the first line that is exactly `## Log` plus optional
spaces/tabs. None → body without trailing LFs + LF LF `## Log` LF + line + LF.
Found → find the next heading line (1–6 `#` then a space) after it; none → body
without trailing LFs + LF + line + LF; otherwise the section text between the
two headings loses its trailing LFs and becomes section + LF + line + LF LF
(or LF + line + LF LF when the section was empty). `## Logistics` is never the
log.

**create(merged, body, agent, now, suffixed)**:
1. validate_fields(merged, require_required = true).
2. meta = merged, then `created` = now minute stamp and `agent` = agent (an
   existing key keeps its position; new keys go last).
3. text = `---` LF + each meta line + LF + `---` LF LF + body (empty when none),
   after appending the status log line when `status` is among merged.
   changed = `{k: {from: null, to: v}}` for every merged field.
4. stem = render_filename. Write `{folder}/{stem}.md` create-only with
   `confirm_outside_claude`. On NoteExists: not suffixed → re-raise; suffixed →
   try `{stem} 2`, `{stem} 3` … until a write succeeds.
5. Return `{result: "created", path, changed}`.

**sheet_upsert(sheet, fields, body = null, agent = null)**:
1. agent = argument, else the instance agent name (03-vault-rules).
2. schema = get_schema; (row, backfill) = match_row(fields).
3. No row → create(fields + backfill, unsuffixed). If that raises NoteExists
   (another agent created the file meanwhile) re-match: still no row → create
   suffixed; a row → continue as an update of it.
4. Update: body given → `body may only be supplied on create; omit it when
   updating an existing row — sheet_upsert never edits an existing body (the
   '## Log' status append is the sole exception).`
5. merged = fields + backfill without `created` and `agent`;
   validate_fields(require_required = false).
6. Read the row strictly. Split: if the text starts with `---` and has LF `---`
   after position 3, the frontmatter lines are the text from position 4 to that
   point (lines (00 §3.2)) and the body is the rest after the closing `---` with leading
   CRs, then leading LFs, removed; otherwise no lines and the whole text.
7. changed = for each merged field whose normalised stored value differs from
   the incoming value: `{field: {from: normalised stored (null if absent), to:
   incoming}}`. Empty → return `{result: "updated", path, changed: {}}` without
   touching the file.
8. Patch the lines with the changed values, then patch `agent`. *Patching*: a
   line starting a top-level key (`^([A-Za-z_][\w-]*):`) whose key is being
   set is replaced by its new line and every following line up to the next
   top-level key line is dropped (block lists, comments); keys not found are
   appended in order. Other lines stay byte-identical.
9. Append the status log line when `status` changed. Overwrite with `---` LF +
   lines joined by LF + LF `---` LF LF + body (confirmed outside Claude/).
10. Return `{result: "updated", path, changed}`.

`created` is never patched and never appears in changed; `agent` always becomes
the effective agent and never appears in changed (SH-34).

## 7. Query

**sheet_query(sheet, filters = null, sort = null, limit = 50)**. Filters map a
column to `{operator: expected}`.

1. Validate all filters first: unknown operator → `Unknown operator '{op}'
   (allowed: ['contains', 'eq', 'gt', 'gte', 'in', 'lt', 'lte', 'missing',
   'ne', 'nin']).`; `lt lte gt gte` on a column whose type is not date/number
   (or an unknown column) → `Column '{col}' ({type, or null}) does not support
   ordering operators; only ['date', 'number'] columns do.`
2. A row matches when every operator holds (AND):
   - `missing`: (value is null or `""`) equals truthiness of expected;
   - value null or `""`: true only for `ne` and `nin`;
   - otherwise both sides are normalised (§3) and `eq`/`ne` compare equal /
     not equal, `in`/`nin` test membership in expected, `contains` tests
     casefolded display() substring, ordering operators compare (strings
     lexically, numbers numerically; comparing a number with a string is an
     error).
3. Output rows `{path, …frontmatter}` in row order (stored values: dates are
   serialised by the MCP layer as `YYYY-MM-DD`, date-times as
   `YYYY-MM-DDTHH:MM:SS`).
4. `sort {by, dir}`: rows having a non-null, non-empty `by` value are sorted by
   it (descending only when `dir` is `desc`; stable), followed by the others in
   original order.
5. Return the first `limit` rows.

## 8. Schema info

- **sheet_schema()** → `{name: {folder, rows: count}}` for every registered
  sheet, plus key `invalid` = the invalid list when non-empty.
- **sheet_schema(name)** → `{sheet, folder, filename, key, identity, columns:
  {name: {type, required, values, max_length}}, instructions}` where
  instructions is the schema body (split as in §6 step 6, read strictly),
  trimmed.

## 9. Check

**check(vault)** over registered sheets (discovery order): for each row,
validate its frontmatter without standard columns with
`require_required = true` (failures → `{path, error}`); detect duplicates by
(tuple of key norm_str values, identity value or null): a later row with a
seen tuple adds `{paths: [first path, this path]}`. Report:

```json
{"sheets": {"roles": {"rows": 3, "invalid": [], "duplicates": []}}, "clean": true}
```

printed as indented File JSON with non-ASCII written literally (16 §3.5); `clean` is false when any sheet has
invalid rows or duplicates. The Librarian's `invalid_sheet_rows` is the total
number of invalid rows found the same way. CLI: 14 §2 (sheets).

## 10. Edge cases (intended)

1. **Uncalendared dates.** `2026-02-30` passes date validation; only the shape is
   checked.
2. A schema placed at the vault root registers folder `""`.
3. The Ambiguous-match message has no trailing full stop.

## 11. Acceptance map

SH-01…SH-38 → §§2–9; MA-10…MA-12 → §6; SV-20 → §§6–8.
