---
name: rounds
description: Checks the health of the Enigma vault without changing anything, and reports problems most serious first. Use it for "run rounds", "how's the vault", "is anything drifting", "check the caretakers", or before trusting the vault again after time away. Looks at the last caretaker run, the graph, filing suggestions and tasks.
---

# Rounds

Requires the `enigma` MCP server.

This skill inspects. It does not fix. Any call with `apply=True` happens
only after the user has read the report and asked for it.

## How to run it

1. **Caretaker.** `librarian_status()`. Note the time of the last run, any
   step that failed, the health checks (stale embeddings, manifest drift,
   orphaned entities, cache consistency) and the number of pending
   proposals. A last run older than 48 hours puts `⚠ stale sweep` at the
   top of the report.
2. **Graph.** `graph_stats()` for the totals. Then
   `consolidate_graph(apply=False)`: report how many merges it suggests
   and quote the first three as returned.
3. **Filing.** `organize_vault(apply=False)`: report how many suggestions
   there are and give three as `note → suggested folder`.
4. **Tasks.** `list_tasks()`: report the number open. Quote any that are
   overdue or plainly duplicated. Leave them as they are.
5. **Report in the chat, in three groups, in this order.**
   - 🔴 **Broken**: failed runs, failed health checks, tools returning
     errors.
   - 🟡 **Drifting**: a stale run, duplicate entities, a growing pile of
     suggestions, dead tasks.
   - 🟢 **Healthy**: the counts, on one line.
   Give each finding a line with the evidence for it. Write "none" in an
   empty group.
6. **Offer next steps.** Suggest one for each red or yellow finding, and
   create an `add_task` only for those the user accepts. Merging and moving
   need the user's explicit word; they then use the same tools with
   `apply=True`.
7. **Do not save the report.** It describes one moment. If the user wants
   it on record, add it to the day's `log_session`.
