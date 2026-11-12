---
name: connections
description: Looks through the Enigma entity graph for notes that are linked to a topic in ways the user probably does not have in mind. Use it for "what connects to X", "show me something I forgot", "anything related to what I'm working on", or after wrapping up work on a topic.
---

# Connections

Requires the `enigma` MCP server. Results are shown in the chat. Nothing is
written unless the user says a connection is worth keeping.

## How to run it

1. **Choose starting points.**
   - Topic given: entities from `find_entity(topic)` and notes from
     `search_brain(topic, limit=3)`.
   - No topic: from `list_recent(10)`, take the two latest notes that sit
     in `Claude/Sessions/` or `Claude/Answers/`.
2. **Walk the graph.** `related_notes(path, hops=2)` for each starting
   note.
3. **Rank for surprise.** Relevance is not the goal here.
   - A result reached through two or more entities (see its `via` chain)
     beats a one-step result, which the user most likely knows about.
   - Remove anything the starting note links to itself. Check its
     `[[wikilinks]]` via `read_note`, or call `get_backlinks`.
4. **Present three to five.** One line per result:
   `[[note]] — via <chain>`, then a sentence on why it could matter today.
5. **Keep what the user likes.** One `capture` per chosen connection:
   `capture("<start> ↔ <note>: <why it matters>")`.
