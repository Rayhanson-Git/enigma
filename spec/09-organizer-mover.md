# 09 — Organizer and link-safe mover

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/organizer.yaml`, `acceptance/mover.yaml`. Formats and error kinds:
05 §0. MCP tools `organize_vault` / `undo_move` (02-tools) and the CLI
`organize` (14-cli) use this file.

The organizer files notes into the human's existing top-level folders by a
nearest-neighbour vote over note embeddings. Every move goes through the
mover, which rewrites path-qualified links so nothing breaks, is journalled,
and can be undone.

---

## 1. Constants and scope

| Name | Value |
|---|---|
| `EXCLUDED_DIRS` | `Claude`, `00 - Maps of Content`, `.obsidian`, `.smart-env`, `.trash`, `.space`, `copilot` |
| `EXCLUDED_ROOT_FILES` | `CLAUDE.md`, `AGENTS.md`, `README.md` |
| `VOTE_K` | 10 |
| `VOTE_THRESHOLD` | 0.7 |
| move log note | `Claude/Organizer.md` |
| note seed | `# Organizer` LF LF `Autonomous move log and proposals. See constitution → Organizer.` LF LF `## Log` LF |
| journal | `organizer_journal.jsonl` in the state directory |
| suggestions state | `organizer_suggestions.json` in the state directory (16 §8.1) |
| `SUGGESTION_TTL_DAYS` | 15 |
| block markers | `<!-- enigma:suggestions:start -->` and `<!-- enigma:suggestions:end -->` |

A *sheet folder* is a folder containing a file `_schema.md` (10 §2). A path is
*in a sheet folder* when it has at least two segments and its parent folder is
a sheet folder.

- **Taxonomy**: names of the vault root's immediate sub-directories that are
  not in EXCLUDED_DIRS, do not start with `.` and are not sheet folders,
  sorted ascending.
- **Organized notes** (the labelled set): for each taxonomy folder in order,
  every `*.md` file below it at any depth (hidden sub-folders included), sorted
  by path; vault-relative with `/`.
- **Candidates**: the root's `*.md` files (extension exactly `.md`, not in
  EXCLUDED_ROOT_FILES) sorted by name, followed by the organized notes; minus
  notes whose frontmatter `organize` is the boolean false (YAML `false`, `no`,
  `off` …; the note is read strictly) and minus notes in a sheet folder.
  Nothing under `Claude/`, `00 - Maps of Content` or a dot folder is a
  candidate.

## 2. Classification

**classify(path, vectors, labelled)** → (folder or none, share, neighbours):

1. The path has no vector → (none, 0.0, []).
2. Score every labelled path other than itself that has a vector by cosine
   (04-search); keep scores > 0. None left → (none, 0.0, []).
3. Sort by score descending (stable, so ties keep labelled order); take the
   first 10.
4. Votes: add each kept score to the vote of its path's first segment
   (folders in first-seen order). Winner = the first folder with the maximum
   vote. share = winner's vote / total of all votes. neighbours = the kept
   paths in order.

## 3. Sweep: run(vault, embedder, apply = false, max_moves = null)

1. vectors = note vectors for the whole vault (04-search `get_note_vectors`,
   persisted only when `apply`).
2. Compute taxonomy, labelled = organized notes, candidates.
3. For each candidate *c* (in order), with *current* = its first segment when
   `c` contains `/` and that segment is in the taxonomy, else none, and
   (folder, share, neighbours) = classify(c):

| # | Condition | Outcome |
|---|---|---|
| 1 | folder is none | skipped, reason `no vector or no labeled neighbors` |
| 2 | folder = current | skipped, `already correctly filed` |
| 3 | share < 0.7 and current is none | proposal, reason `low confidence` |
| 4 | share < 0.7 and current is a folder | skipped, `low-confidence disagreement` |
| 5 | another note anywhere in the vault has the same stem (§4.2) | proposal, `duplicate stem — bare links would become ambiguous` |
| 6 | max_moves given and moves so far ≥ max_moves | skipped, `move cap reached` |
| 7 | target = `{folder}/{file name of c}`; dry run and the move would rewrite links inside notes in sheet folders | proposal, `linked from sheet rows: ` + first 3 such paths joined by `, ` |
| 8 | apply: move with sheet-folder notes protected (§4.3); ProtectedLink | proposal, same reason built from the error's paths |
| 9 | otherwise (apply: moved and journalled, §5) | moved |

   Record shapes (key order): moved `{from, to_folder, share, neighbors}`;
   proposal `{path, suggested, share, neighbors, reason}`; skipped `{path, reason}`.
   `share` in records is rounded to 3 decimals (half-to-even on the exact
   binary value); `neighbors` is the first 3 neighbours. A note in a nested
   sub-folder moves to the top of the target folder (only its file name is
   kept).
4. Apply and at least one move → rebuild the graph cache; `cache_rebuilt` = true
   (else false).
5. Apply → update the suggestions block (§3.1).
6. Return `{moved, proposals, skipped, cache_rebuilt}`.

A dry run moves nothing and writes no journal, no vectors, no suggestions
state and nothing in the move log note.

**Acknowledged suggestions.** Before step 3 the run reads the acknowledged
pairs: those stored in the suggestions state plus every ticked line of the
current block (§3.1). A candidate whose outcome would be a proposal of an
acknowledged (path, suggested folder) pair is left out of `proposals` and
reported in `skipped` (after the other skipped records, in proposal order)
with reason `suggestion acknowledged`, in both modes, and is never proposed
again. Acknowledging never moves a note.

### 3.1 Suggestions block (one sticky note)

`Claude/Organizer.md` holds one block of current suggestions, delimited by the
two block markers, in place of the earlier `### Proposals {day}` sections
(which stay in old notes as written). Text outside the block, including
anything the owner writes there and the move log, is preserved byte for byte;
text inside the block is regenerated.

The **suggestions state** is `{"open": [{path, suggested, reason, proposed}],
"acknowledged": [{path, suggested, at}]}` (`proposed` a day stamp, `at` a
second stamp; absent file → both empty; a file that does not parse fails the
run with InvalidData before anything moves).

In an applying run, after the moves, with P = this run's proposals:

1. **Ticks.** Every line inside the block matching
   ``- [x] `{path}` → **{suggested}**`` (`x` or `X`) is acknowledged: the pair
   joins `acknowledged` with `at` = now, leaves `open`, and the history line
   `suggestion acknowledged` is logged.
2. **Existing items** of `open`, in order:
   - not in P and `{path}` is not a file → dropped, history
     `suggestion dropped (note moved or deleted)`;
   - not in P otherwise → dropped, history `suggestion dropped (no longer applies)`;
   - in P and whole days from `proposed` to now ≥ `SUGGESTION_TTL_DAYS` (or
     `proposed` does not parse) → dropped, history
     `suggestion expired after 15 days`; the pair is not added again in this
     run, and a later run proposes it afresh (new date) if it still applies;
   - otherwise kept in place, with `reason` updated from P.
3. **New items**: each pair of P not kept and not expired in this run is
   appended to `open` with `proposed` = today's day stamp.
4. **Render** the block (LF line ends):

   ```text
   <!-- enigma:suggestions:start -->
   ## Suggestions

   Tick a box to dismiss a suggestion; it will not be proposed again. Enigma never moves a note because of this list.

   - [ ] `{path}` → **{suggested}** ({reason}) — proposed {proposed}
   <!-- enigma:suggestions:end -->
   ```

   one item line per `open` entry in order, or the single line
   `_No open suggestions._` when `open` is empty.
5. **Place** it: the note's current block (from the first start marker to the
   first end marker after it) is replaced. Without a block, nothing is written
   when `open` is empty and no history line was produced; otherwise the move
   log note is made to exist (seed, §1) and the block plus an empty line is
   inserted before the first line that is exactly `## Log`, or appended at the
   end (after an empty line) when there is no such line.
6. **History**: each history line is appended at the end of the note (the move
   log), never inside the block:

   ```text
   - {second stamp} — {history}: `{path}` → **{suggested}**
   ```

7. The note is written (overwrite) only when its text changed, and the state
   only when it changed: an unchanged set of proposals with unchanged reasons
   leaves both byte-identical.

## 4. Mover

### 4.1 Link pattern

For a vault-relative path P, let P′ = P without a trailing `.md`. A *link to P*
is the text `[[` + P′ (matched literally, case-sensitive) immediately followed
by `]`, `|` or `#` (not consumed). Rewriting a move P → Q replaces each
non-overlapping match's `[[` + P′ with `[[` + Q′. So `[[Signal]]`,
`[[Signal|alias]]` and `[[Signal#Part]]` are rewritten, `[[Signal 2]]` is not,
and bare-stem links keep working because stems never change.

Files are handled as bytes: decode UTF-8 keeping undecodable bytes as they are
and re-encode them unchanged; CRLF and every other byte outside the link text
is preserved exactly.

### 4.2 duplicate_stem_exists(path)

True when more than one note in the vault (04-search iteration) has a stem
(file name minus its last extension) whose casefold equals the casefolded
stem of `path`.

### 4.3 move_note(old, new, protect = none) → record

1. Resolve both (escape errors per 03-vault-rules). `old` not a file →
   VaultError `Cannot move: not a file: {old}`. `new` exists → VaultError
   `Cannot move: destination exists: {new}`.
2. *Plan*: for every note in vault-wide iteration order, compute the rewritten
   bytes and the match count; keep notes with count > 0. Nothing is written.
3. If `protect` is given and accepts any planned path → ProtectedLink
   `move would edit protected notes: {paths joined by ", "}` (plan order); the
   error carries the list. Nothing has been touched.
4. Move the file: create the destination's parent folders, atomic rename
   (with the lock-retry policy of 03-vault-rules).
5. Write each plan in order; a plan for the moved note itself is written at
   its new location. Each write goes to `{file name}.tmp-move` in the same
   folder, then an atomic replace (lock-retry).
6. If anything in step 5 fails: restore every file already written to its
   original bytes (newest first, ignoring errors), move the note back
   (ignoring errors), and re-raise. The vault ends byte-identical.
7. `rename_manifest_entry(old, new)` (06 §2).
8. Return `{from: old, to: new, rewrites: [{path, count}]}` with each plan's
   original path.

### 4.4 reverse_rewrites(old, new, paths)

For each listed path that is still a file: rewrite links to `new` into links to
`old` (same pattern rules) and write atomically when there was at least one
match.

## 5. Journal, log and undo

**record_move(record, share, neighbours)**, after a successful move:
1. Append one compact File JSON line to the journal: `{ts, from, to,
   share, neighbors, rewrites, undone: false}` — ts is the current second
   stamp, share the unrounded float, neighbours the first 3.
2. Make sure the move log note exists (seed if reading fails), then append
   this line (exact text, then LF):

   ```text
   - {ts} — moved [[{to without .md}|{stem of to}]] from `{from}` (share {share 2-decimal})
   ```

**undo_move(path)** → `{restored: from, was: to}`:
1. Read the journal (absent → no entries; each non-blank line is JSON). Find
   the latest entry whose `to` equals `path` exactly and whose `undone` is not
   true. None → VaultError `No undoable move found for: {path}`.
2. `to` is not a file → VaultError `Cannot undo: file no longer at {to}`;
   `from` exists → VaultError `Cannot undo: original location occupied: {from}`.
3. Move the file back (parents created, lock-retry); reverse_rewrites(from,
   to, the entry's rewrite paths); rename_manifest_entry(to, from) (a failure
   record moves back too).
4. Mark the entry undone and rewrite the whole journal (one compact line per
   entry, LF-terminated).
5. Make sure the move log note exists and append

   ```text
   - {second stamp} — UNDID move of `{to}` back to `{from}`
   ```

   (then LF).

## 6. Locking

Applying sweeps run under the caretaker lock (taken by the CLI `organize`
without `--dry-run`, by `organize_vault(apply = true)` and by the Librarian).
Dry runs and `undo_move` do not take it.

## 7. Known quirks (kept on purpose)

1. Only path-qualified links are rewritten; bare `[[Stem]]` links are relied on
   to stay unique through the duplicate-stem guard.
2. Organized notes include files inside hidden sub-folders of a taxonomy
   folder, but those have no vector and are skipped.
3. A candidate with undecodable bytes fails the frontmatter read and fails the
   whole sweep step.

(The earlier `### Proposals {day}` section appended on every applying run
was replaced after v1 by the suggestions block of §3.1.)

## 8. Acceptance map

OR-01…OR-24 → §§1–3, 5 (OR-17…OR-24: §3.1); MV-01…MV-10 → §4.
