# Changelog

## 1.3.1

- `related_notes` and `context_bundle` stop as soon as there is nothing left to follow, so a very large `hops`
  value returns at once. Results are unchanged.
- `manifest.json` and the organizer's suggestions file are replaced through a temp file and a rename, so a crash
  cannot leave them half written.
- Adding aliases to an entity note that has no frontmatter no longer rewrites text above a `---` rule.
- Windows: a running `enigma-mcp.exe index` is now recognised by the nightly runner.
- Very large frontmatter mappings parse in linear time.
- Docs: the minimum Rust version is 1.89 (now also set in `Cargo.toml`); the quick start shows the real install
  commands after each preview; the state folder section says what is lost with it; the shared-index and
  `evals` instructions match the code; `uv` and the default extractor are listed as prerequisites.
- CI runs with a read-only token.

## 1.3.0

- **Windows support.** Enigma builds and passes its tests on Windows as well as macOS and Linux. Behaviour on
  macOS and Linux is unchanged.
- On Windows: the vault path is shown without the `\\?\` prefix; paths inside `Claude/` and the `.md` ending are
  matched in any case; access-denied, sharing and lock violations are retried as lock errors; programs are found
  through `PATHEXT`, so a `claude.cmd` or `codex.cmd` works as the extractor; `mcp-sync` prints commands with
  Windows quoting; the owner name comes from the account's display name.
- `schedule` stays macOS only. On Windows it says so at once and points to Task Scheduler.
- Text files are checked out with LF on every system, because the binary embeds prompts, skills and guides.
- CI runs on macOS, Ubuntu and Windows.
- README: the graph time-lapse is now a recording of Obsidian's own graph animation on a real vault, sped up, with a live node and link counter.
  Banner, feature strip and badges use a blue scheme that matches the graph; the banner's emblem keeps the logo's
  colours. The three diagrams give each kind of part its own colour.
- Diagram labels are no longer cut off at the right edge: the diagrams use the page's default font.
- Proofread: three garbled sentences and two ungrammatical phrasings in the spec, a stale version number,
  and a few mixed spellings.

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
