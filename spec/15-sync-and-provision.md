# 15 — Conventions, hooks, sync commands, provisioning, nightly runner

Author: Harvey Rayhanson (Rayhanson-Git). Normative. Acceptance:
`acceptance/sync.yaml` (CV, HK, HS, SK, MS, PV, NT, PL cases). Conventions and
error kinds: 05 §0; CLI flags and exit codes: 14-cli. EXE below is the
absolute canonical path of the running `enigma-mcp` executable.

Everything here is a subcommand of the one binary; no interpreter, virtual
environment or script directory exists (PL-02). Text files Enigma writes
outside notes (settings.json, community-plugins.json, rendered plists, state
files) use LF line ends on every OS.

---

## 1. Bundled assets

Compiled into the binary byte for byte from the source tree:

| Asset | Source | Installed as |
|---|---|---|
| CONSTITUTION | `spec/constitution/README.md` | `Claude/README.md`, `{OWNER}` filled |
| GUIDE | `spec/constitution/vault-guide.md` | `CLAUDE.md` and `AGENTS.md`, `{OWNER}` filled |
| SKILLS | `skills/<name>/SKILL.md` for clip, connections, decide, digest, distill, dossier, recall, resume, rounds | `<dest>/<name>/SKILL.md` |
| MCP MANIFEST | §6.1 | — |
| PLUGIN LOCK | §7.1 | — |
| SMART ENV | §7.4 | `.smart-env/smart_env.json` |

## 2. Conventions install

**install(vault, owner = null)** → created list (vault-relative, `/` separators):

1. For `Claude/Inbox`, `Claude/Sessions`, `Claude/Concepts`, `Claude/Answers`,
   `Claude/Digests` in this order: not a directory → create it (with parents)
   and record it.
2. `Claude/README.md` absent (no file system entry) → write CONSTITUTION, record.
3. `Claude/Index.md` absent → write exactly
   `# Index\n\nMap of agent-written notes. Session entries append below.\n\n`, record.
4. `Claude/Decisions.md` absent → write exactly the text below (LF line ends,
   U+2014 dash, ends with one empty line), record:

```text
---
created: 2026-01-01
agent: claude
tags: [decisions]
---

# Decisions

Append-only log. One line per decision:
`- YYYY-MM-DD — <decision> ([[session note]])`

```

5. owner name = `owner` when given (even empty), else `ENIGMA_OWNER`, else the
   full name of the system account running the command (the first field of
   its passwd entry; on Windows the account's display name, when it has
   one). Each candidate is stripped and has every CR and LF
   replaced with one space; a candidate that is then empty is skipped (an
   explicit empty `owner` skips straight to the fallback). Fallback:
   `the vault owner`. Guide text = GUIDE, and constitution text =
   CONSTITUTION, each with every `{OWNER}` replaced.
6. For `CLAUDE.md` then `AGENTS.md` at the vault root: absent → write the guide
   text, record.

Existing files and folders are never touched. Writes are plain file writes
(the conventions installer runs outside the MCP quarantine).

**CLI** `enigma-mcp install-conventions VAULT`: no argument → stderr
`Usage: enigma-mcp install-conventions <vault-path>` + LF, exit 1. VAULT not a directory → stderr
`Vault not found: {VAULT}` + LF, exit 1, nothing created. Otherwise stdout
`Created: {created joined by ", "}` + LF, or `Created: nothing (already installed)`
+ LF; exit 0. The owner comes from `ENIGMA_OWNER`, else the system account (step 5).

## 3. Hook: session-start

`enigma-mcp hook session-start` is Claude Code's SessionStart hook. It never
blocks a session: every path below exits 0.

1. Read all of stdin as UTF-8; empty → `{}`. Not JSON, or not an object → exit
   silently.
2. cwd = the payload's `cwd`; a value that is not a string (or null/absent) →
   empty. Strip; replace every `\` with `/`; remove trailing `/`; project = the
   text after the last `/`. Empty → exit silently (recall not run). Both
   separators count on every OS.
3. `ENIGMA_VAULT_PATH` unset or empty → stderr
   `enigma session-start: ENIGMA_VAULT_PATH is not set; set it to enable recall.`
   + LF. Not a directory → stderr
   `enigma session-start: vault not found at {quoted(path)}; set ENIGMA_VAULT_PATH to enable recall.`
   + LF. No default vault exists.
4. Produce the recall context block exactly as
   `enigma-mcp recall --vault <vault> --context --project <project>` would
   (11 §4), bounded by 15 seconds (a child process of EXE with a 15 s
   timeout is the reference implementation). On success with non-empty output,
   write it to stdout unchanged (no newline added). Failure, timeout or empty
   output → nothing. The recall step's stderr is never copied to stdout.

## 4. Hook: stop-nudge

`enigma-mcp hook stop-nudge` is the Stop hook. NUDGE is exactly
`Significant session with no log_session — consider filing a session log before finishing.`
(U+2014 dash). Threshold 10.

1. Read stdin; empty → `{}`. Not a JSON object, no non-empty string
   `transcript_path`, or the path is not a file → exit 0 silently.
2. Read the transcript leniently (invalid UTF-8 dropped) and split into lines.
3. For each line: strip; skip empty lines, non-JSON lines and JSON values that
   are not objects. message = the entry's `message` when that key is present,
   else the entry itself; a message that is not an object or whose `content` is
   not a list contributes nothing. Each content element that is an object
   with `type` == `tool_use` counts once; a log is seen when display() of its
   `name` (absent → empty) contains `log_session`.
4. count ≥ 10 and no log seen → stdout NUDGE + LF. Exit 0 in every case.

## 5. hook-sync

Settings file S = `CLAUDE_SETTINGS_PATH` when set and non-empty, else
`~/.claude/settings.json`.

| Event | Subcommand | Expected command |
|---|---|---|
| `SessionStart` | `session-start` | `"EXE" hook session-start` |
| `Stop` | `stop-nudge` | `"EXE" hook stop-nudge` |

(EXE wrapped in double quotes, one space, `hook`, one space, the subcommand.)

**Recognition** (the "enigma-mcp hook" marker): for an event, walk
`hooks[event]` entries in order (null → none); entries without a `hooks` list
are skipped; within an entry, hook objects in order (a missing `command`
counts as `""`). A hook object is ours when its command, after deleting every
`"` character, contains `enigma-mcp hook <sub>` or `enigma-mcp.exe hook <sub>`
for that event's subcommand. The first match decides.

**Read S**: absent, empty or whitespace only → `{}`. Invalid JSON → SyncError
(implementation-defined parser message). Not an object → SyncError
`{S}: settings.json must be a JSON object`. `"hooks": null` counts as no hooks,
and a null `hooks.SessionStart` or `hooks.Stop` counts as no entries.

**Malformed hook data** is refused rather than replaced, so user content is
never discarded: a `hooks` value that is neither null nor an object, a
`SessionStart` or `Stop` value that is neither null nor a list, or such a list
holding an element that is not an object → InvalidData
`{S}: {what}; settings left unchanged`, where what is
`"hooks" is not a JSON object`, `hooks.{event} is not a list` or
`hooks.{event} holds an entry that is not a JSON object` (the first problem
found, events in table order). Both `--check` and `--install` then print
nothing on stdout, write nothing, and end with stderr
`error: InvalidData: {message}` + LF, exit 1 (the general rule of 14 §1; the
file is left untouched). Values of other
events are not inspected.

**Classify** → `{missing, present, drift}` (events in table order): no
recognised hook → missing; recognised and its command equals the expected
command exactly → present; otherwise drift.

**Install**: classify first (this result is what gets printed). If anything
is missing: for each missing event append `{"hooks": [{"type": "command",
"command": <expected>}]}` to `hooks[event]` (creating the `hooks` object or the
event list at the end of their parent when absent; replacing a null
`hooks` or a null event value), create S's parent folders, and write S as indented File JSON plus a
final LF. Existing keys keep their order; drifted entries are never touched or
duplicated. Nothing missing → no write.

**CLI** `enigma-mcp hook-sync (--check | --install)`: neither flag → usage error
containing `specify --check or --install` (exit 2). Both → behaves as
`--check`. Prints the classification as indented File JSON + LF, e.g.

```json
{
  "missing": [
    "SessionStart",
    "Stop"
  ],
  "present": [],
  "drift": []
}
```

`--check` never writes and exits 1 when missing or drift is non-empty, else 0.
`--install` exits 0. A SyncError prints `error: {message}` + LF to stderr, exit 2,
S unchanged. Malformed hook data exits 1 as described above.

## 6. mcp-sync

### 6.1 Manifest

JSON `{"servers": [entry …]}`; entry fields `name`, `transport` (default
`stdio`), `command`, `url`, `args` (default `[]`), `env` (default `{}`), `why`
(default `""`). The built-in manifest, in this order:

| name | command | args | env | why |
|---|---|---|---|---|
| `enigma` | `{ENIGMA_BIN}` | `[]` | `{ENIGMA_VAULT_PATH: "{VAULT}"}` | `The memory vault: persistent shared memory for agents.` |
| `fetch` | `uvx` | `["mcp-server-fetch@2026.6.4"]` | `{PYTHONIOENCODING: "utf-8"}` | `Web ingest: turns a URL into clean markdown for /clip.` |
| `arxiv` | `uvx` | `["arxiv-mcp-server@0.5.0"]` | `{}` | `Paper ingest: arXiv search and download as markdown. Paper content is untrusted input.` |

All three are transport `stdio`. An entry with neither a truthy `url` nor a
truthy `command` → abort `manifest entry {quoted(name)}: stdio transport requires a command (or provide a url)`
(a missing name renders as `null`).

**Placeholders** in command, url, every arg and every env value (never in
names, env keys or why; the manifest data is not modified): `{ENIGMA_BIN}` →
EXE; `{REPO}` → the absolute directory containing the manifest file in use
(the current directory for the built-in manifest); `{VAULT}` → `--vault`, else
`ENIGMA_VAULT_PATH`; when neither exists and a value contains `{VAULT}` → abort
`manifest entry needs {VAULT}: pass --vault or set ENIGMA_VAULT_PATH`.

### 6.2 Config and classification

Config C = `--config`, default `~/.claude.json`. Read C before the manifest:
absent → no servers; present but unreadable (permission denied, a directory,
…) → IoError (below); invalid JSON → abort `cannot parse {C}: {parser one-line reason}`
(the reason text is implementation-defined, one line);
not an object or `mcpServers` not an object → abort
`unexpected structure in {C}: mcpServers must be an object`.

A `--manifest` file is then read: unreadable → IoError; invalid JSON →
SyncError `cannot parse {M}: {parser one-line reason}`; not an object with a
`servers` list (no `servers` key, or a value that is not a list) → SyncError
`{M}: the manifest has no "servers" list`. These IoError and SyncError cases
are not aborts: they print nothing on stdout, end with stderr
`error: {Kind}: {message}` + LF (14 §1; IoError messages per 00 §5) and exit 1.
Nothing runs.

For each resolved entry in manifest order: name not in `mcpServers` →
*missing*; otherwise the first difference, if any, makes it *drifted*:

1. url entries compare only `url`:
   `url: config={quoted(config url)} manifest={quoted(url)}`;
2. else `command`: `command: config={…} manifest={…}`;
3. else `args` (list equality; absent = `[]`): `args: config={…} manifest={…}`;
4. else each manifest env key in order: `env[{K}]: config={…} manifest={…}`.

Values use quoted(); an absent config value renders `null`. Extra env keys in
the config are ignored. No difference → *present*. *extras* = config names not
in the manifest, sorted.

### 6.3 Output and registration

Report lines (each + LF): every present `present : {name}`; every drifted
`DRIFTED : {name} — {diff}` followed by
`  to fix drift manually: claude mcp remove --scope user {name} && re-run this sync`;
every extra `extra   : {name} (not in manifest; left alone)`; every missing (manifest
order) `MISSING : {name} — {why}`.

An abort prints `ABORT (nothing changed): {message}` + LF, exit 2, and runs
nothing.

`--check` stops after the report: exit 1 when anything is missing or drifted,
else 0; it never looks for `claude`.

Otherwise, for missing servers only (drift is never re-registered and does not
fail the run), the add command is
`claude mcp add --scope user --transport http {name} {url}` for url entries, else
`claude mcp add --scope user {name}` + one `--env K=V` pair per env entry (manifest
order) + `--` + command + args. Nothing missing → exit 0 without looking for
`claude`.

1. Look up `claude` on PATH. Not found → print `register: {text}` for every
   pending command, then `claude CLI not found on PATH. Run the printed command(s) manually, or install Claude Code. Nothing was registered.`;
   exit 3.
2. For each pending command: print `register: {text}`, run it with argv[0] =
   the resolved claude path. Spawn fails with not-found → print the register
   lines of the remaining commands, then
   `claude CLI not found on PATH. Run the printed command(s) manually, or install Claude Code.`;
   exit 3. Spawn fails for any other reason (e.g. permission denied) → stop
   at once: stderr `error: IoError: {OS error}: {claude path}` + LF, exit 1;
   later servers are neither printed nor attempted. Exit code ≠ 0 →
   `  FAILED (exit {code}) — continuing with the rest`.
3. Exit 1 if any registration failed, else 0. C is never written by Enigma.

**Printed text** always starts with the word `claude`. POSIX: each word printed
bare when it consists only of `A–Z a–z 0–9 @ % + = : , . / _ -`; the empty word
as `''`; any other word in single quotes with each embedded `'` written as
`'"'"'`. Windows: words joined with the Microsoft C runtime command-line
quoting rules.

**CLI** `enigma-mcp mcp-sync [--check] [--vault V] [--manifest M] [--config C]`:
`--manifest` naming a missing file → usage error
`manifest not found at {M} — pass --manifest pointing at your checkout's mcp-servers.json`
(exit 2); without `--manifest` the built-in manifest is used.

## 7. Provisioning

### 7.1 Plugin lock

Fourteen plugins, in this order (id, repo, version); every lock names
`manifest.json` and `main.js`, and `styles.css` unless marked "no css":

| id | repo | version | manifest.json | main.js | styles.css |
|---|---|---|---|---|---|
| smart-connections | brianpetro/obsidian-smart-connections | 4.5.3 | 6b24c3f9f47dfc12e697946607b197d0e4931a7cbd7a1d445a92b477b4f12ed2 | 11c422b17553b9a09c9d25695ffa91000845920415d6a32e5b0c2a3dbc41e01f | 721936640e608c4a74024bd22a354589acff668d9c688e883781553056c88fd7 |
| obsidian-tasks-plugin | obsidian-tasks-group/obsidian-tasks | 8.2.2 | db6fe0eb4f033955cdae3e545a39f69748c87262ea8b352805662c4ccbcb714b | 5c68dd0f4e1838f3bd263df39aa508d66ed94e85cc4a48bb338170be2955e077 | 32b3d394b697a058f2dcaef0d38476b3c3e585aea63549a816bcc237cf3e3872 |
| obsidian-livesync | vrtmrz/obsidian-livesync | 0.25.79 | d53719a268a82df164d3cc6fd9bff8f6651b59ba528f20506569eddf2248deb3 | 4e207c117d15749312d2b4ef5209b87714cb3a45b267d85356948444ef1bbf63 | 37d31798186d7e97ea979e6d2aae8021ea1ac1df2c3b9d2b03dce269959c27f3 |
| text-extractor | scambier/obsidian-text-extractor | 0.7.0 | ff57053e0491b20f0c5d9becc0600c378d341a00f1fffa6efcb8b0006d2122b3 | 1eec6b986b64717434ce5acec3e7470bc08aafb649aca5f0d33c7ccc720d5eef | no css |
| dataview | blacksmithgu/obsidian-dataview | 0.5.68 | 9235db47112da81b85591c79ecb9ae2574e5e72207056e976472f90616286185 | 794e9eaede73920bb8d54b0eda4f5de2182d698cc638774500f24f14bcd4da0b | 3306dd9032e00f989ba7233a37fd255bc4d3f4340cee661762e952f3f6aa1de9 |
| omnisearch | scambier/obsidian-omnisearch | 1.29.3 | f9b2791c44ea02e6347383cd02719309f59792eb39361f32ccaafc6f49753b7b | d87ceddfe9fc429fe0b64f3d480be488c1e491f2889d24370061d3a2e5afcf4c | 818e6b361e426ab3a868a4524a20df5a6416bf614876b61ede340d0f78c9ebc8 |
| tag-wrangler | pjeby/tag-wrangler | 0.6.4 | 4e58dc8924009b2b9839692fe258d08758512eca6ab54c174f8022c7eda719bb | 94ad72ea45e60629d7a628686f8cf7956a7ad963f43d39f201ab8016d2ee64a2 | no css |
| obsidian-kanban | mgmeyers/obsidian-kanban | 2.0.51 | 24976787097ead467969e014a35654e7a80e4db49a977689a48afadfa15e1854 | a7e3bd4cf25f9b7f53a841c44ce990db0ef5f7954ebcab17ae6dca80310c39ac | ecf6dd31f1727c441cce6f54794b0d3916dcfffc87fa17b855c79ba04a85d9a7 |
| table-editor-obsidian | tgrosinger/advanced-tables-obsidian | 0.23.2 | 698b4f77445e07d887f33450eaf533a28e099b7b483f642fa883362ffbd8ffe9 | cf5dd4ddbddebef68cc99cd93a883e33895c7f123d04bc5d1106ea6e338ba791 | 23fa30d76f117fd3d1624c4c2e6ddedabf809923996b0534895f8254ea6a39f7 |
| obsidian-importer | obsidianmd/obsidian-importer | 1.8.12 | 509541ee1fde6ff544ba4092899ccff73cc525cd507fad1923d652f12dca88a4 | 6fdb295673627c16d3b20471c5850bc1ba073e332a0bb423047781b5be5c6e12 | 0119337488746fe7052f7f9841b92df7150f4e2f64cbc8e82443fa8812a32fe0 |
| extended-graph | ElsaTam/obsidian-extended-graph | 2.7.6 | d02c3be055973d8e79a0d6fe56f9aa32d71ccbb3cf55d0ce83b0f1c64905a772 | 56bf1761dc350fc7d996cc3e286400b5488f7b0e33f26f152e7dbd1670c3c99c | ce754cb01a6435e83c0197f765feee866896556f14cff24166bf399c3a329cef |
| graph-styler | moonweave/obsidian-graph-styler | 0.1.5 | b407127f2d9673537f6bef17daabbafa246cd52b715fd56aeba65ae6d7605318 | c3cfb30b7ac3c5ba7f131fec2b2fa829a249ffc1bd354a41ee59b796c575701f | 6b0c5d76e5888a896bcbf8db04ba38bd5bc8991187adcf7e1f66b9cad3c3d031 |
| translucent-bg | mellowmarsh-git/Obsidian-Translucent-BG | 1.1.2 | 7cc184eb0d9ebb5d25abcfb8bfe0f532c4fdc07a919ff97b1311450298b58ccb | cd0bced878c7f75f39aef7cdc68643036d834faa8401053942a281039ac54626 | 1b6d62d365fbd0a8cb438909c4a795e50d958d4140b18b91600d646038b4f6a9 |
| obsidian-style-settings | mgmeyers/obsidian-style-settings | 1.0.9 | 9cffdc20cf2aa1354820e0050b694b0f9b576446e3d5d44a36ae8b0187e5bfb8 | 1828abaacdab4c5578b705a625c585b30512f8efad4c7cfc5a18e70cc3557468 | ee7937d2be50653a89ccb30ae5f0572b23507d5b7f1328d005271a363075bfd8 |

The lock is stored as a JSON list of `{id, repo, version, sha256: {asset: hex}}`.

**Validation** (ProvisionError; ENTRY = display() of the entry object), in
order: `id`, `repo`, `version` missing or empty →
`plugins.json entry missing '{field}': {ENTRY}`; `sha256` not an object →
`plugins.json entry missing 'sha256' lock: {ENTRY}`; `manifest.json` then `main.js`
absent → `{id}: sha256 lock missing '{name}'`; each lock entry in order: unknown
asset (not manifest.json, main.js, styles.css) →
`{id}: sha256 lock names unknown asset '{name}'`; digest not 64 lowercase hex →
`{id}: sha256 for '{name}' is not 64 lowercase hex chars`.

### 7.2 Fetching and installing

fetch(url) → bytes or absent. Asset URL
`https://github.com/{repo}/releases/download/{version}/{filename}`. The real
fetcher sends User-Agent `enigma-mcp-provisioner`, times out after 30 s, treats
HTTP 404 as absent, other HTTP statuses → `download failed {url}: HTTP {code}`,
transport errors → `download failed {url}: {reason}`.

*installed version* = `version` of `.obsidian/plugins/<id>/manifest.json`
(absent, unreadable or invalid JSON → none).

**install_plugin**: installed version = pin → `ok` (nothing fetched). Otherwise
fetch and verify each locked asset in lock order (assets not in the lock are
never requested); the first failure stops: absent →
`{id}: no {file} in release {version} of {repo}`; digest mismatch →
`{id}: {file} sha256 {actual} does not match lock {expected} — refusing`. After
all verify, the release manifest's `id` must equal the plugin id, else
`{id}: release manifest declares id '{display(declared)}' — refusing`. Only then
create `.obsidian/plugins/<id>/` and write each asset's bytes → `installed`. A
refusal writes nothing.

**enable_plugins(ids)**: existing = `.obsidian/community-plugins.json` list (absent
→ []); added = ids not in existing (order kept); if any, write existing +
added as indented File JSON with no final newline. Return added.

### 7.3 provision(vault, fetch)

1. Vault not a directory → ProvisionError `Vault root does not exist: {vault}`
   before anything is written.
2. Create `.obsidian`. Install each plugin; a ProvisionError is recorded in
   `errors[id]` (message) and the next plugin continues; else
   `plugins[id]` = `installed` / `ok`.
3. enabled = enable_plugins(ids without errors, lock order).
4. overlays = §7.4. conventions = install(vault) (§2).
5. Return `{plugins, errors, enabled, overlays, conventions}`.

**check(vault)** → `{id: {pinned, installed, status}}` in lock order: `ok` when
installed = pinned, `missing` when none (installed null), else `drift`. check
makes no existence check: a vault path that is missing or is not a directory
simply has no installed versions, so every plugin is `missing`.

### 7.4 Overlays

Per-plugin overlays (none are bundled today) would be written to
`.obsidian/plugins/<id>/data.json` only when absent, reported as `<id>/data.json`
in sorted id order. Then SMART ENV: when `.smart-env/smart_env.json` is absent,
create the folder and write exactly this text followed by a single LF, reporting
`.smart-env/smart_env.json`:

```json
{
  "is_obsidian_vault": true,
  "smart_sources": {
    "min_chars": 200,
    "embed_model": {
      "adapter": "transformers",
      "transformers": {
        "model_key": "TaylorAI/bge-micro-v2"
      }
    }
  },
  "models": {
    "embedding_platform": "transformers"
  }
}
```

An existing file is never touched or reported.

### 7.5 CLI

`enigma-mcp provision VAULT --check` prints the check report (indented File JSON
+ LF), changes nothing, exit 0, also when VAULT is missing or is a file (every
plugin `missing`). `enigma-mcp provision VAULT` prints the report
(indented File JSON + LF); when errors is non-empty it then prints an empty
line and `WARNING: some plugins failed — re-run after checking pins/network.`;
it always ends with an empty line, the text below (each line LF-terminated) and
one more empty line; exit 0. Without `--check`, a missing vault → stderr
`error: Vault root does not exist: {vault}` + LF, exit 2.

```text
Provisioning done. Remaining human steps:
 1. Open the vault in Obsidian once and turn OFF Restricted Mode
    (Settings -> Community plugins) so the installed plugins load.
 2. Complete LiveSync setup via its Setup-URI flow — server URI and the
    E2E passphrase live in your password manager, never in this repo.
 3. Register the vault with the MCP server (enigma-mcp mcp-sync) and run the
    index_brain tool to build the semantic graph.
```

## 8. skill-sync

Source = the nine bundled skills; destination D = `--dest`, else
`~/.claude/skills`. Two skill folders are *identical* when the sets of
relative file paths under them and every file's bytes are equal.

For each bundled skill in sorted order: D/<name> absent → copy it (creating
D) unless checking → *installed*; identical → *up_to_date* (nothing
rewritten); else `--force` and not checking → delete D/<name> and copy →
*updated*; else → *drift*. Folders in D that are not bundled skills are never
touched or reported.

**CLI** `enigma-mcp skill-sync [--check] [--force] [--dest D]` prints
`{installed, updated, up_to_date, drift}` (lists of names, sorted) as indented
File JSON + LF. `--check` writes nothing (not even D), reports a drifted skill
under drift even with `--force`, and exits 1 when installed or drift is
non-empty. Other runs exit 0, even with drift.

## 9. Nightly index runner

`enigma-mcp index-nightly VAULT [STATE_DIR]`. No VAULT → stderr
`usage: enigma-mcp index-nightly <vault> [state-dir]` + LF, exit 1, before
anything else (no start line, no lock). LOCK = `$TMPDIR/enigma-index.lock`
(`/tmp/enigma-index.lock` when `TMPDIR` is unset or empty; on Windows the
system temp folder in that case), a directory holding a
file `pid` with the runner's pid and LF. STAMP = second stamp.

1. Environment for the rounds: `ENIGMA_VAULT_PATH` = VAULT; `ENIGMA_STATE_DIR` =
   STATE_DIR when given; `ENIGMA_EXTRACTOR` defaults to `claude` and
   `ENIGMA_EXTRACTOR_NO_TOOLS` to `1` (explicit non-empty values are kept);
   `HF_HUB_OFFLINE` is passed through and never forced.
2. Print `[STAMP] index-nightly start`.
3. VAULT missing or not listable → print
   `cannot read vault (macOS privacy permission for enigma-mcp/launchd?)`; exit 2
   (no lock, no round).
4. Create LOCK atomically. If it exists: pid = content of LOCK/pid (trimmed;
   none if unreadable). pid given and a live process with that pid has
   `index-nightly` in its command line → print
   `another index run (pid {pid}) holds {LOCK}; skipping`, exit 0, LOCK untouched.
   Otherwise print `removing stale lock {LOCK} (pid {pid or unknown} is not running)`,
   remove it and create it again; if that fails print
   `another run took {LOCK}; skipping`, exit 0. Write the pid file. From here on
   LOCK is removed on every exit (success, failed round, interrupt).
5. Another process whose command line contains `enigma-mcp index ` (trailing
   space) is running → print `indexer already running; skipping`, exit 0.
   The process list is `ps -Ao pid=,command=` on POSIX and the `Win32_Process`
   class, read through PowerShell, on Windows; a list that cannot be read
   counts as empty. On Windows a command line is lowercased and has every `"`
   and `.exe` removed before it is searched, so `"C:\…\enigma-mcp.exe" index …`
   matches.
6. Up to 20 rounds: run `EXE index VAULT` (default batch, no flags) capturing
   stdout and stderr together. Non-zero exit (e.g. LockBusy while a Librarian
   sweep holds the caretaker lock) → print the last 5 lines of its output,
   exit 1. Otherwise print the last 12 lines; stop after the first round whose
   output contains `"remaining": 0`.
7. Print `[STAMP] index-nightly done`; exit 0 (also after 20 rounds without
   draining).
8. Interrupts. While LOCK is held, SIGINT, SIGTERM and SIGHUP do not stop the
   runner at once and the runner never signals the round itself. A signal
   sent to the whole process group (Ctrl-C, a supervisor stopping the job)
   also reaches the round, which then ends by itself; a signal sent to the
   runner's pid alone lets the running round finish its batch. Either way,
   when the running round ends (or before the next round would start) the
   runner removes LOCK, prints nothing more (no last-lines block, no done
   line), runs no further round, and terminates by that same signal with its
   default action. Its parent therefore sees death by the signal, not an
   exit code; a shell reports 128 + the signal number (130, 143, 129).
   Windows has no SIGHUP: only SIGINT and SIGTERM are handled there.

### 9.1 launchd templates

Rendered with `{BIN}` = EXE, `{VAULT}`, `{HOME}`, each value XML-escaped
(`&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`). Install only one: the Librarian
sweep already indexes. Only the Librarian template carries the first-run note:
`index-nightly` has no first-run guard (§9.2 warns about it at install time).

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Enigma nightly graph index (no filing, no moves). Install only one of
     com.enigma.index and com.enigma.librarian: the Librarian sweep already
     indexes. Install with: enigma-mcp schedule install --job index VAULT
     (or fill in {BIN}, {VAULT} and {HOME}, copy to
     ~/Library/LaunchAgents/com.enigma.index.plist, then
     launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enigma.index.plist).
     Remove with: enigma-mcp schedule uninstall --job index -->
<plist version="1.0">
<dict>
  <key>Label</key><string>com.enigma.index</string>
  <key>ProgramArguments</key>
  <array>
    <string>{BIN}</string>
    <string>index-nightly</string>
    <string>{VAULT}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>{HOME}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>4</integer><key>Minute</key><integer>30</integer></dict>
  <key>StandardOutPath</key><string>{HOME}/.enigma-mcp/index-nightly.log</string>
  <key>StandardErrorPath</key><string>{HOME}/.enigma-mcp/index-nightly.log</string>
</dict>
</plist>
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Enigma Librarian sweep. Install only one of com.enigma.librarian and
     com.enigma.index: this sweep already indexes. A never-swept vault refuses
     a scheduled run: first run "enigma-mcp librarian <vault> --dry-run" by
     hand, review the report, then run once with --confirm-first-run.
     Install with: enigma-mcp schedule install --job librarian VAULT
     (or fill in {BIN}, {VAULT} and {HOME}, copy to
     ~/Library/LaunchAgents/com.enigma.librarian.plist, then
     launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.enigma.librarian.plist).
     Remove with: enigma-mcp schedule uninstall --job librarian -->
<plist version="1.0">
<dict>
  <key>Label</key><string>com.enigma.librarian</string>
  <key>ProgramArguments</key>
  <array>
    <string>{BIN}</string>
    <string>librarian</string>
    <string>{VAULT}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>ENIGMA_EXTRACTOR</key><string>claude</string>
    <!-- unattended extraction reads clipped web pages: deny the CLI its tools -->
    <key>ENIGMA_EXTRACTOR_NO_TOOLS</key><string>1</string>
    <key>PATH</key><string>{HOME}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>4</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>{HOME}/.enigma-mcp/librarian-task.log</string>
  <key>StandardErrorPath</key><string>{HOME}/.enigma-mcp/librarian-task.log</string>
</dict>
</plist>
```

### 9.2 schedule command

`enigma-mcp schedule ACTION --job JOB [VAULT] [--force] [--dry-run]`, ACTION one
of `install`, `uninstall`, `print`; JOB one of `index` (template
`com.enigma.index`) and `librarian` (`com.enigma.librarian`); the other job is
OTHER. VAULT is required for `install` and `print` (usage error without it)
and ignored by `uninstall`.

Terms: DIR = `{HOME}/Library/LaunchAgents` (HOME as in 01 §2, so a test or
smoke run points HOME at a temp folder); P = `DIR/com.enigma.{JOB}.plist`;
P' = the same for OTHER; LOGS = `{HOME}/.enigma-mcp`; LAUNCHCTL =
`ENIGMA_LAUNCHCTL` when set (non-empty), else `/bin/launchctl`; DOMAIN =
`gui/{uid}` (the real user id); TEXT = the JOB template (§9.1) rendered with
EXE, the vault root resolved as in 01 §3 (missing → the VaultError of 01 §3,
exit 1) and HOME. Printed commands quote each word as in 15 §6.3 (POSIX).

**install** (and `print`, which is `install --dry-run`), in order:

1. *Warnings* (stderr, one line each, both modes; they never stop the command):
   - the resolved vault lies inside `{HOME}/Desktop`, `{HOME}/Documents`,
     `{HOME}/Downloads`, `{HOME}/Library/Mobile Documents`,
     `{HOME}/Library/CloudStorage` (HOME with symbolic links resolved) or
     `/Volumes` →
     `warning: {vault} is in a folder macOS protects ({folder}); a launchd job can read it only after you grant Full Disk Access to {EXE} (System Settings > Privacy & Security > Full Disk Access), otherwise the scheduled run fails with "cannot read vault (macOS privacy permission for enigma-mcp/launchd?)" or a permission error`
     where folder is the matched folder;
   - JOB is `librarian` and the Librarian state file (08, 16 §7) does not exist
     in the state dir the job will use (01 §4 derived from HOME and the vault,
     ignoring `ENIGMA_STATE_DIR`, which the job does not get) →
     `warning: no Librarian sweep has run on this vault yet; the scheduled sweep will refuse until you run "enigma-mcp librarian {vault} --dry-run", review the report, then run "enigma-mcp librarian {vault} --confirm-first-run"`.
2. *Refusals* (stderr `error: {message}` + LF, exit 2, nothing written or run):
   - P' exists →
     `com.enigma.{OTHER} is already installed ({P'}); install only one of the two jobs. Remove it first with: enigma-mcp schedule uninstall --job {OTHER}`;
   - P exists, its bytes differ from TEXT and `--force` is absent →
     `{P} exists and differs from the plist this command would install; re-run with --force to replace it`.
3. P exists with exactly TEXT → stdout `already installed: {P}` + LF, exit 0;
   nothing written or run (also with `--force`).
4. *Plan*: when P exists (a forced replacement) run
   `LAUNCHCTL bootout DOMAIN/com.enigma.{JOB}` first; then create DIR and LOGS
   (launchd does not create the log folder) and write P = TEXT; then run
   `LAUNCHCTL bootstrap DOMAIN P`.
5. `--dry-run` / `print`: stdout TEXT, an empty line, then
   `# would write {P}` and one `# would run: {command}` line per launchctl
   command in plan order (each + LF); exit 0; nothing is created, written or
   run, LAUNCHCTL is never started.
6. Otherwise carry out the plan. A failing `bootout` is ignored (the job may
   not be loaded). `bootstrap` exits non-zero or cannot start →
   `error: launchctl bootstrap failed ({exit N | OS error}){detail}; {P} stays installed and loads at next login; remove it with: enigma-mcp schedule uninstall --job {JOB}`
   (detail = `: ` + the first line of its stderr, trimmed, or nothing when it is empty),
   exit 1. Success → stdout `installed: {P}` and
   `loaded: com.enigma.{JOB} ({DOMAIN})` (each + LF), exit 0.

**uninstall**: P absent → stdout `not installed: com.enigma.{JOB}` + LF, exit 0,
LAUNCHCTL never started (a clean no-op). Otherwise run
`LAUNCHCTL bootout DOMAIN/com.enigma.{JOB}`: exit 0 → stdout
`unloaded: com.enigma.{JOB}`; anything else → stderr
`warning: launchctl bootout failed ({exit N | OS error}){detail}; the job was probably not loaded`.
Then delete P and print `removed: {P}`; exit 0 (a failure to delete P is
IoError, exit 1). `--dry-run` prints `# would run: {command}` and
`# would remove {P}` and changes nothing.

The command never touches any other file in DIR, never edits a plist in
place, and never runs `launchctl` except as above. Tests and smoke runs set
HOME to a temp folder and `ENIGMA_LAUNCHCTL` to a stub that only logs its
arguments.

Enigma ships no Windows scheduled-task file (00 §7). On Windows every ACTION
fails at once, before anything is read, written or run: stderr
`error: schedule manages macOS launchd jobs only; on Windows, schedule "enigma-mcp librarian <vault>" with Task Scheduler`
+ LF, exit 2. A Windows user who wants nightly sweeps can schedule
`EXE librarian VAULT` with Task Scheduler and the same two environment
variables (informative).

## 10. Acceptance map

CV-01…CV-09 → §2; HK-01…HK-06 → §3; HK-10…HK-14 → §4; HS-01…HS-09 → §5;
SK-01…SK-07 → §8; MS-01…MS-16 → §6; PV-01…PV-13 → §7; NT-01…NT-11 → §9; SC-01…SC-09 → §9.2;
PL-01 → 03 §5.2; PL-02 → this file's preamble.
