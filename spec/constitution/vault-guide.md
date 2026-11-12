# Enigma Vault — Guide for Agents

This vault is the shared memory of {OWNER} and the AI agents that work with
them (Claude Code, Codex, Claudian and whichever agents come next). The
markdown files are the source of truth, and Self-hosted LiveSync copies them
between machines.

## Start here

- How agents write in this vault is set out in [[Claude/README]], the
  constitution. Read it before your first write.
- Everything outside `Claude/` belongs to {OWNER}: read it freely, write to it
  only when asked to.

## Routing rules — where things go

- `Claude/Sessions/` — work logs from agents: what was done, learned and decided.
- `Claude/Concepts/` — evergreen notes on one topic each; extend rather than duplicate.
- `Claude/Inbox/` — short-lived captures that may be pruned.
- `Claude/Tasks.md` — follow-up work as Obsidian Tasks checkboxes.
- `Claude/Decisions.md` — the append-only decision ledger.
- `Claude/Graph/` — the semantic entity graph (People, Organizations,
  Domains, Topics, Projects, Sources). Ask it before exploring by hand.

<!-- When a new top-level content folder appears, add one routing line for it here, e.g.:
- `Interviews/` — interviews the owner ran (agents read only)
- `Resources/` — reference material imported from elsewhere
-->

## Tools

When the enigma MCP server is connected, use its tools instead of reading
and writing files directly: `search_brain` (full-text and semantic search),
`query_notes` (frontmatter queries in the style of Dataview),
`find_entity` / `related_notes` / `graph_stats` (the semantic graph),
`log_session`, `capture`, `upsert_concept`, `add_task` and `write_note`.
