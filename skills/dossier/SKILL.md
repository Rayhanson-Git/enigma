---
name: dossier
description: Builds a full briefing on a topic from the Enigma vault, with a source for every statement, and saves it in Claude/Answers/. Use it when a short answer is not enough, such as "give me the dossier on X", "brief me on everything about Y" or "prepare me for this meeting".
---

# Dossier

Requires the `enigma` MCP server.

Where `/recall` answers one question, a dossier covers a whole topic. The
same honesty rules apply: link each statement to its note or tag it
*(not from the vault)*, and when the vault cannot support a briefing, say
so and save nothing.

## How to run it

1. **Gather and check.** Call `context_bundle(topic, limit=10)`. A dossier
   needs three or more results that are truly on topic, drawn from at least
   two kinds of note (session logs, concepts, earlier answers, graph
   entities). Short of that, stop: report that there is not enough, name
   the nearest notes, and suggest `/recall` with a narrower question or an
   `add_task` to research the topic.
2. **Read.** Open the five to eight best notes with `read_note`. Add
   context from `find_entity` and from one or two `related_notes` paths if
   they cover ground the search results do not.
3. **Write these five sections.**
   - `## Executive summary`: the half-minute version.
   - `## Timeline`: dated events from session logs and
     `Claude/Decisions.md`, earliest first.
   - `## Key concepts`: the ideas the topic depends on, each linked.
   - `## Tensions and open questions`: where notes contradict each other
     (show both, with dates) and what the vault has no answer for.
   - `## Sources`: all linked notes.
4. **Save.** `write_note` to `Claude/Answers/YYYY-MM-DD Dossier <slug>.md`.
   Build the slug from the first six words at most, and drop these characters: `\ / : * ? " < > | [ ] # ^`. Add ` 2`, ` 3` and so on if the
   name is in use. Frontmatter for the note: `created: YYYY-MM-DD HH:MM`, `agent: claude`, `project:` the project if clear and `""` if not, `tags: [dossier]`,
   `type: dossier`, `question:` the topic in the user's words.
5. **Reply.** Show the full dossier in the chat. A rendered page is a nice
   extra where the client supports one, but the vault note is the record.
6. **Follow up.** Offer an `add_task` for each open question.
