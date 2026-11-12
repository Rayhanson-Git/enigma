# 04 — Search

Hybrid retrieval behind `search_brain` and `context_bundle`: a keyword lane
(BM25L) and a vector lane (Smart Connections vectors where fresh, a local
same-model fallback elsewhere), fused by Reciprocal Rank Fusion, with a
substring lane as fallback when the keyword lane finds nothing. Everything is
recomputed per query from the current vault; only vectors are cached.

Acceptance: `search.yaml` (BM-*, SE-*, HY-*, EM-*, SC-*).

## 1. Constants

| Name | Value |
|---|---|
| Skipped folder names | `.obsidian`, `.trash`, `.git` |
| BM25 variant | BM25L, k1 = 1.5, b = 0.75, delta = 0.5 |
| Lane depth | 50 (each lane keeps at most 50 paths) |
| RRF constant k | 60 |
| Default search limit | 20 |
| Excerpt fallback length | 120 characters |
| Title-match excerpt | `(title match)` |
| Fallback vector cache | `fallback_embeddings.json` in the state dir (16 §5) |
| Smart Connections folder | `.smart-env/multi`, files `*.ajson` |
| Smart Connections key prefix | `smart_sources:` |
| Model key | `TaylorAI/bge-micro-v2` |

## 2. Note walk

walk(folder) yields `(absolute path, rel path)` pairs:

1. Base: if folder is null or empty, the vault root; otherwise folder resolved
   per 03 §2 (an escaping folder fails with the escape error).
2. Recursively list every entry under base whose name matches `*.md` (case
   sensitive on POSIX, case-insensitive on Windows). Names starting with a
   dot match and dot-folders are descended into. Symbolic links to files are
   included; symbolic links to directories are not descended into. A
   directory whose name ends in `.md` is also yielded (quirk Q11). A base
   that does not exist or is not a directory yields nothing.
3. rel = the path relative to the **vault root** (not to base), components
   joined with `/`.
4. Skip the entry if any component of rel (including the file name) is
   `.obsidian`, `.trash` or `.git`. Nothing else is skipped here (for
   example `.smart-env` notes are walked).
5. Yield in path order (00 §3.3).

## 3. Candidate notes

candidates(tags, folder) → ordered list of `(rel, text)`:

- For every walked note (walk(folder)), read its text leniently (00 §3.1).
- If tags is a non-empty list, keep the note only when the set of casefolded
  tags is a subset of the casefolded str_list(parse(text)["tags"]) (03 §6).
  A scalar `tags: solo` counts as `[solo]`. Matching is case-insensitive:
  `[grid]` and `[GRID]` select the same notes; `[grid, nope]` selects none.
- A null or empty tags list applies no filter.

## 4. BM25L keyword lane

**tokenize(text)**: lowercase the text (00 §3.3), then take every maximal run
of ASCII `a-z` and `0-9` characters, in order. Example:
`Lighthouse-Grid Pipeline!` → `[lighthouse, grid, pipeline]`. Non-ASCII
letters split tokens.

**rank(corpus, query, limit)** with corpus an ordered `rel → text` map:

1. Empty corpus: `[]`.
2. Tokenize every document (in corpus order) and the query.
3. N = number of documents; dl(d) = token count of d; avgdl = total tokens /
   N; df(t) = number of documents containing t; tf(t, d) = occurrences of t
   in d.
4. idf(t) = ln(N + 1) − ln(df(t) + 0.5) for every token t that occurs in the
   corpus; a query token that occurs nowhere has idf 0.
5. score(d) = sum over the query tokens q, **in order and with repeats**, of

   idf(q) × tf(q, d) × (k1 + 1) × (c + delta) / (k1 + c + delta),
   where c = tf(q, d) / (1 − b + b × dl(d) / avgdl).

   All arithmetic is 64-bit floating point. A document without q gets 0 for
   that term. If avgdl is 0 the scores are NaN and nothing passes step 7.
6. Sort documents by score descending, stable (equal scores keep corpus
   order).
7. Keep scores strictly greater than 0; return the first limit
   `(rel, score)` pairs.

Examples: in `{a.md: "the lighthouse grid pipeline ingests outage data",
b.md: "an unrelated note about weather patterns"}` the query
`lighthouse pipeline` returns `[a.md]` only; `zephyr` ranks a note containing
it first; an unknown term returns `[]`.

## 5. Local vectors and the fallback cache

**cosine(a, b)** = dot / (|a| × |b|), where dot sums products over positions
present in both vectors (the shorter length) and each norm is over that
vector's full length. If either norm is 0 the result is 0.0.

**content hash**: lowercase hex SHA-256 of the UTF-8 bytes of the note text as
read leniently (newlines normalised). The text embedded is the whole note
text, frontmatter included.

**Vector cache files** (fallback cache here, entity cache in 07) share one
loader and saver; their format is in 16 §5.

- load(path): if the file cannot be stat-ed, `{}`. If this process already
  parsed this path and its (modification time in nanoseconds, size) is
  unchanged, return a copy of the remembered map. Otherwise read and parse
  it; on a read or JSON error return `{}` (and remember nothing). On success
  remember (stamp, map) and return a copy. Callers mutate their copy; an
  unsaved mutation never affects later loads.
- save(path, map): create a temp file in the same folder named
  `{file name}.` + random characters + `.tmp`, write the map as compact File
  JSON (16 §3.2), close, then atomically rename it over path. No lock retry.
  On any failure delete the temp file and surface the error (the old file is
  untouched). After success remember the new stamp and map.

**partition(note_texts)**:

1. sc = Smart Connections vectors (§6).
2. note_texts: the given ordered `rel → text` map, or, when not given, every
   walked note of the whole vault (walk(null)), read leniently, in walk
   order.
3. cache = load(`<state dir>/fallback_embeddings.json`).
4. For each rel in note_texts order: if sc has rel and it is fresh, its
   vector is used; else if cache has rel and that entry's `hash` equals the
   content hash, the cached `vec` is used; else rel is **stale**.

**note_vectors(note_texts, persist = true)**:

1. Partition.
2. If any note is stale: embed all stale texts in one batch (stale order),
   add each vector to the result (after the non-stale ones) and set
   cache[rel] = `{"hash": content hash, "vec": vector}`; then, if persist,
   save the whole cache (including entries for notes that no longer exist;
   nothing is pruned here).
3. Return the ordered `rel → vector` map: non-stale notes in note_texts order,
   then stale notes in stale order.

A second call with nothing changed embeds nothing. `persist=false` (dry
runs) embeds in memory and writes no file.

**stale_notes()**: the stale list of partition(whole vault). Read-only: never
embeds and never creates or writes the cache file.

## 6. Smart Connections reader

Reads the vectors the Smart Connections plugin stores in the vault.

1. Folder `<root>/.smart-env/multi`. Not a directory: `{}`.
2. Files directly in it whose name matches `*.ajson`, sorted by name.
3. Parse each file (memoised per path by (mtime ns, size), like §5, including
   failed parses):
   - Read leniently; split into lines; strip each line; drop lines that are
     empty after strip; from each remaining line remove all trailing `,`
     characters.
   - Join the lines with `,` and wrap in `{` … `}` (no lines: `{}`); parse as
     one JSON object. Duplicate keys: the last occurrence wins (so a later
     line for the same key supersedes earlier ones). Parse failure: the file
     contributes `{}`.
4. For each file in order, for each key in that file's object order:
   - skip keys not starting with `smart_sources:` (e.g. `smart_blocks:…`);
   - skip entries that are not objects, entries without a truthy `path`,
     and entries whose `embeddings` object lacks a truthy
     `TaylorAI/bge-micro-v2` member containing a `vec` member;
   - skip if `<root>/<path>` is not a regular file;
   - at = `last_embed.at` (missing object or member: 0); mtime_ms = the note
     file's modification time in seconds (64-bit float) × 1000;
   - result[path] = `{"vec": vec, "fresh": at >= mtime_ms}`. A later file
     overrides an earlier one for the same path.

`path` is used verbatim as the rel key. Example (one line of an `.ajson`
file):

```text
"smart_sources:Daily.md": {"path": "Daily.md", "embeddings": {"TaylorAI/bge-micro-v2": {"vec": [0.1, 0.2, 0.3]}}, "last_embed": {"at": 1783000000000}},
```

## 7. Hybrid search

hybrid(query, tags = null, folder = null, limit = 20) → list of
`{path, excerpt}`:

1. C = candidates(tags, folder) (§3). If C is empty return `[]`.
2. corpus = C as an ordered `rel → text` map; candidate set = its keys.
3. Keyword lane K = the rels of rank(corpus, query, 50).
4. Vectors V = note_vectors(note_texts, persist = true) where note_texts is
   corpus when there is no filter (tags null or empty and folder null or
   empty), and not given otherwise — a filtered search therefore reads and
   may embed the **whole vault** (quirk Q8).
5. q = the embedding of query (always computed, even for an empty query).
6. Vector lane S: for each `(rel, vec)` of V in V's order with rel in the
   candidate set, score cosine(vec, q); sort by score descending (stable);
   keep scores strictly greater than 0; first 50 rels.
7. Lanes = [K, S]; if K is empty, append the substring lane (below).
8. fused = RRF(lanes), then take `fused[:limit]` with slice semantics:
   limit ≥ 0 keeps the first limit items; a negative limit drops |limit|
   items from the end (quirk Q7).
9. Return `{"path": rel, "excerpt": excerpt(text, rel, query)}` for each.

**RRF(lanes)**: for each lane in order, for each item at 1-based rank r,
add 1 / (60 + r) to the item's score (64-bit float, accumulated in lane
order). Items are kept in first-seen order; the result is sorted by score
descending, stable on first-seen order.

**Substring lane** (only when K is empty): ql = lowercase(query); iterate the
corpus entries sorted by rel (plain code-point string order, not path order);
include rel if ql occurs in lowercase(stem), or in lowercase(line) for any line
of the full text (frontmatter included); stop after 50.

**excerpt(text, rel, query)**:

1. ql = lowercase(query); stem = the last `/`-component of rel minus its
   last 3 characters.
2. If ql occurs in lowercase(stem): `(title match)`.
3. Otherwise, for the lines of body(text) (03 §6): the first line whose
   lowercase contains ql, stripped.
4. Otherwise the first line of body(text) that is non-empty after strip,
   stripped, truncated to its first 120 characters.
5. Otherwise `""`.

An empty query therefore yields `(title match)` for every hit.

Examples (fixture mini vault, 00 and acceptance README):

| Query | Result |
|---|---|
| `ingestion pipeline` | first hit `{"path": "Projects/Lighthouse Grid.md", "excerpt": "Grid outage ingestion pipeline with a queue-free architecture."}` |
| `e` with tags `[grid]` | exactly `[Projects/Lighthouse Grid.md]` (no BM25 token, substring lane) |
| `port` over `aaa.md "an important announcement was made"` and `zzz.md "the port of hamburg is busy"` | zzz.md present, aaa.md absent (BM25 has hits, so no substring lane) |
| `weekly` on `Claude/Sessions/Weekly Review.md` | excerpt `(title match)` |
| unrelated query on `---\ntags: [x]\n---\n\n# Title Line\n\nReal content here.\n` | excerpt `# Title Line` |
| frontmatter-only note | excerpt `""` |

## 8. Cost notes (informative)

Each query re-reads candidate notes, re-tokenizes and re-ranks; vectors are
re-read from the memoised caches and only stale notes are embedded. A vault
of a few thousand notes is the design target.
