# 00 — Overview and shared conventions

Enigma MCP ("Enigma") is a Model Context Protocol server, plus a set of
command-line maintenance commands, that turns an Obsidian vault into shared,
persistent memory for a human and their AI agents. It is a single Rust binary
named `enigma-mcp`. Author: Harvey Rayhanson (Rayhanson-Git). Licence: MIT.

This file defines the vocabulary, the text and data conventions every other
spec file relies on, the error model, and the edge cases Enigma handles on
purpose. "See 00 §X" in another file means a section of this file.

## 1. Spec map

| File | Content |
|---|---|
| 00-overview.md | This file: glossary, conventions, error kinds, intended edge cases |
| 01-config.md | Environment variables, tool profiles, state directory, embedding model |
| 02-tools.md | The 25 MCP tools: parameters, return shapes, errors, profile membership |
| 03-vault-rules.md | Vault access rules, frontmatter, agent notes, tasks, locking |
| 04-search.md | Note scanning, BM25, vectors, Smart Connections reader, hybrid fusion |
| 05-graph.md | Graph store, SQLite cache, entity queries, related notes |
| 06-indexer-extractor.md | Incremental indexer and the LLM extractor |
| 07-consolidate.md | Duplicate-entity consolidation and blocking |
| 08-librarian.md | The scheduled caretaker sweep |
| 09-organizer-mover.md | Neighbour-vote filing, moves, undo |
| 10-sheets.md | Typed sheets (schema folders) |
| 11-recall.md | Digest / resume bundles and the context block |
| 12-cleanup.md | Deleted-note retraction, relation repair, retirements |
| 13-evals.md | Retrieval evaluation harness |
| 14-cli.md | Command-line subcommands |
| 15-sync-and-provision.md | mcp-sync, skill-sync, hook-sync, hooks, provisioning |
| 16-file-formats.md | On-disk formats and the exact YAML / JSON emission rules |
| acceptance/*.yaml | Normative behavioural cases |

Normative words: MUST, MUST NOT, SHOULD, MAY. Acceptance cases are normative
together with the prose. When prose and a case disagree, report it as a spec
defect; do not guess.

## 2. Glossary

| Term | Meaning |
|---|---|
| vault | A folder of Markdown notes managed by Obsidian. Its root is `ENIGMA_VAULT_PATH`, resolved to a canonical absolute path. |
| note | A file whose name ends in `.md` (case-sensitive on POSIX, case-insensitive on Windows) inside the vault. |
| rel path | A note's path relative to the vault root, components joined with `/` on every OS, including the `.md` suffix unless stated otherwise. |
| stem | The final component of a rel path without its trailing `.md` (the last three characters). |
| agent-owned folder | The subtree `Claude/` at the vault root. Agents may write there freely. The name is kept for compatibility with existing vaults. |
| human area | Everything outside `Claude/`. Agents may read it; writes need `confirm_outside_claude=true` (the "quarantine"). |
| agent name | The identity this server instance stamps on notes it writes (`ENIGMA_AGENT_NAME`, default `claude`). |
| state dir | A per-vault directory outside the vault holding caches, manifests, journals and the caretaker lock (01 §4). |
| caretaker | Any process that rewrites shared derived state: the Librarian sweep, the indexer, the organizer, consolidation, cleanup repairs. Caretakers serialise on the caretaker lock (03 §10). |
| graph | Entity notes under `Claude/Graph/` plus the derived SQLite cache `graph.db` (05). |
| sheet | A human-designated folder holding `_schema.md`; its direct-child notes are typed rows (10). |
| Smart Connections | An Obsidian plugin whose `.smart-env/` vectors Enigma reads when fresh (04 §6). |
| now | Local wall-clock time without zone, read once per operation unless stated. |

## 3. Text conventions

These rules apply everywhere unless a spec file says otherwise.

### 3.1 Encoding and line endings

- All note text is UTF-8. Text Enigma writes into notes is UTF-8 without a
  byte order mark, and its line endings are exactly those supplied. Enigma
  never converts LF to CRLF on any OS.
- **Text reads** (every read of a note as text unless the spec says "raw
  bytes") normalise line endings: each `\r\n` pair and each lone `\r` becomes
  `\n`. A leading UTF-8 byte order mark is kept as the character U+FEFF, so a
  note that starts with a BOM has no frontmatter.
- **Strict reads** (`read_note` and every read through the vault read
  operation of 03 §4) fail with `DecodeError` on bytes that are not UTF-8.
- **Lenient reads** (scans: search, tasks, `query_notes`, backlinks, graph
  rebuild and similar) silently drop byte sequences that are not valid UTF-8.
- Hashes that a spec defines over "raw bytes" use the bytes as stored, with
  no newline normalisation.

### 3.2 Lines

"Split into lines" is the universal line split: a line ends at `\n`, `\r\n`,
`\r`, U+000B, U+000C, U+001C, U+001D, U+001E, U+0085, U+2028 or U+2029. The
terminators are not part of the lines; a trailing terminator does not produce
an extra empty line; the empty string has no lines. "Split keeping ends" is
the same split with each line keeping its terminator.

### 3.3 Whitespace, case and ordering

- **Whitespace** is every character with the Unicode White_Space property,
  plus U+001C, U+001D, U+001E and U+001F.
- **strip** removes leading and trailing whitespace. "Strip X" with explicit
  characters removes only those characters.
- **collapse whitespace** splits on runs of whitespace, drops empty pieces and
  joins the pieces with one ASCII space (so it also strips).
- **lowercase** is the full Unicode lowercase mapping.
- **casefold** is full Unicode case folding (CaseFolding.txt statuses C and F,
  so `ß` folds to `ss`). "Case-insensitive" comparisons casefold both sides
  unless the spec says "lowercase".
- **sorted** without qualification means ascending by Unicode code point,
  character by character, a proper prefix first.
- **path order** (used by every directory walk the spec calls "sorted")
  compares two rel paths component by component, each component by code
  point; on Windows each component is lowercased first. Example: `a/x.md`
  sorts before `a b/x.md` because component `a` precedes `a b`.

### 3.4 display(v) and quoted(v)

Several messages and comparisons render values that came from YAML or JSON.
Spec files name which of these two renderings applies.

**display(v)**, the plain string form:

| Value | display(v) |
|---|---|
| string | the string itself |
| null | `None` |
| true / false | `True` / `False` |
| integer | decimal digits, with `-` if negative |
| float | the float text of 16 §3.4 (`1.0`, `0.5`, `1e-05`, `inf`, `nan`) |
| date (YAML timestamp without time) | `YYYY-MM-DD` |
| date-time (YAML timestamp with time) | `YYYY-MM-DD HH:MM:SS`, then `.ffffff` if the microseconds are non-zero, then `+HH:MM` or `-HH:MM` if it carries a UTC offset |
| list | `[` + items rendered with quoted() joined by `, ` + `]` |
| mapping | `{` + `quoted(k): quoted(v)` pairs joined by `, ` + `}` |

**quoted(v)**, the literal form used inside messages:

- For a string: the quote character is `'`, unless the string contains `'`
  and no `"`, in which case it is `"`. Emit the quote, the escaped content,
  the quote. Escaping: `\` becomes `\\`; when the quote is `'` every `'`
  becomes `\'`; tab, LF and CR become `\t`, `\n`, `\r`; any other
  non-printable character becomes `\xhh` (code point below 0x100), `\uhhhh`
  (below 0x10000) or `\Uhhhhhhhh` with lowercase hex. Non-printable means
  Unicode general category Cc, Cf, Cs, Co, Cn, Zl, Zp, or Zs other than the
  ASCII space. Printable non-ASCII characters are emitted unchanged.
- For a date: `datetime.date(Y, M, D)` with unpadded numbers, e.g.
  `datetime.date(2026, 7, 5)`.
- For a date-time: `datetime.datetime(Y, M, D, h, m`, then `, s` if the
  seconds or microseconds are non-zero, then `, us` if the microseconds are
  non-zero, then the zone part, then `)`. Numbers are unpadded. The zone part
  is empty for a value without offset, `, tzinfo=datetime.timezone.utc` for
  offset zero, `, tzinfo=datetime.timezone(datetime.timedelta(seconds=N))`
  for a positive offset of N seconds, and
  `, tzinfo=datetime.timezone(datetime.timedelta(days=-1, seconds=N))` with
  N = 86400 − |offset| for a negative one. Example:
  `datetime.datetime(2026, 7, 5, 10, 0)`.
- Any other value: quoted(v) = display(v), except that strings, dates and
  date-times nested inside lists and mappings use quoted().

Examples: quoted(`weekly`) is `'weekly'`; quoted(`it's`) is `"it's"`;
quoted(true) is `True`; display and quoted of the list `[a, 2]` are both
`['a', 2]`.

### 3.5 Timestamps

Local time, no zone suffix, every field zero-padded, 24-hour clock.

| Name | Pattern | Example |
|---|---|---|
| minute stamp | `YYYY-MM-DD HH:MM` | `2026-07-05 14:30` |
| second stamp | `YYYY-MM-DD HH:MM:SS` | `2026-07-05 14:30:09` |
| ISO second stamp | `YYYY-MM-DDTHH:MM:SS` | `2026-07-05T14:30:09` |
| day | `YYYY-MM-DD` | `2026-07-05` |
| clock | `HH:MM` | `14:30` |

### 3.6 JSON

Two JSON styles exist; spec files say which applies. Both are defined exactly
in 16 §3.

- **MCP JSON**: tool results (pretty, 2-space indent, raw UTF-8).
- **File JSON**: everything Enigma writes into the state dir or prints from
  CLI commands (non-ASCII escaped; compact with `, ` and `: ` separators, or
  indented by 2).

Object keys keep insertion order. Every spec table that lists keys lists them
in emission order.

## 4. Components (informative)

```
MCP client --stdio--> enigma-mcp (server)        enigma-mcp <subcommand> (CLI, 14)
                        | tools (02)               | index, librarian, organize, consolidate,
                        v                          | cleanup, sheets, recall, evals, sync, hook
                 vault access layer (03) <---------+
                        |
     +------------------+---------------------+----------------------+
     v                  v                     v                      v
 vault notes       search (04)          graph store (05)        state dir (16)
 (Markdown)        BM25 + vectors       Claude/Graph/*.md       graph.db, vector caches,
                   + RRF fusion         + graph.db cache        manifest, journals, lock
```

- The server speaks MCP over stdio; its MCP server name is `enigma`.
- The embedding model (01 §6) is loaded before the server starts serving.
- LLM work (entity extraction, duplicate adjudication) is delegated to an
  external CLI (`codex` or `claude`), never to a network API directly (06).
- Everything in the state dir is derived and can be rebuilt from the vault,
  except the organizer journal (needed for undo) and the Librarian state.

## 5. Error model

Every failure has exactly one **error kind** from the table below and a
**message**. A kind's display name is used wherever a spec renders
`"<Kind>: <message>"` (Librarian step errors, health-check errors, recall
section errors). On the MCP surface the client sees the message wrapped as in
02 §3; the kind name does not appear there.

Sub-kinds are instances of their parent: a rule that "catches VaultError"
also catches NoteExists, LockBusy and ProtectedLink. The display name is
always the most specific kind.

| Kind | Parent | Raised for |
|---|---|---|
| `VaultError` | — | invalid vault operations and configuration: missing `ENIGMA_VAULT_PATH`, missing vault root, path escapes, quarantine refusals, note not found, directory instead of note, graph cache missing, refused tool apply, recall argument errors, move and undo refusals |
| `NoteExists` | VaultError | a create-only write collided with an existing note |
| `LockBusy` | VaultError | the caretaker lock is held by another process |
| `ProtectedLink` | VaultError | a move would edit protected notes (09) |
| `SheetError` | — | sheet schema, validation, matching and query errors (10) |
| `ExtractorError` | — | extractor CLI failures and unparseable replies (06) |
| `InvalidArgument` | — | malformed argument values (bad due date, negative related-notes limit) and impossible calendar values met while parsing YAML timestamps |
| `DecodeError` | InvalidArgument | strict read of bytes that are not valid UTF-8 |
| `InvalidData` | InvalidArgument | a state-dir JSON file that does not parse where the spec makes parsing mandatory; malformed hook data that hook-sync refuses (15 §5) |
| `IoError` | — | any other operating-system failure (permission denied, disk full, a lock error still failing after the retry budget) |
| `NotFound` | IoError | a file the operation needs is absent at the OS level |
| `EvalConfigError` | — | golden-set problems in the eval harness (13) |
| `ProvisionError` | — | provisioning problems (15) |
| `SyncError` | — | mcp-sync / hook-sync configuration problems (15) |
| `Internal` | — | the Librarian's index step not draining within 40 rounds (08 §3.1, 06 §4); anything else (a bug) |

Message rules:

- A message that a spec file writes out literally MUST be produced byte for
  byte. Placeholders in braces are filled with display(v) unless the spec
  writes quoted(v).
- Messages of IoError, NotFound, InvalidData and DecodeError that no spec
  writes out are implementation-defined, MUST be one line, and SHOULD name the
  path involved.
- DecodeError messages SHOULD read
  `invalid UTF-8: byte 0x{hh} at offset {offset}: {reason}`,
  reason being `not a valid first byte`, `not a valid continuation byte` or
  `the text ends mid-character`.

## 6. Edge cases (intended)

Enigma keeps these behaviours on purpose. They are part of the
contract and tests may pin them. The file named specifies each in detail.

| # | Edge case | Where |
|---|---|---|
| Q1 | **Substring section-header match.** Locating the `## Mentions` or `## Relations` section searches for the header as a plain substring, so it also matches inside `### Mentions` or `## Mentions of x`; the section runs to the next `\n## ` after it. | 05 |
| Q2 | **Uncalendared sheet dates.** A sheet `date` column accepts any `NNNN-NN-NN` digit string, including impossible dates such as `2026-02-31`. | 10 |
| Q3 | **Consolidation-hash pruning no-op.** When the Librarian state has no `consolidation` block yet, cleanup prunes checked hashes inside a temporary empty map that is then discarded; the checked-hash part of the prune count is 0 and nothing is saved for it. | 12, 08 |
| Q4 | **Loose frontmatter fences.** The closing fence is the first `\n---` after the opening `---`, so `\n----` or `\n---x` also closes it, and a first line such as `---x` also opens it. | 03 §6 |
| Q5 | **Impossible YAML timestamps fail the operation.** A frontmatter scalar that matches the YAML timestamp pattern but is not a real calendar value (e.g. `2026-02-30`) makes the whole operation parsing it fail with InvalidArgument (`that day does not exist in that month`) instead of yielding `{}`. | 03 §6 |
| Q6 | **Null displays as None.** Comparisons through display() treat a null `project` as the string `None`, so `query_notes(project="None")` matches notes with an empty `project:`. | 03 §6, 02 |
| Q7 | **Limit edge cases.** Early-exit loops (`query_notes`) return one row for `limit` ≤ 0; slice-style limits (`search_brain`, `context_bundle`, `list_recent`, `sheet_query`) treat a negative limit as "drop that many from the end". Each tool in 02 says which applies. | 02 §5 |
| Q8 | **Whole-vault vector refresh on filtered search.** A search filtered by tags or folder still reads every vault note, embeds any stale one, and persists the fallback cache. | 04 §7 |
| Q9 | **Links with `.md` are not backlinks.** `[[Postgres.md]]` is not a backlink of `Postgres.md`; only links whose last component equals the stem match. | 02 §5.18 |
| Q10 | **Duplicate new aliases in one merge** are both added. | 05 |
| Q11 | **Directories named `*.md`.** A directory whose name ends in `.md` is yielded by note walks; reading it then fails the operation with IoError. The indexer is the exception: it records the error as a per-note failure (06 §4). | 04 §2 |
| Q12 | **String aliases split into characters.** When the extractor LLM returns `aliases` (or the consolidation LLM returns `duplicates`) as one string instead of a list, each character becomes an alias (duplicate). | 06 §5.2, 06 §6, 07 §6 |
| Q13 | **`merged_entities` counts proposals.** The consolidation result counts every proposed duplicate, including duplicates skipped during apply because the note is missing or already a stub. | 07 §6 |
| Q14 | **Stub relations protect from orphaning.** Relation lines inside stubs (`merged_into`) and tombstones (`retired`) still count as inbound support, so their targets are never proposed as orphans. | 12 §10 |
| Q15 | **Dry-run report stamped at print time.** The Librarian report printed by `--dry-run` carries the time of printing, not the sweep's `now`. | 08 §8, 14 §2 |

## 7. Deliberate design choices

Each of these is intended behaviour, not an accident:

- Names: MCP server name `enigma`, binary `enigma-mcp`, environment prefix
  `ENIGMA_`, state root `~/.enigma-mcp/`, nightly lock directory
  `enigma-index.lock`, launchd labels `com.enigma.*`, hook commands
  `enigma-mcp hook session-start` and `enigma-mcp hook stop-nudge`.
- No default vault path anywhere; a missing `ENIGMA_VAULT_PATH` is an error.
- Error kind names are those of §5, so `"<Kind>: <message>"` strings name
  those kinds.
- Texts that are normative byte for byte where they are written out and
  nowhere else: the server instructions (02 §2), the tool descriptions
  (02 §5), the extraction prompt (06 §5.3) and the consolidation prompt
  (07 §3.1).
- State-dir text files are written with LF line endings on every OS.
- Due dates accept ASCII digits only (03 §8).
- The concept lookup scans folder entries in path order, not in the OS
  listing order (03 §7.5); this matters only when two entries casefold to
  the same name.
- Invalid tool arguments produce the validation text of 02 §3.
- Consolidation walks candidate pairs in sorted order, so cluster order is
  deterministic (07 §2.4, 07 §6).
- `consolidate --apply` rebuilds the graph cache of the vault being
  consolidated, not the vault named by the environment (07 §5, 07 §6).
- Malformed LLM reply entries (merge elements or entity/relation items that
  are not objects) are skipped instead of aborting the run (06 §5.2,
  07 §3.2, 07 §6).
- Extractor stdout and stderr are decoded as UTF-8 with invalid bytes
  replaced by U+FFFD instead of failing (06 §5.5).
- Every bundled asset (constitution, vault guide, skills, MCP manifest,
  plugin lock, smart-env overlay) is compiled into the binary (15 §1), so a
  missing skills directory or manifest cannot happen.
- `hook-sync` with an unreadable or invalid `settings.json`, and `provision`
  (without `--check`) with a missing vault, print one line `error: …` to
  stderr and exit 2 (14 §2, 15 §5, 15 §7.5).
- No Windows scheduled-task file is shipped; only the macOS launchd
  templates are (15 §9.1).
- Note walks (search, tasks, queries, graph, organizer, Librarian) skip any
  file or folder whose resolved real path lies outside the vault root, so a
  symlink can never expose outside content through excerpts or task lists.
  Symlinks that resolve inside the vault are still walked. This matches the
  read/write path rule in 03 §1.

Further choices:

- **Uncaught errors.** An error that reaches the top of a command
  prints one line `error: {Kind}: {message}` (14 §1), never a stack trace.
- **Usage-error text.** Usage errors use clap's usage block and
  wording. Exit codes and every message written out in 14 are normative.
- **Invalid MCP arguments.** The validation text is that of 02 §3.
- **Transport and parser detail.** The text after
  `download failed {url}: ` (15 §7.2), after `cannot parse {path}: `
  (15 §6.2) and in the settings.json SyncError (15 §5) is
  implementation-defined (one line).
- **`evals --init-live` path.** `created: {path}` /
  `already exists: {path}` print the vault path as given, not with symbolic
  links resolved (13, 14 §2).
- **Nightly lock path text.** The lock is `enigma-index.lock`, joined to
  TMPDIR as a path, so a trailing slash in TMPDIR never doubles (15 §9).
- **Bundled and rendered texts.** Server instructions, onboard
  placeholders (`… run enigma-mcp install-conventions VAULT`), the
  constitution, `CLAUDE.md` and `AGENTS.md` guides, the nine skills, the
  `Claude/Evals.md` seed, the mcp-servers `why` texts, provision's last step
  (`(enigma-mcp mcp-sync)`), the
  `macOS privacy permission for enigma-mcp/launchd?` line and the hook
  commands are normative as bundled.
- **D19, MCP protocol details.** `initialize` advertises only the `tools`
  capability with `serverInfo` `enigma` / the package version; input schemas
  carry no `title` keys; an unknown method is JSON-RPC `-32601`
  (method not found) naming the method; a missing embedding model is the
  one line of 01 §8.
- **Eval corpus default.** `evals` with no `--vault` uses the bundled spec
  fixtures (13 §1).
