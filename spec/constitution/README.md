---
created: 2026-11-11
agent: claude
tags: [meta, constitution]
---

# The Claude/ Constitution

The rules every AI agent follows when writing to this vault. Read this note
before your first write in a session.

## Who owns what

- `Claude/` is the agents' area. Write there freely, using the layout below.
- Everything else in the vault belongs to the vault owner. Read it as much as
  you like, but write outside `Claude/` only when the owner explicitly asks,
  and confirm the exact path with them first.

## Layout of Claude/

- `Claude/Inbox/` — quick captures. One note per day named `YYYY-MM-DD.md`,
  each line a bullet that starts with the time.
- `Claude/Sessions/` — one note for each significant piece of work, named
  `YYYY-MM-DD <short title>.md`, saying what was done, what was learned and
  what was decided. Choose a title that will stand out later; a second note
  with the same title on the same day becomes `<title> 2.md` instead of
  replacing the first.
- `Claude/Concepts/` — evergreen notes, one idea each. Run `search_brain` on
  the topic before you create one, and extend an existing note instead of
  starting a near-copy. Concept names ignore case (`Tubeless Tyres` and
  `tubeless tyres` are one note). Add new material under a dated
  `## Update` heading; never quietly rewrite what is already there.
- `Claude/Index.md` — the table of contents. Each new session note adds one
  line here.
- `Claude/Tasks.md` — follow-up work as `- [ ]` checkboxes in Obsidian Tasks
  format, with due dates written `📅 YYYY-MM-DD`. Put tasks here rather than
  leaving them inside prose.
- `Claude/Decisions.md` — an append-only ledger with one line per decision:
  `- YYYY-MM-DD — <decision> ([[session note]])`. Add a line whenever a
  session settles something real, and still describe it in the session note.
- `Claude/Graph/` — the semantic graph: entity notes for People,
  Organizations, Domains, Topics, Projects and Sources, kept up to date by
  `index_brain`. The graph is plain markdown, so correct a wrong fact by
  editing the entity note. When collecting context on a subject, reach for
  `related_notes` and `find_entity` first.
- `Claude/Answers/` — answers produced by `/recall`, one note per question,
  named `YYYY-MM-DD <question slug>.md` with frontmatter `type: answer` and
  `question:`. Every claim links its source note as a `[[wikilink]]`, and
  anything that comes from model knowledge rather than the vault is marked
  *(not from the vault)*. Earlier answers are valid sources for later
  searches; that is why they are kept. Milestone snapshots from
  `/resume --save` (`type: resume`) and dossiers (`type: dossier`) live here
  too.
- `Claude/Digests/` — one review note per `/digest` run, named
  `YYYY-MM-DD.md` with frontmatter `type: digest`: caretaker health,
  captures waiting for triage, tasks, recent changes, pending proposals, new
  graph activity and suggested questions. Running it again on the same day
  replaces that day's digest.

## What to keep

- Memory comes in two kinds. **Context** lasts: Sessions, Concepts,
  Decisions. **Connections** fade: Inbox captures and passing references.
  Keep the first; let the second expire.
- Before promoting anything to a Concept or a Decision, ask: *will this
  still matter a year from now?* If not, it is noise and stays in the Inbox.
- `Claude/Inbox/` may be pruned at any time. Move anything worth keeping into
  Concepts, Tasks or Decisions before it goes.
- The filing rule for recall work: **save what lasts, skip what
  expires.** Answers and digests are filed because they become better
  search sources over time. Resume briefings and connection lists nobody
  approved are not filed, because stored stale state teaches search to
  surface stale context.

## How notes look

- Every agent note starts with YAML frontmatter holding `created`, `agent`,
  `project` and `tags`.
- Link related notes with `[[wikilinks]]`; a note with no links is a memory
  nobody will find again.
- Write for a reader who was not in the session: whole sentences, no pasted
  transcripts.

## Sharing the vault

- Never settle a sync conflict by deleting someone else's text.
- When you add to an existing note, append. Do not reorder or reword what a
  human wrote.

## Sheets

Some folders outside `Claude/` are **sheets**: structured records that the
owner has opened to agents by putting a `_schema.md` file in the folder.
That gives three classes of write:

| Where | Rule |
|---|---|
| `Claude/` | agents write freely |
| Sheet folders (contain `_schema.md`) | write only through `sheet_upsert`, which validates against the schema |
| Anywhere else | only on an explicit request from the owner (`confirm_outside_claude`) |

Read a sheet's contract with `sheet_schema`. Each row is one note. Rows are
never deleted; a sheet has states such as Withdrawn or Rejected for that.
The schema file belongs to the owner: suggest changes in words and never
edit it yourself.

## Organizer

The vault owner has given the organizer standing permission to move notes
inside the human topic folders on its own: notes sitting at the vault root
and notes filed in the wrong topic folder. Each note goes where its nearest
semantic neighbours already live (a cosine vote over the 10 nearest notes,
with a winning share of at least 0.7). The organizer never touches
`Claude/`, `00 - Maps of Content`, folders whose names start with a dot,
non-markdown files, or notes whose frontmatter says `organize: false`. It
never creates or renames folders, and when another note shares the same
file name it makes a proposal instead of moving. Every move is written to
`Claude/Organizer.md` and can be reversed with the `undo_move` tool.
Classifications with too little confidence are queued as proposals in the
same note; settling them improves later votes.
