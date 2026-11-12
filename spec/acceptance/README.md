# Acceptance cases

Each YAML file lists behavioural cases as data: an id, the setup ("given"), the action ("when") and the
required observable result ("then"). Cases are written for Enigma and are normative together with the
numbered spec files. Implementations should turn every case into an automated test.

## Shared fixture: "mini vault" (MV)

Unless a case says otherwise, tests run against a temporary vault with this content and with
ENIGMA_STATE_DIR pointing at a fresh temporary directory (no test may touch a real state dir):

| Path | Content |
|---|---|
| .obsidian/app.json | {} |
| Projects/Lighthouse Grid.md | "---\ntags: [project, grid]\n---\n\n# Lighthouse Grid\n\nGrid outage ingestion pipeline with a queue-free architecture.\n" |
| Daily.md | "# Daily\n\nRemember to check the pipeline.\n" |
| Claude/Sessions/ | (empty dir) |
| Claude/Inbox/ | (empty dir) |
| Claude/Index.md | "# Index\n\n" |
| Claude/Concepts/Postgres.md | "---\ntags: [concept]\n---\n\n# Postgres\n\nRelational database used by the sync relay.\n" |

The fake embedder used in caretaker tests maps a text to [len(text), 0.0] (deterministic, no model).

## Files

| File | Spec |
|---|---|
| vault.yaml | 03-vault-rules |
| notes.yaml, tasks.yaml, multi_agent.yaml | 03-vault-rules, 02-tools |
| locking.yaml | 03-vault-rules, 14-cli |
| search.yaml (bm25, helpers, hybrid, embeddings, smart-connections) | 04-search |
| graph.yaml (query, cache, graphstore) | 05-graph |
| indexer.yaml, extractor.yaml | 06-indexer-extractor |
| consolidate.yaml, blocking.yaml | 07-consolidate |
| librarian.yaml | 08-librarian |
| organizer.yaml, mover.yaml | 09-organizer-mover |
| sheets.yaml | 10-sheets |
| recall.yaml | 11-recall |
| cleanup.yaml | 12-cleanup |
| evals.yaml | 13-evals |
| server.yaml | 02-tools, 01-config |
| sync.yaml (mcp-sync, skill-sync, hook-sync, hooks, provision, conventions, nightly, platform) | 15-sync-and-provision, 14-cli |
| sanity.yaml (binary, CLI surface, server start) | 14-cli, 00-overview, 02-tools |
