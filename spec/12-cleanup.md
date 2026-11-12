# 12 — Graph deletion and orphan cleanup

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/cleanup.yaml` (plus LB-33…LB-38 for the Librarian step). Formats
and error kinds: 05 §0; entity-note terms: 05-graph §2.

Cleanup makes the graph converge on facts that still exist: it retracts
mentions of deleted notes, repairs relations that point at stubs or dead
entities, flattens stub chains, proposes retiring unsupported entities and
prunes consolidation caches. The Librarian runs it report-only unless
auto-repair is on (08 §3.4); retirement is always propose-only and applied by
hand.

---

## 1. Constants

| Name | Value |
|---|---|
| `MAX_RETRACTIONS_PER_SWEEP` | 100 |
| `MAX_RELATION_FIXES_PER_SWEEP` | 200 |
| `MAX_PENDING_RETIREMENTS` | 200 |
| `MASS_DELETION_MIN_NOTES` | 20 |
| `MASS_DELETION_MAX_FRACTION` | 0.10 |
| retirement reason (orphans) | `orphaned — no mentions or relations` |
| retirement reason (dead stubs) | `merge redirect target gone` |
| orphan reason text | `orphaned: no mentions or relations` |

## 2. Deleted notes and the mass-deletion guard

- **tracked** = keys of manifest `hashes` ∪ keys of `failures` (06 §2).
- **deleted_notes** = tracked minus the paths of scan_notes (06 §3), sorted.
  Organizer moves never show up here because the mover renames manifest
  entries.
- **suspect(deleted)** = deleted count ≥ 20 **and** deleted count > 0.10 ×
  tracked count. (25 of 100 → true; 25 of 1000 → false; 19 of 19 → false.)

## 3. Retraction: retract_deleted(limit = 100, force = false)

1. deleted = deleted_notes. If not `force` and suspect → change nothing and
   return `{retracted_notes: 0, removed_mentions: 0, remaining: len(deleted),
   suspect_mass_deletion: true}`.
2. todo = first `limit` of deleted. For each: find the *mentioning entities* —
   with graph.db present, `note_entity_paths(db, note)` (05 §3.3); without it,
   every `*.md` under `Claude/Graph/` (sorted, read leniently) whose whole text
   contains `[[{note without .md}|`, as entity paths, sorted. For each,
   `remove_mention(entity path + ".md", note)` (05 §2.8), counting true
   results; a VaultError (entity note gone) is ignored. Then drop the note
   from both manifest maps.
3. Save the manifest if todo is non-empty.
4. Return `{retracted_notes: len(todo), removed_mentions, remaining:
   len(deleted) − len(todo)}` (no `suspect_mass_deletion` key).

## 4. Relations and stubs

**target status(path)** for an entity path: `path.md` escapes the vault or is
not a file → *gone*; read leniently; `retired` truthy → *gone*; `merged_into`
truthy → *stub* with canonical = resolve_redirect(path) (05 §2.4, may be
none); otherwise *live*.

### 4.1 repair_relations(limit = 200, dry = false) → {fixed, removed}

No `Claude/Graph/` folder → zeros. Otherwise for each `*.md` below it (sorted,
read leniently), stopping before a file once fixed + removed ≥ limit; skip
stubs and tombstones. Walk the file's lines (split keeping ends (00 §3.2)); a
line is examined only when its trimmed form matches the relation regex
(05 §2.3) and the limit is not yet reached; target statuses are memoised for
the pass.

| Target status | Effect |
|---|---|
| live | line kept |
| stub with a canonical C | new line = `- {relation} [[{C}\|{last segment of C}]]` (one `|` between C and the segment) LF. If that exact text (with LF) is already among the lines written so far or among the file's original lines → the old line is dropped and *removed* += 1; else the new line replaces it and *fixed* += 1 |
| stub without canonical, or gone | line dropped, *removed* += 1 |

A changed file is overwritten (not when `dry`). Lines are never touched outside
the relation grammar, and every other byte is kept.

### 4.2 flatten_stubs(now, dry = false) → {flattened, retired_stubs}

No graph folder → zeros. For each `*.md` (sorted, read leniently) that is a
stub and not a tombstone: target = display() of `merged_into`; if target status is
live → skip. Else final = resolve_redirect(target).
- dry → count *flattened* when final exists, else *retired_stubs*.
- final exists → set `merged_into` = final in the parsed frontmatter and
  overwrite: `---` LF + YAML dump (flow-leaf style) + `---` LF LF + `# {file stem}`
  LF LF `Merged into [[{last segment of final}]].` LF.
- otherwise retire the stub (§5) with reason `merge redirect target gone`.

A → B → C becomes A → C; a stub whose chain ends at a tombstone, a missing
note or a cycle is retired.

## 5. retire_note(rel, now, reason)

Read strictly; parse frontmatter; set `retired` = now minute stamp (a new key
goes last); summary = entity summary (05 §2.3) of the old text; stem = file
name without `.md`. Overwrite with `---` LF + YAML dump (flow-leaf style) +
`---` LF LF + `# {stem}` LF LF + (summary LF LF, if non-empty) +
`Retired: {reason}.` LF. Aliases stay in the frontmatter and the summary stays
in the body so the entity can be revived (05 §2.7 step 2).

## 6. Orphans and retirement proposals

**find_orphans** (markdown is the source of truth, not graph.db): no graph
folder → `[]`. One pass over `*.md` below `Claude/Graph/` (sorted, read
leniently):
1. For every note — stubs and tombstones included — each relation section
   line (05 §2.3) whose trimmed form matches the relation regex adds its
   target to the *inbound* set.
2. Live notes become candidates `{path: entity path, name: file stem, type:
   display() of entity or "topic", supported}` where supported = the Mentions
   section has any line starting `- ` or the Relations section has any.
Result: candidates not supported and not in inbound, as `{path, name, type,
reason: "orphaned: no mentions or relations"}`. An entity created only as a
relation endpoint is supported by its inbound edge.

**update_retirement_proposals(block, orphans, limit = 200)**: pending = the
block's `pending_retirements` (absent → []) filtered to paths still in
orphans; then append each orphan whose path is not already pending, stopping
once pending holds `limit` entries; store it in the block and return it.

**apply_retirements(paths = null, now)**: recompute find_orphans (stored
proposals are never trusted); with `paths` given keep only those paths (an
empty list retires nothing); retire each with reason
`orphaned — no mentions or relations`; rebuild the cache if any were retired;
return `{retired: sorted paths}`.

## 7. apply_repairs(now, force = false)

ret = retract_deleted(force); rel = repair_relations(); stubs =
flatten_stubs(now). Rebuild the cache when any of removed_mentions,
retracted_notes, fixed, removed, flattened, retired_stubs is non-zero. Return
`{retract: ret, relations: rel, stubs}`.

## 8. Cache pruning

- prune_entity_vectors(live) — 07 §2.2.
- **prune_checked_hash(block, live)**: take the block's `checked_hash` map (a
  throwaway empty map when absent or empty), delete keys not in `live` in
  place, store it back when anything was deleted, return the count. See 08
  Known quirks for the absent-block case.

## 9. CLI (flags in 14 §2, cleanup)

| Invocation | Lock | Output (indented File JSON) |
|---|---|---|
| `cleanup VAULT` | none | `{deleted_notes, orphans}` |
| `cleanup VAULT --apply-repairs [--force]` | caretaker | apply_repairs result |
| `cleanup VAULT --apply-retirements [--paths P …]` | caretaker | apply_retirements result |

`--apply-repairs` wins when both apply flags are given.

## 10. Known quirks (kept on purpose)

1. Relation lines in stubs and tombstones still count as inbound support in
   find_orphans.
2. Graph notes are read with line endings normalised (00 §3.1), so a
   repaired note is rewritten with LF line ends throughout.
3. The markdown fallback for mentioning entities matches the marker anywhere
   in an entity note, not only in its Mentions section.
4. Consolidation-hash pruning is a no-op when the state has no consolidation
   block (00 Q3, 08 §8).

## 11. Acceptance map

CL-01…CL-17 → §§2–8; LB-33…LB-38 → 08 §3.4 using this file.
