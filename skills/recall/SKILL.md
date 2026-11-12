---
name: recall
description: Answers a question from the Enigma vault with a source for every statement, then saves the answer in Claude/Answers/. Use it for "recall X", "what do we know about X", "what did I write about X", or any request for a researched answer from the user's own notes.
---

# Recall

Requires the `enigma` MCP server.

The job is to report what the vault says, not what you know. Three rules
hold throughout:

- A statement taken from a note carries that note as a `[[wikilink]]`. A
  statement that comes from anywhere else is tagged *(not from the vault)*.
- If the vault has too little on the question, the honest answer is "too
  little". Say it, save nothing, and do not pad the reply with general
  knowledge.
- Every answer closes with a `## Gaps` section.

## How to run it

1. **Gather.** `context_bundle(question, limit=10)` returns matching notes,
   the entities in them and the notes connected to those entities.
2. **Check there is enough.** Fewer than two results that truly address the
   question means stop here: tell the user, name the nearest notes you did
   find, and offer `capture` (to keep the question) or `add_task` (to
   research it later).
3. **Read properly.** Open the three to five strongest notes with
   `read_note`. Excerpts only point at notes; they are not evidence. Use
   `related_notes` once or twice if a connection looks like it holds
   context the direct results lack.
4. **Write three sections, in this order.**
   - `## Answer`: your synthesis, with a link on each statement.
   - `## Sources`: one bullet per note you linked.
   - `## Gaps`: what is missing, what conflicts, what looks outdated.
5. **Turn gaps into work.** Offer an `add_task` or a `capture` for each gap.
6. **Save.** `write_note` to `Claude/Answers/YYYY-MM-DD <slug>.md`.
   Build the slug from the first six words at most, and drop these characters: `\ / : * ? " < > | [ ] # ^`. If the file exists, add ` 2`, then
   ` 3`, and so on. Frontmatter for the note: `created: YYYY-MM-DD HH:MM`, `agent: claude`, `project:` the project if it is clear and `""` if not, `tags: [answer]`,
   `type: answer`, `question:` the user's exact wording in quotes.
7. **Reply.** Put the complete answer in the chat too. The saved note is for
   later; the user is waiting now.
