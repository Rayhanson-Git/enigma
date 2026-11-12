# 14 — Command line

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/sanity.yaml` (SN-01…SN-08) plus the CLI cases in indexer, librarian,
sheets, recall, evals, locking and sync. Conventions and error kinds: 05 §0.

Enigma is one executable, `enigma-mcp`. Every maintenance command is a
subcommand; without a subcommand it is the MCP server.

---

## 1. Global rules

| Topic | Rule |
|---|---|
| no subcommand | Run the stdio MCP server (01, 02). Closing stdin ends it with exit 0. |
| `--version`, `-V` | Print `enigma-mcp {VERSION}` + LF to stdout, nothing to stderr, exit 0. VERSION is the Cargo.toml package version (today `enigma-mcp 1.1.0`). Never reads the environment, loads the model, opens a vault or creates a state dir. |
| `--help`, `-h` | Print usage to stdout naming the binary and every subcommand of §2; exit 0. Each subcommand also accepts `--help`. |
| usage errors | Unknown subcommand or flag, missing or malformed argument (except the missing VAULT of `index-nightly` and `install-conventions`, which exit 1 as written in §2), or a rule below marked *usage error*: usage text on stderr ending with the line `enigma-mcp {subcommand}: error: {message}` (`enigma-mcp: error: …` at top level), nothing on stdout, exit 2. The usage text itself is not normative; the message is where this spec writes it. |
| other failures | Unless a subcommand says otherwise, an error ends the command with stderr `error: {Kind}: {message}` + LF (kind display name of 00 §5) and exit 1. |
| output | stdout and stderr are UTF-8 on every platform (also on legacy-codepage consoles); lines end with LF. JSON output is File JSON (16 §3) unless stated. |
| locks | Taken as listed in 03 §10; a busy lock is LockBusy (exit 1). |
| vault argument | A positional VAULT is used as given; a vault root that does not exist fails with the VaultError of 03 §1 (exit 1) unless the subcommand says otherwise. |

## 2. Subcommands

### index

`enigma-mcp index VAULT [--backend NAME] [--batch N] [--force] [--retry-failures] [--rebuild-only]`

Always under the caretaker lock. `--rebuild-only`: rebuild graph.db from the
markdown (no LLM) and print compact File JSON `{"rebuilt": true, "db": "<db path>"}`
+ LF. Otherwise run one indexer batch (06 §4; batch default 25, embeddings
precomputed) with the extraction extractor for `--backend` (else
`ENIGMA_EXTRACTOR`) and print the counts as indented File JSON + LF. Exit 0.
`--backend` accepts any string when the arguments are parsed; the name is
checked when the extractor is created, so an unknown backend is stderr
`error: ExtractorError: Unknown backend: {name}` + LF, exit 1 (not a usage
error). A note that fails is not a command error: a missing extractor CLI
(`{backend} CLI not found on PATH`), a note that is not valid UTF-8 or cannot
be read, or any other per-note error is recorded in the manifest (06 §4,
§5.4), and the command still prints the counts and exits 0. Only the run
errors listed in 06 §4 exit 1.

### consolidate

`enigma-mcp consolidate VAULT [--backend NAME] [--apply]`

07 §5 with the consolidation extractor; `--backend` is checked as for `index`
(unknown → ExtractorError, exit 1). Dry run by default (no lock);
`--apply` takes the caretaker lock and applies merges. Prints the result as
indented File JSON + LF; exit 0.

### organize

`enigma-mcp organize VAULT [--dry-run]`

Organizer sweep (09 §3), applying unless `--dry-run`; the caretaker lock is
taken unless `--dry-run`. Prints the report as indented File JSON + LF; exit 0.

### librarian

`enigma-mcp librarian VAULT [--dry-run] [--max-moves N] [--auto-cleanup] [--confirm-first-run]`

1. `--max-moves` < 0 → usage error `--max-moves must be >= 0`.
2. Not `--dry-run`, not `--confirm-first-run`, and no Librarian state file in
   the vault's state dir → usage error
   `no Librarian sweep has run on this vault yet. Run it with --dry-run, review the report, then run once by hand with --confirm-first-run before scheduling it.`
   Nothing is written (not even `Claude/Librarian.md`).
3. Run the sweep (08 §5) under the caretaker lock unless `--dry-run`;
   `--auto-cleanup` forces auto-cleanup on, otherwise `ENIGMA_AUTO_CLEANUP`
   decides.
4. `--dry-run`: print the report section (08 §7.1, stamped with the time of
   printing; its last line ends with LF), then one more LF, so a blank line
   separates it from the JSON. Then, in both modes, print the result as
   indented File JSON + LF.
5. Exit 1 when the result's errors map is non-empty, else 0.

### cleanup

`enigma-mcp cleanup VAULT [--apply-repairs [--force]] [--apply-retirements [--paths P …]]`

12 §9. Report-only by default; `--apply-repairs` wins over
`--apply-retirements`; `--force` lifts the mass-deletion guard; `--paths` with no
values retires nothing. Exit 0.

### sheets

`enigma-mcp sheets VAULT --check`

`--check` is required (usage error without it). Prints the 10 §9 report as
indented File JSON with non-ASCII characters written literally (16 §3.5) + LF. Exit 0 when
clean, 1 otherwise. A duplicate sheet name is SheetError (exit 1).

### recall

`enigma-mcp recall --vault V [--context] [--project P] [--budget N]`

11 §5. `--vault` is required and `--budget` must be an integer (default 2000);
only those argument errors exit 2. With `--context` prints the context block
with no trailing newline; without it prints nothing. Every runtime error
(including a missing vault) is swallowed: empty stdout, exit 0.

### evals

`enigma-mcp evals [--vault V] [--golden G] [--live] [--json] [--no-history] [--init-live]`

13. `--live` / `--init-live` use `ENIGMA_VAULT_PATH` and `Claude/Evals.md` (taking
precedence over `--vault` / `--golden`); without the variable →
`--live/--init-live require ENIGMA_VAULT_PATH`. Defaults: the fixture vault
and golden set (13 §1). `--init-live` prints `created: {path}` or
`already exists: {path}` + LF, exit 0. Otherwise prints the table + LF (or the
JSON + LF with `--json`), appends history unless `--no-history` (a failure is
the 13 §6.3 warning), exit 0. EvalConfigError or VaultError → stderr
`error: {message}` + LF, exit 2.

### index-nightly

`enigma-mcp index-nightly VAULT [STATE_DIR]` — 15 §9. No VAULT argument → stderr
`usage: enigma-mcp index-nightly <vault> [state-dir]` + LF, nothing on stdout,
exit 1 (not a usage error). Otherwise exits 0, 1 or 2 as stated there.

### install-conventions

`enigma-mcp install-conventions VAULT` — 15 §2. No argument → stderr
`Usage: enigma-mcp install-conventions <vault-path>` + LF, nothing on stdout,
exit 1 (not a usage error); missing vault → exit 1.

### mcp-sync

`enigma-mcp mcp-sync [--check] [--vault V] [--manifest M] [--config C]` — 15 §6.
Exits 0, 1 (check found work, a registration failed, an unreadable config or
manifest, a manifest that is not JSON or has no `servers` list, or claude
could not be started), 2 (abort or usage) or 3 (claude CLI missing).

### skill-sync

`enigma-mcp skill-sync [--check] [--force] [--dest D]` — 15 §8. Exit 1 only for
`--check` with pending work.

### hook-sync

`enigma-mcp hook-sync [--check] [--install]` — 15 §5. Exit 0, 1 (check found
missing or drift, or malformed hook data refused with `error: InvalidData: …`)
or 2 (usage or SyncError).

### hook

`enigma-mcp hook session-start` and `enigma-mcp hook stop-nudge` — 15 §3 and §4.
Both read stdin and always exit 0, whatever the input (usage errors in the
subcommand name still exit 2).

### schedule

`enigma-mcp schedule (install|uninstall|print) --job (index|librarian) [VAULT] [--force] [--dry-run]`
— 15 §9.2. Exit 0; 1 for a missing vault or a failed `launchctl bootstrap`;
2 for usage errors and refusals (the other job installed, a different plist
without `--force`).

### provision

`enigma-mcp provision VAULT [--check]` — 15 §7.5. Exit 0. Without `--check` a
missing vault → `error: Vault root does not exist: {vault}`, exit 2; `--check`
never checks the vault and reports every plugin missing.

## 3. Exit code summary

| Code | Meaning |
|---|---|
| 0 | success (also: recall and hook on any runtime failure; index-nightly skips) |
| 1 | failure: an error, a librarian sweep with errors, sheets not clean, a check that found pending work, mcp-sync registration failure or read error, malformed hook data, an index-nightly round that failed, a schedule install whose launchctl bootstrap failed |
| 2 | usage error; schedule refusal; EvalConfigError; provision (without `--check`) on a missing vault; hook-sync SyncError; mcp-sync abort; unreadable vault in index-nightly |
| 3 | mcp-sync could not find or start the claude CLI |

## 4. Sanity case

`enigma-mcp --version` with no environment at all prints exactly
`enigma-mcp 1.2.0` + LF and exits 0 (SN-01, SN-04). It is the first check of
every build and replaces the reference's import smoke test; the handshake and
smoke call of SN-05/SN-06 replace its probe script.
