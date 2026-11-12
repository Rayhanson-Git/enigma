# Changelog

## Unreleased

- README: the graph time-lapse is now a recording of Obsidian's own graph animation on a real vault, sped up.
  Banner, feature strip and badges use a blue scheme that matches the graph; the banner's emblem keeps the logo's
  colours. The three diagrams give each kind of part its own colour.

## 1.2.1

- Search: entity notes under `Claude/Graph/` count half in the final ranking, so the note a person or agent
  wrote comes before the entity extracted from it. On the real-vault test set MRR rises from 0.66 to 0.84.
- README: a time-lapse of a real vault's graph, built from Obsidian's own layout and ending on a real screenshot,
  and a brushed-metal emblem in the banner.

## 1.2.0

- The extractor's model now runs with its tools switched off unless `ENIGMA_EXTRACTOR_NO_TOOLS` is explicitly off.
- `organize_vault(apply=true)` called by an agent moves at most `ENIGMA_MAX_MOVES` notes; the default is 0.
- New wording for the tool descriptions, the server instructions, the model prompts and the nine skills.
- The skills moved to a top-level `skills/` folder.
- `undo_move` takes the maintenance lock, so it cannot run in the middle of a sweep.
- The organizer checks that the undo journal is writable before it moves a note.
- The undo journal and Claude's settings file are replaced through a temp file and a rename.
- Error messages for dates, times and invalid UTF-8 say what is wrong in plain words.
- Empty values and booleans print as `null`, `true` and `false`.
- CI runs on macOS and Ubuntu.
- Keyword ranking corrected: a note's score levels off as a word repeats, and title words weigh more.
  On a real 1,570-note vault this lifts the right note into the top five for 94% of test questions.
- After the organizer moves a note, its vector is refreshed in the same run.
- Vector cache files and a newly created state folder are readable by their owner only.
- `AGENTS.md`: setup instructions an AI agent can follow from the repository link.
- `install-conventions` fills in the owner's name by itself: `ENIGMA_OWNER`, else the system account's full name.

## 1.1.0

- `schedule` command: install, remove or print one nightly launchd job.
- `Claude/Organizer.md` keeps one block of current suggestions; tick a box to dismiss one, and unticked ones
  expire after 15 days.
- An error in one note never stops an indexing run; the note is recorded and retried.
- The Librarian's own notes no longer count as stale in the health check.

## 1.0.0

- First release: 25 MCP tools, hybrid search, the knowledge graph, the organizer, the Librarian, typed sheets,
  hooks, provisioning and nine skills.
