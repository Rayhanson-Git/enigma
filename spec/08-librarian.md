# 08 — Librarian (caretaker sweep)

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/librarian.yaml` (and LK-04/LK-05 in `locking.yaml`). Shared
conventions: 05 §0; formats: 16; error kinds: 00 §5. The CLI surface is in
14 §2 (librarian).

The Librarian is a thin orchestrator. One *sweep* runs, in order: index,
organize, cache, cleanup, consolidate, then health checks; an applying sweep
saves state and appends a report to `Claude/Librarian.md`. It owns no
indexing, filing or merging logic of its own.

---

## 1. Constants

| Name | Value |
|---|---|
| state file | `librarian_state.json` in the state directory (01 §4; format 16 §7) |
| `BACKSTOP_MIN_INTERVAL_DAYS` | 14 |
| `SLICE_SIZE` / `MAX_ENTITIES_PER_CALL` | 200 / 40 (07 §2) |
| `REPORT_MAX_SWEEPS` | 30 |
| `MAX_INDEX_ROUNDS` | 40 |
| report note | `Claude/Librarian.md` |
| report seed | `# Librarian` LF LF `Caretaker sweep reports (newest last). See constitution.` LF |
| auto-cleanup setting | `ENIGMA_AUTO_CLEANUP` (truthy set {`1`, `true`, `yes`, `on`} after trim + lower-case) |
| stamp format | second stamp `YYYY-MM-DD HH:MM:SS` (00 §3.5) |

## 2. State file

Default state (when the file is absent), key order as shown:

```json
{"last_sweep": null, "steps": {}, "health": {}, "errors": {}, "consolidation": {}}
```

Blocks that appear over time:

| Key | Content |
|---|---|
| `consolidation.checked_hash` | entity path → identity hash last adjudicated |
| `consolidation.cursor` | entity path where the backstop rotation stopped, or null |
| `consolidation.pending_proposals` | merges proposed by the last pass |
| `consolidation.last_pass` | second stamp |
| `consolidation.backstop_last_advance` | second stamp; absent until the first applying sweep |
| `cleanup.pending_retirements` | orphan records (12 §6) |
| `cleanup.last_pass` | second stamp |

New keys are appended after existing ones; `cleanup` is added after
`consolidation` the first time it is set.

- **Load**: absent → default; present → JSON parse; a parse failure is
  InvalidData, and the sweep (dry or applying) fails before any step runs.
- **Save**: atomically, in the format and through the temp file of 16 §7.
  A save interrupted before the replace leaves the previous file intact
  (LB-04).
- **status()** (MCP `librarian_status`): file absent →
  `{"status": "no sweep yet"}`; JSON parse failure →
  `{"status": "state file unreadable", "error": <decoder detail>, "path": <state file path>}`;
  otherwise the stored state, verbatim.

## 3. Steps

Each step runs isolated: if it fails with any error, `steps[name]` = null and
`errors[name]` = `{Kind}: {message}` (display name of 00 §5); the sweep continues.

### 3.1 index

- Apply: drain the indexer (06 §4, default arguments, extraction extractor
  unless one is injected). Run up to 40 rounds. Totals start as a copy of the
  first round's counts; later rounds add every count except `remaining` and
  `skipped`, which take the latest round's values. Stop after a round with
  remaining 0. If remaining is still > 0 after 40 rounds → Internal
  `index did not drain after 40 rounds; {remaining} notes still pending`.
- Dry run: `{"pending": n}` where n = number of pending notes per 06 §3
  (benched notes are not counted). Nothing is extracted or written.

### 3.2 organize

The organizer sweep (09 §3) with `apply` = the sweep's mode and `max_moves`
passed through. Its full report is the step value.

### 3.3 cache

Using the step values so far:
1. index value has processed > 0 → `{"rebuilt": true, "by": "index"}`;
2. else organize value has `cache_rebuilt` true → `{"rebuilt": true, "by": "organize"}`;
3. else graph.db missing: dry → `{"rebuilt": false, "by": "none"}`; apply →
   rebuild the cache, `{"rebuilt": true, "by": "librarian"}`;
4. else `{"rebuilt": false, "by": "none"}`.

### 3.4 cleanup

Let `auto` = the sweep's auto-cleanup flag. Uses 12-cleanup.

**Dry run** returns (key order):
`applied: false, auto_repair: auto, deleted_pending: len(deleted_notes),
orphans: len(find_orphans), retracted_notes: 0, removed_mentions: 0,
fixed_relations: 0, removed_relations: 0, flattened_stubs: 0, retired_stubs: 0,
pruned_cache_keys: 0, pending_retirements: 0, pending_repairs: <pending repairs>`.

*pending repairs* = `{deleted_notes: len(deleted), dangling_relations: fixed +
removed of a dry relation repair, stub_chains: flattened + retired_stubs of a dry
stub flatten, suspect_mass_deletion: suspect(deleted)}` (12 §§2–4).

**Apply**:
1. If auto: ret = retract_deleted (cap 100); rel = repair_relations (cap 200);
   stubs = flatten_stubs(now). Rebuild the cache when removed_mentions, fixed,
   removed, flattened or retired_stubs is non-zero. pending repairs = null.
   Otherwise: ret = `{retracted_notes: 0, removed_mentions: 0, remaining:
   len(deleted_notes), suspect_mass_deletion: false}`, rel = `{fixed: 0, removed: 0}`,
   stubs = `{flattened: 0, retired_stubs: 0}`, pending repairs computed.
2. orphans = find_orphans. The `cleanup` block of the state (created empty if
   absent) gets `update_retirement_proposals(block, orphans)` (cap 200) and
   `last_pass` = now; it is stored back into the state.
3. live = entity paths of gather_entities (07 §1). pruned =
   prune_entity_vectors(live) + prune_checked_hash(consolidation block, live)
   (12 §8; see Known quirks).
4. If pending repairs is null and ret reports suspect_mass_deletion, compute
   pending repairs now.
5. Return (key order): `applied: true, auto_repair: auto, pending_repairs,
   deleted_pending: ret.remaining, orphans: len(orphans), retracted_notes,
   removed_mentions, fixed_relations: rel.fixed, removed_relations: rel.removed,
   flattened_stubs, retired_stubs, pruned_cache_keys: pruned,
   pending_retirements: len(pending list)`.

Cleanup runs before consolidate so the slice never spends LLM calls on
entities this sweep just unsupported.

### 3.5 consolidate (incremental, never merges)

1. entities = gather_entities. None → `{ran: false, reason: "no entities",
   proposed: [], skipped_batches: 0}`.
2. block = state `consolidation` (an empty block, not yet attached to the state,
   when absent or empty); checked = copy of `checked_hash` (empty if absent);
   cursor = `cursor` (null if absent).
3. *backstop due*: marker `backstop_last_advance` absent or empty → false; else
   the whole days between the marker (parsed as a second stamp; a malformed
   marker fails the step) and now, rounded down, ≥ 14.
4. vectors = compute_entity_vectors(entities, persist = apply).
5. (slice, new cursor, used) = select_slice(entities, checked, cursor, 200,
   backstop due).
6. Empty slice: if apply and the block has no marker key, set marker = now and
   attach the block to the state. Return `{ran: false, reason: "nothing to
   check", proposed: [], skipped_batches: 0}`.
7. Otherwise clusters = candidate_clusters(slice, entities); batches; (proposed,
   skipped) = adjudicate_batches with the consolidation extractor (or the
   injected consolidator). reason = `backstop ({len(slice)})` when used, else
   `{len(slice)} unchecked`.
8. Apply only: checked[path] = identity hash for every slice entity; block
   gets `checked_hash` = checked, `cursor` = new cursor, `pending_proposals` =
   proposed, `last_pass` = now; when used or the block has no marker key,
   marker = now; attach the block to the state.
9. Return `{ran: true, reason, proposed, skipped_batches: skipped}`.

A batch with zero clusters makes no LLM call, so an unchanged vault costs
nothing (LB-15) and one edited entity costs one call (LB-16).

## 4. Health checks

Run after the steps, each isolated: a failing check's value becomes
`{"error": "{Kind}: {message}"}`. Keys in this order:

| Key | Value |
|---|---|
| `stale_embeddings` | number of notes whose vector is stale (04-search `stale_notes`; read-only), not counting the caretaker notes `Claude/Organizer.md` and `Claude/Librarian.md` (06 §3): the sweep itself writes them after the index step has embedded the vault, so they are always stale at this point and are never a reason to warn. A user note edited after it was embedded still counts. |
| `manifest_drift` | `{deleted_but_tracked: sorted(manifest hashes keys − scanned paths), present_but_untracked: sorted(scanned − hashes keys)}` (failures are not "tracked" here) |
| `orphaned_entities` | graph.db missing → `[]`; else distinct (entity_path, note_path) mention rows (first-occurrence row order) whose `{vault}/{note_path}.md` is not a file → `[{entity, missing_note}]` |
| `cache_consistency` | md = number of gathered entities; db missing → `{db_entities: null, md_entities: md, consistent: md == 0}`; else `{db_entities: rows in entities, md_entities: md, consistent: equal}` |
| `pending_proposals` | number of organize proposals (0 if the step failed) + (consolidate ran → its proposed count; else the stored `consolidation.pending_proposals` count) |
| `invalid_sheet_rows` | rows failing required validation over all discovered sheets (10 §9) |
| `pending_retirements` | length of `cleanup.pending_retirements` in the state |
| `sweep_errors` | copy of the errors map |

## 5. run_sweep(apply = true, max_moves = null, now, auto_cleanup = null)

1. now defaults to the current local time; auto_cleanup null → read
   `ENIGMA_AUTO_CLEANUP`.
2. Load the state. result = `{steps: {}, health: {}, errors: {}, applied: apply}`.
3. Run index, organize, cache, cleanup, consolidate (§3), then health (§4).
4. Apply only: state `last_sweep` = now (second stamp), `steps` = summary (§6),
   `health` = health, `errors` = copy of errors; save the state; append the
   report (§7) for `now`.
5. Return result. A dry run writes nothing to the vault or the state directory
   (vectors are computed in memory), even from cold caches.

The caller holds the caretaker lock for applying sweeps; dry runs do not take
it (LK-05). The CLI refuses an applying first sweep (14 §2).

## 6. Step summary stored in state

| Step | Stored |
|---|---|
| index | null, or the subset of `processed, failed, remaining, pending` present |
| organize | null, or `{moved: n, proposals: n, skipped: n}` (list lengths) |
| cache | the step value |
| cleanup | null, or the subset of `retracted_notes, fixed_relations, removed_relations, flattened_stubs, retired_stubs, pruned_cache_keys, pending_retirements` present, followed by `auto_repair` (absent → `false`) and `pending_repairs` (absent → null) |
| consolidate | null, or `{ran, reason, proposed: count, skipped_batches}` (`skipped_batches` absent → 0) |

A step value lacking a defaulted key is summarised with the default, never
reported as an error.

## 7. Report

### 7.1 Section format

Each line ends with LF; `{x}` are integers unless noted.

1. `## Sweep {now minute stamp}`
2. index: null → `- index: FAILED`; has `pending` → `- index: {pending} pending (dry-run)`;
   else `- index: processed {processed}, failed {failed}, remaining {remaining}`.
3. organize: `- organize: FAILED` or `- organize: moved {m}, proposals {p}, skipped {s}`.
4. cache: `- cache: FAILED`, `- cache: rebuilt ({by})`, or `- cache: fresh, no rebuild needed`.
5. cleanup:
   - null → `- cleanup: FAILED`;
   - dry → `- cleanup: {deleted_pending} deleted notes pending, {orphans} orphans (dry-run)`;
   - applied, not auto → this exact text (one line; the numbers come from
     pending repairs, 0 when absent; the command is in single backticks):

     ```text
     - cleanup: report-only — {deleted_notes} deleted notes, {dangling_relations} dangling relations, {stub_chains} stub chains await `cleanup --apply-repairs`; {pending_retirements} retirement proposals
     ```

   - applied, auto → `- cleanup: retracted {retracted_notes} notes, fixed
     {fixed_relations + removed_relations} relations, {pending_retirements}
     retirement proposals`.
6. If the cleanup value's pending repairs say `suspect_mass_deletion`: an extra
   line `- cleanup: ⚠ so many tracked notes are missing that the vault may be
   only partly present; retraction held`.
7. consolidate: `- consolidate: FAILED`; ran → `- consolidate: ran ({reason}) —
   {len(proposed)} merge proposals`; else `- consolidate: skipped ({reason})`.
   `{reason}` is the step value's `reason` text echoed verbatim; the report
   never interprets it (a sweep produces only the four reasons of §3.5, but
   the formatter accepts any text, which LB-24 relies on).
8. health (one line, segments joined by ` | `):
   `- health: stale_embeddings {a} {m} | manifest_drift {b} {m} |
   orphaned_entities {c} {m} | cache_consistency {m} | invalid_sheet_rows {d} {m}
   | pending_proposals {e} | pending_retirements {f}` where:
   a = stale_embeddings if an integer else −1; b = total of both drift lists,
   −1 if the value is not a drift mapping or holds an error; c = number of
   orphan records, −1 if not a list; d = invalid_sheet_rows if an integer else
   −1; mark {m} = `✓` when the number is 0, else `⚠`; for cache_consistency `✓`
   when the value is a mapping with `consistent` true; e and f are printed as
   stored (0 when absent; an error mapping prints in quoted()).
9. errors: `- errors: none`, or `- errors: ` + `{step}: {text}` pairs joined by `, `.

### 7.2 Appending to Claude/Librarian.md

Read the note (missing → the seed). text = text without trailing LFs + LF LF +
section. Split text at every occurrence of LF `## Sweep `: the first piece is
the header, the rest are sweeps; keep only the last 30 sweeps; rejoin as header
+ (LF `## Sweep ` + sweep) for each kept sweep; ensure a trailing LF; overwrite
the note (it is inside `Claude/`). The note is a caretaker note: the indexer
never scans it (06 §3).

## 8. Known quirks (kept on purpose)

1. **Consolidation-hash pruning no-op.** Cleanup prunes `checked_hash` on the
   state's existing `consolidation` block in place. When that block is absent
   or empty a throwaway block is pruned, nothing is stored and the count from
   it is 0.
2. The `pending_proposals` health count mixes this sweep's organize proposals
   with the last stored consolidation proposals when consolidate did not run.
3. Each drain round recomputes note embeddings (06 §4 step 9).
4. The dry-run report printed by the CLI is stamped with the time of printing,
   not the sweep's `now`.

## 9. Acceptance map

LB-01…LB-40 → §§1–7; LK-04/LK-05 → §5 and 14 §2.
