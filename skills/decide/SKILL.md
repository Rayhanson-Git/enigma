---
name: decide
description: Records a decision in Claude/Decisions.md after checking it against earlier decisions, with the reason and a note of anything it replaces. Use it for "we're going with X", "log this decision", "help me choose between A and B", or when work reaches a choice that future sessions must follow.
---

# Decide

Requires the `enigma` MCP server.

`Claude/Decisions.md` is a log that is only added to. Existing lines stay
exactly as written, even when a later decision overturns them; the later
line records that it does.

## How to run it

1. **Know the history first.** `read_note` the whole of
   `Claude/Decisions.md`, then `search_brain("<topic>", limit=5)` for
   session logs and answers on the same subject.
2. **Surface conflicts.** If an existing line disagrees with the new
   decision, or would be replaced by it, show that line to the user
   verbatim and ask whether the new decision replaces it. Without a yes,
   do not record the new one: two contradictory lines would both be
   returned by later searches.
3. **Add one line at the end.** Write the file back with
   `write_note(overwrite=True)`, unchanged except for the new last line.
   Match the style the file uses. If it has none yet:
   `- YYYY-MM-DD <decision> — <reason in one line>`, and when step 2
   applied, append ` (supersedes YYYY-MM-DD "<old decision>")`.
4. **Record the reason.** A decision without its reason is hard to trust a
   few months on. Link any notes that back it up.
5. **Capture the consequences.** Create an `add_task` for each piece of
   work the decision causes, with `due` if the user named a date.
6. **Point to enforcement.** The log remembers; it does not enforce. If
   agents must act differently in some repository from now on, tell the
   user that the rule belongs in that repository's AGENTS.md or CLAUDE.md.
