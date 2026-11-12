---
name: distill
description: Turns recurring material from captures and session logs into lasting, source-linked concept notes in Claude/Concepts/. Use it for "distill the inbox", "promote captures to concepts", "consolidate what we learned about X", or when the Inbox has piled up.
---

# Distill

Requires the `enigma` MCP server.

Captures and session logs are raw. Concept notes are what the vault should
still be saying a year from now. Most raw material should stay raw, so
there is a bar to clear.

## How to run it

1. **Collect.**
   - Topic given: `search_brain(topic, limit=10)` together with
     `query_notes(folder="Claude/Inbox")`.
   - No topic: `query_notes(folder="Claude/Inbox")`, and from
     `list_recent` the `Claude/Sessions/` notes of about the past two
     weeks.
2. **Apply the bar.** Sort the material into possible concepts. One passes
   only when at least two different notes say it, or when the user asked
   for that topic by name. What does not pass is left in the Inbox.
3. **Find where it belongs.** Check `find_entity(name)` and
   `query_notes(folder="Claude/Concepts")`. A concept that is already
   there, even under a slightly different name, gets extended. Do not
   create a twin.
4. **Read the sources in full** with `read_note` before writing anything.
5. **Write.** `upsert_concept(name, content)`, with content that:
   - states only what should hold for a long time;
   - links the source note on every statement;
   - shows both sides, each with its date, where sources conflict, and
     notes which is more recent without ruling on which is right.
6. **Report.** In the chat, say what was added to which concept, and for
   each item left behind, one line on why it did not pass.
7. **Change nothing else.** No deletions and no moves. Clearing the Inbox
   is up to the user.
