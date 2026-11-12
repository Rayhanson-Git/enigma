# 07 — Entity consolidation and candidate blocking

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/consolidate.yaml`, `acceptance/blocking.yaml`. Shared conventions:
05 §0; formats: 16; error kinds: 00 §5; entity-note terms: 05 §2.

Consolidation finds entity notes that name the same real-world thing and, on
apply, folds duplicates into one canonical note, leaving *stubs* that redirect
to it. Work is bounded: an embedding pass groups likely duplicates (*blocking*)
and only those small groups are shown to an LLM.

---

## 1. Entities

**gather_entities(vault)**: every `*.md` under `Claude/Graph/` (recursive, path
order; none if the folder is absent), read leniently, skipping stubs and
tombstones. Each yields, in this key order:

| Key | Value |
|---|---|
| `name` | the file stem (not the H1) |
| `type` | display() of frontmatter `entity`, or `topic` when falsy |
| `path` | entity path (vault-relative, no `.md`) |
| `aliases` | `str_list(aliases)` |
| `summary` | entity summary (05 §2.3) |

## 2. Blocking

Constants: `SIM_THRESHOLD` = 0.85, `K_NEIGHBORS` = 5, `MAX_CLUSTER` = 10,
`SLICE_SIZE` = 200, `MAX_ENTITIES_PER_CALL` = 40, cache file
`entity_vectors.json` in the state directory.

### 2.1 Identity

- *identity text* = `{name}` LF `{aliases joined by ", "}` LF `{summary}`, then
  trimmed of surrounding whitespace. Example: Cedar Cloud / [CC] / "Cloud VM." →
  `Cedar Cloud\nCC\nCloud VM.`
- *identity hash* = lowercase hex SHA-256 of the UTF-8 identity text. The path
  is not part of it.

### 2.2 Entity vector cache

`entity_vectors.json` maps entity path → `{"hash": identity hash, "vec": [floats]}`.
It is loaded and saved exactly like the note fallback cache of 04-search
(missing or corrupt file → empty map; memo by modification time and size;
atomic save through a temp file in the same directory; compact File JSON).

**compute_entity_vectors(entities, embedder, persist = true)** → map path →
vector. For each entity in order: a cache entry with the same hash is reused;
others are collected. All collected identity texts are embedded in **one**
batch call (in collection order) and stored into the cache map. When
something was embedded and `persist` is true, save the cache. `persist = false`
(dry runs) never writes the file.

**prune_entity_vectors(live paths)** → number of cache keys not in `live paths`;
removes them and saves when that number is > 0.

The embedder is the one defined in 04-search: model `TaylorAI/bge-micro-v2`
(BERT, 3 layers, hidden size 384), uncased WordPiece tokenisation (lower-case,
accents stripped, CJK characters split), at most 512 tokens including
`[CLS]`/`[SEP]` with truncation on the right, **mean pooling** over the
attention mask, **no** L2 normalisation (the model's `modules.json` lists only
a Transformer and a Pooling module; `1_Pooling/config.json` sets
`pooling_mode_mean_tokens: true`; `sentence_bert_config.json` sets
`max_seq_length: 512`). Cosine similarity is 04-search's (zero norm → 0).

### 2.3 Candidate pairs

Inputs: a *slice* of entities, all entities, and the vector map. For each
slice entity with a vector, compare it with every other entity of the same
type (from all entities, excluding itself) that has a vector; keep those with
cosine ≥ 0.85; order them by (cosine, path) descending; take the first 5. Each
kept neighbour forms the unordered pair {slice path, neighbour path}, stored as
(smaller path, larger path). The result is a set of pairs.

### 2.4 Clusters

1. Walk the pairs in ascending order and union their two paths (disjoint-set
   union).
2. Each connected component becomes a group; members are sorted ascending.
   Groups are ordered by their smallest member.
3. A group of at most 10 members is one cluster. A larger group of n members
   is split into k = ceil(n / 10) consecutive chunks of the sorted members;
   with base = n div k and extra = n mod k, the first `extra` chunks have
   base + 1 members, the rest base members. (A star of 11 → chunks of 6 then 5,
   the multiset {5, 6} of BL-06; no member is lost.)
4. **candidate_clusters** maps each cluster back to the entity records and
   drops clusters of fewer than 2 members.

### 2.5 Batches

**batch_clusters**: walk clusters in order, appending each whole cluster to
the current batch; before appending, if the batch is non-empty and its entity
count plus the cluster size would exceed 40, close the batch and start a new
one. Clusters are never split. (3, 4, 3 under a cap of 8 → batches of 7 and 3.)

### 2.6 Slice selection

**select_slice(entities, checked_hash, cursor, slice_size, backstop_due)** →
(slice, new cursor, used_backstop):

1. Sort entities by path (code-point order).
2. slice = entities whose `checked_hash[path]` differs from their identity hash
   (absent counts as different), in path order, at most `slice_size`.
3. new cursor = cursor; used_backstop = false.
4. If `backstop_due` and the slice is short: build the rotation = the sorted
   entities starting at the first path strictly greater than `cursor`, then
   wrapping to the start (no cursor → plain sorted order). Walk it, appending
   entities not already chosen until the slice is full; each appended entity
   sets new cursor to its path and used_backstop to true.

Examples: all checked, cursor `b`, size 2, paths a–d → [c, d], cursor d, used;
next call with cursor d → [a, b], cursor b. Backstop not due → no rotation.

---

## 3. Adjudication

### 3.1 Prompt

One LLM call per batch through the consolidation extractor's `complete_json`
(06 §5.7). The listing has one line per entity of the batch (clusters
flattened in order):

`{type} | {name} | {aliases joined by ", ", or "-" when there are none}`

The prompt below is **normative byte for byte**:
LF line ends, `{listing}` replaced by the lines joined
with LF, no trailing LF (CO-11 checks the whole text):

```text
Some lines in this knowledge-graph entity list may be duplicate entries: two or more names for one real-world thing.
Each line reads: type | name | aliases.

Find the sets of names that refer to the same thing and say which name should survive.
- Group only entries of the same type.
- Group only different spellings or forms of one name, such as a short form and its longer form. Things that are related but distinct stay separate.
- The surviving (canonical) name is the fullest, most widely used form. It and every duplicate must be names taken from the list.
- When you are not sure, leave the entries alone.

Output a single JSON object and nothing else:
{"merges": [{"type": "...", "canonical": "...", "duplicates": ["..."]}]}
When nothing should be merged, output {"merges": []}.

Entities:
{listing}
```

### 3.2 Validating merges

*Known* = {(type, casefolded name)} over **all** gathered entities (names are
file stems; aliases do not count). For each element of the reply's `merges`
(falsy → none; non-object elements are ignored):

1. type = display() of `type` (falsy → empty), lower-cased, not trimmed.
2. canonical = display() of `canonical` (falsy → empty), trimmed.
3. duplicates = each element of `duplicates` (falsy → none; a string is
   iterated by character) as display(), trimmed, empties dropped, and those whose
   casefold equals the canonical's casefold dropped.
4. Keep the merge only if canonical and duplicates are non-empty, type is one
   of the six entity types, (type, canonical casefold) is known and every
   (type, duplicate casefold) is known. Result `{type, canonical, duplicates}`.

### 3.3 adjudicate_batches(backend, batches, all entities) → (merges, skipped)

For each batch in order: call the backend; **any** failure of that call
(including parse failures) increments `skipped` and moves on. Valid merges are
appended unless an identical one was already kept, identity being (type,
casefolded canonical, sorted casefolded duplicates).

---

## 4. Applying a merge

For merge (type, canonical, duplicates), with one timestamp `now` for the run:

1. canon = `find_entity_note(type, canonical)` (05 §2.5).
2. For each duplicate name *d*:
   - *dup note*: if the entity rel path for (type, d) exists, read it
     strictly; it is the dup note unless it is a stub or tombstone (then
     none). Otherwise scan the type folder (non-recursive, file-name order)
     for the first stem whose casefold equals d's casefold — again none if
     that note is a stub or tombstone; none if there is no such file. Aliases
     are not consulted.
   - Skip d when the dup note is none, equals canon, or canon is none.
   - For each section line (05 §2.3) of the dup note's `## Mentions`: insert it
     into canon's Mentions with marker = the line up to its first `|` plus `|`
     (the whole line plus `|` when it has none).
   - For each section line of `## Relations`: insert it into canon's Relations
     with the whole line as marker.
   - `merge_aliases(canon, [d] + dup note's str_list(aliases))`.
   - Overwrite the dup note with a stub: `---` LF + YAML dump (block style) of
     `{created: now minute stamp, agent: claude, entity: type, merged_into: canon
     entity path, tags: [graph/{type}]}` + `---` LF LF + `# {d}` LF LF
     `Merged into [[{canon stem}]].` LF.

## 5. run(vault, backend, apply = false, embedder) → result

1. entities = gather. result = `{entities: n, proposed: [], applied: false,
   merged_entities: 0, skipped_batches: 0}` (this key order). No entities →
   return it.
2. vectors = compute_entity_vectors(entities, persist = apply).
3. clusters = candidate_clusters(slice = all entities, all entities);
   batches = batch_clusters(clusters); (merges, skipped) = adjudicate.
4. proposed = merges; skipped_batches = skipped.
5. If `apply` and merges exist: apply each merge in order; merged_entities +=
   number of proposed duplicates of each merge; applied = true; rebuild the
   graph cache in **this vault's** state directory.

Used by the MCP tool `consolidate_graph(apply = false)` (02-tools, lock and
`ENIGMA_NO_TOOL_APPLY` rules there) and the CLI `consolidate` (14-cli). The
Librarian uses blocking and adjudication incrementally (08 §3.5) and never
applies merges.

## 6. Edge cases (intended)

1. `merged_entities` counts proposed duplicates, including any skipped during
   apply (missing note, already a stub).
2. A string `duplicates` value is split into characters.
3. Merge validation compares against file stems, so a canonical known only as
   an alias is rejected.

Deliberate clarifications: (a) the reference walks candidate pairs in an
unspecified order; Enigma walks them sorted, so cluster order is
deterministic; (b) the reference rebuilt the cache from the environment's
vault on apply; Enigma uses the vault being consolidated; (c) non-object merge
elements are ignored rather than aborting.

## 7. Acceptance map

CO-01…CO-11 → §§1, 3–5; BL-01…BL-16 → §2.
