---
name: resume
description: Briefs the user on where a project stands, using the session logs, decisions and tasks in the Enigma vault. Use it for "resume X", "where did I leave off", "where was I on X" or "what's the state of Y". The briefing is shown in the chat and saved only when --save is passed.
---

# Resume

Requires the `enigma` MCP server.

## How to run it

1. **Know the project.** It is the argument. With no argument, list the
   distinct `project` values from `query_notes(folder="Claude/Sessions")`
   and ask the user to choose.
2. **Gather.** `recall_bundle(mode="resume", project="<project>")`.
3. **Read the latest work.** Open the newest one or two session logs with
   `read_note`. The bundle shows where they are; only the full log says
   where things actually stopped.
4. **Brief the user in the chat, under four headings.**
   - **Last state**: the point the latest session reached.
   - **Open threads**: what recent sessions left unfinished.
   - **Decisions in force**: the bundle's lines from `Decisions.md`.
   - **Next actions**: the bundle's open tasks.
5. **Do not save it.** A status briefing goes stale in days, and a saved
   one would keep surfacing in search long after it stopped being true.
6. **Unless asked.** With `--save`, keep a snapshot: `write_note` to
   `Claude/Answers/YYYY-MM-DD Resume <project>.md`. Frontmatter for the note: `created: YYYY-MM-DD HH:MM`, `agent: claude`, `project: <project>`, `tags: [resume]`, `type: resume`.
