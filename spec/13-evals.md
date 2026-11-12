# 13 — Search evaluation harness

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/evals.yaml`. Formats and error kinds: 05 §0. The CLI flags
are in 14 §2 (evals).

The harness scores the production hybrid search (04-search `hybrid_search`,
real embedder) against *golden* queries: a synthetic fixture corpus shipped
with the source tree, and optionally a private set inside the live vault.
Metrics are rank-based floors, never exact orderings.

---

## 1. Constants

| Name | Value |
|---|---|
| fixture vault | `{source root}/spec/fixtures/vault` |
| fixture golden set | `{source root}/spec/fixtures/vault-golden.yaml` (17 queries) |
| live golden note | `Claude/Evals.md` |
| history file | `eval_history.jsonl` in the state directory |
| retrieval limit | 20 |
| k values | 5, 10 |

`{source root}` is the source checkout the binary was built from (the crate
manifest directory recorded at build time). The harness is a repo-local
developer tool; the defaults only make sense in a checkout.

## 2. Metrics

- **first relevant rank(hits, relevant)**: 1-based position of the first hit
  in `relevant`; none when absent (also for no hits). `([a,b,c], {b})` → 2.
- **recall@k(hits, expect)**: |first k hits ∩ expect| / |expect|; 0.0 when
  expect is empty. `([a,b,c,d], {a,d}, 2)` → 0.5.
- **success@k(hits, relevant)**: any of the first k hits is relevant.

## 3. Golden sets

Let FENCE be three backtick characters.

**load_golden(path)** — every failure is an EvalConfigError:

1. Not a file → `golden file not found: {path}`.
2. Read strictly. If the extension is `.md` (case-insensitive), use the first
   fenced block: FENCE, `yaml`, optional whitespace, a line break, then
   everything up to the next FENCE (dot matches newline). None → the text
   `no ` + FENCE + `yaml block found in {path}` (that is: "no", a space,
   FENCE, then "yaml block found in" and the path).
3. Parse YAML (1.1 safe rules, as frontmatter). Failure →
   `invalid YAML in {path}: {parser detail}`. Not a list →
   `golden set must be a YAML list: {path}`.
4. For entry i (0-based): not a mapping, or lacking `id` or `query` →
   `entry {i} in {path} needs 'id' and 'query'`. display() of id or query trimmed
   empty → `entry {i} in {path}: 'id' and 'query' must be non-empty strings`.
5. Fields: id = display()(id); query = display()(query); expect and accept via
   *as list*; tags via *as list* only when truthy (else none); folder = display()
   when truthy (else none); note = display() of a truthy note, else empty.
   *as list*: null → []; string, integer, float or boolean → [display()] (so
   `true` → `["True"]`); list → display() of each; anything else →
   `golden {id}: '{field}' must be a string or list, got {type name}` with type
   names `dict`, `date`, `datetime`, `bytes`, `set` for mapping, date,
   date-time, binary and set values.
6. After all entries, per query in order: a repeated id →
   `duplicate golden id: {id}`; empty expect →
   `golden {id}: 'expect' must be non-empty`.

Paths are vault-relative and compared exactly (no case folding).

## 4. Path validation

**validate(queries, vault root, strict)** → map id → missing paths. For each
query, a path of expect + accept (in that order) is *missing* when it is
absolute, has a drive, contains a `..` segment (decided lexically, never
probed on disk), or `{root}/{path}` is not a file. Strict and anything
missing → EvalConfigError `golden paths missing from vault: {detail}` where
detail = per query `{id}: {paths joined by ", "}`, joined by `; `.

## 5. Running

**run(vault, state dir, embedder, queries, lenient)**:
1. Create the state directory if missing.
2. missing = validate(strict = not lenient).
3. For each query: lenient and every expect path is missing → a *skipped*
   result (hits [], rank none, recall 0.0, success false, missing = its
   missing list). Otherwise hits = paths of hybrid_search(query, tags,
   folder, limit 20); expect′ = expect − missing; relevant = (expect ∪ accept)
   − missing; rank, recall@5/10 over expect′, success@5/10 over relevant;
   missing = its missing list.
4. Aggregates over scored (non-skipped) results with n = their count (1 when
   zero): success@k = number of successes / n; recall@k = sum / n; MRR = sum
   of 1/rank over scored results having a rank / n; skipped = count.

An accept-only hit yields a rank (and success) but recall 0.0.

## 6. Output

### 6.1 Table

Lines joined by LF (no trailing LF; the CLI adds one):
1. header: `id` left-aligned to width 28, space, `rank` right-aligned to 4,
   space, `r@10` right-aligned to 5, two spaces, `miss`.
2. per query: id left-aligned to 28 (longer ids are not cut), space, rank
   right-aligned to 4 (`-` when none; `skip` for skipped), space, recall@10
   2-decimal right-aligned to 5 (`-` for skipped), two spaces, missing paths
   joined by `, `.
3. sixty `-` characters.
4. One line, segments separated by two spaces, every {x} 2-decimal:
   `queries {scored}  skipped {s}  success@5 {x}  success@10 {x}  recall@5 {x}  recall@10 {x}  MRR {x}`

### 6.2 JSON

Keys in order: `success_at_5, success_at_10, recall_at_5, recall_at_10, mrr,
skipped, n_queries, queries` where queries = `[{id, first_rank, skipped,
recall: recall@10, missing}]`. Rates are floats (float text: `1.0`, `0.5`, `0.0`);
first_rank integer or null. Printed as indented File JSON.

### 6.3 History

Unless `--no-history`: create the state directory, append one compact File JSON
line (ASCII) + LF to `eval_history.jsonl`: `{ts, git, vault, golden,
…the §6.2 keys}`. ts = local `YYYY-MM-DDTHH:MM:SS`; git = trimmed stdout of
`git -C {source root} rev-parse --short HEAD` (5 s timeout; empty output, a
spawn failure or a timeout → null); vault and golden = the paths used, as
given, with redundant separators and `.` segments removed. An I/O failure
prints `warning: could not write eval history: {OS error text}` to stderr and
does not change the exit code.

## 7. Live golden set

`--init-live`: target = `{vault}/Claude/Evals.md`. Exists (any file system
entry) → not created, untouched. Otherwise create parent folders and write
exactly the following (between the four-backtick fence; ends with LF after
the closing FENCE):

````markdown
# Search eval golden set (live vault)

Queries for `enigma-mcp evals --live`. Paths are vault-relative. A query
whose expected notes have all gone is skipped and reported, never fatal,
because a live vault drifts. Add a query whenever a search disappoints you.

```yaml
- id: example-constitution
  query: what are the rules for agents writing to the vault
  expect:
    - Claude/README.md
  note: seed example -- replace or extend freely
```
````

Live runs are lenient.

## 8. Quality gate (EV-18)

Only on request and with the real model available: when
`ENIGMA_RUN_EVALS` is set to any non-empty value, the paraphrase query
`para-query` (query `puppy vaccination clinic`, expected
`Inbox/Biscuit vet reminder.md`, fixtures/vault-golden.yaml) ranks its
expected note within the top 10 of the fixture run; when it is exactly `1`,
that run also has skipped 0, success@10 ≥ 0.80 and MRR ≥ 0.50.

## 9. Acceptance map

EV-01…EV-03 → §2; EV-04…EV-09 → §§3–4; EV-10…EV-13 → §5; EV-14…EV-17 →
§§6–7 and 14-cli; EV-18 → §8.
