---
name: digest
description: Writes a review of what has changed in the Enigma vault since the last review and saves it in Claude/Digests/. Use it for "digest", "what's new in the vault", "catch me up", or a morning or weekly review. Covers caretaker health, new captures, tasks, changed notes, pending proposals and new graph entities.
---

# Digest

Requires the `enigma` MCP server.

## How to run it

1. **Find the starting date.** Look in `Claude/Digests/`
   (`query_notes(folder="Claude/Digests")`). The latest file is named
   `YYYY-MM-DD.md`; that date is `since`. No earlier digest means no
   `since`, and the bundle then looks back one week.
2. **Gather.** `recall_bundle(mode="digest", since="<YYYY-MM-DD>")`.
3. **Write the sections below, always all of them and always in this
   order.** Write "none" under a heading that has nothing to show. If a part
   of the bundle came back with `status: "error"`, print
   `⚠ <section>: unavailable (<error>)` in its place.

   | Heading | What goes in it |
   |---|---|
   | `## Health` | Time of the last caretaker run and a one-line summary of the bundle's `librarian` part. Begin with `⚠ stale sweep` if that run is older than 48 hours. |
   | `## Captures to triage` | Each note in `inbox_captures`, linked. |
   | `## Tasks` | Count of open tasks, the tasks, then `done_recently`. |
   | `## Recent changes` | `recent_notes` as links, grouped by top-level folder. Notes these skills wrote themselves (digests, answers) may be left out when they dominate. |
   | `## Proposals pending` | The `proposals` count, linking `[[Organizer]]` and `[[Librarian]]`. |
   | `## New graph activity` | Each note in `new_entities`, linked; the file name is the entity. |
   | `## Follow-ups due` | See below. |
   | `## Logging habit` | See below. |
   | `## Suggested questions` | Two or three one-line questions that the new material makes answerable, ready to hand to `/recall`. |

   *Follow-ups due.* Query the `jobs` sheet for rows due today or earlier
   that are still live: `sheet_query("jobs", {"next_follow_up": {"lte":
   "<today>"}, "status": {"nin": ["Rejected", "Ghosted", "Withdrawn"]}})`.
   Print each as `[[<note>]] — <company> <role> (<status>, due
   <next_follow_up>)`. A vault without that sheet, or a client without
   `sheet_query`, gets `⚠ Follow-ups: unavailable`; the digest continues.

   *Logging habit.* Count the notes in `Claude/Sessions/` dated within the
   past seven days and report the number in one sentence. If it is zero,
   say that work which is not logged cannot be recalled later. One line,
   no lecture.

4. **Save.** `write_note` to `Claude/Digests/YYYY-MM-DD.md` for today, with
   `overwrite=true`: a later run today supersedes an earlier one.
   Frontmatter for the note: `created: YYYY-MM-DD HH:MM`, `agent: claude`, `project: ""`, `tags: [digest]`, `type: digest`.
5. **Reply.** Post the digest in the chat.
