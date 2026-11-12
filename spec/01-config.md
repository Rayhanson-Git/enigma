# 01 — Configuration

Enigma is configured only through environment variables and command-line
arguments. There is no configuration file and no built-in vault location.

## 1. Truthy values

A variable described as **truthy** is on when its value, after strip (00 §3.3)
and ASCII lowercasing, is one of `1`, `true`, `yes`, `on`. Any other value,
including empty and unset, is off.

## 2. Environment variables

| Variable | Read | Default | Effect | Spec |
|---|---|---|---|---|
| `ENIGMA_VAULT_PATH` | first vault access in the server; at use in CLIs and hooks | none (required) | Vault root folder. | §3 |
| `ENIGMA_STATE_DIR` | every state-dir lookup | derived from the vault path | Overrides the state dir location. | §4 |
| `ENIGMA_AGENT_NAME` | every note write that stamps an agent | `claude` | Agent identity stamped on notes and sheet rows. | §5 |
| `ENIGMA_READONLY` | once, at server start | off | Truthy: the read-only tool profile. | §6 |
| `ENIGMA_NO_TOOL_APPLY` | at each tool call | off | Truthy: MCP calls may not trigger caretaker apply actions. | §7 |
| `ENIGMA_MAX_MOVES` | at each `organize_vault(apply=true)` call | `0` | Most notes one such MCP call may move; not an integer or negative counts as 0. The CLI is not affected. | 02 §5.3 |
| `ENIGMA_EXTRACTOR` | when an extractor is built | `codex` | Extractor CLI backend: `codex` or `claude`. | 06 |
| `ENIGMA_EXTRACTOR_NO_TOOLS` | when each extractor invocation's command is built | on | The extractor CLI is denied its tools unless this is `0`, `false`, `no` or `off`. | 06 |
| `ENIGMA_EXTRACT_MODEL` | when the extraction extractor is built | `haiku` | Model for entity extraction (claude backend only). | 06 |
| `ENIGMA_CONSOLIDATE_MODEL` | when the consolidation extractor is built | `sonnet` | Model for duplicate adjudication (claude backend only). | 06, 07 |
| `ENIGMA_AUTO_CLEANUP` | at sweep start when no CLI flag decides | off | Truthy: the Librarian applies mechanical graph repairs. | 08 |
| `ENIGMA_OWNER` | when conventions are installed without an owner argument | `the vault owner` | Owner name placed in the root guides. | 15 |
| `ENIGMA_LAUNCHCTL` | each launchctl command of `schedule` | `/bin/launchctl` | Path of the launchctl binary that `schedule` runs; tests point it at a stub. | 15 §9.2 |
| `CLAUDE_SETTINGS_PATH` | hook-sync | `~/.claude/settings.json` | Claude Code settings file that hook-sync inspects or edits. Name kept because it is that tool's file. | 15 |
| `HF_HOME`, `HF_HUB_CACHE`, `HF_HUB_OFFLINE` | model load | Hugging Face defaults | Model cache location and offline mode. | §8 |
| `HOME` (POSIX), `USERPROFILE` (Windows) | state-dir derivation, default paths | — | The user's home directory. | §4 |
| `TMPDIR` | nightly index runner | `/tmp` | Parent of the `enigma-index.lock` directory. | 14 |
| `PATH` | extractor and sync commands | — | Executable lookup. | 06, 15 |

An empty value is treated exactly like an unset variable for every
`ENIGMA_*` variable except three, where an
empty value is a value: `ENIGMA_EXTRACTOR` (empty → ExtractorError
`Unknown backend: `), `ENIGMA_EXTRACT_MODEL` and `ENIGMA_CONSOLIDATE_MODEL`
(empty → claude runs without `--model`) (06 §5.4, §5.8).

## 3. Vault path

- The server does not need the vault to start. The first tool call that needs
  the vault reads `ENIGMA_VAULT_PATH`.
- Unset or empty: the call fails with VaultError
  `ENIGMA_VAULT_PATH is not set; point it at the vault folder.`
- Otherwise the path is made absolute (relative to the process working
  directory), symbolic links are resolved, and the result must be an existing
  directory; if not, the call fails with VaultError
  `Vault root does not exist: {root}` where root is the resolved absolute path.
- On success the resolved root is cached for the life of the process. A
  failure is not cached: the next call reads the variable again.
- On Windows the resolved path is rendered without the `\\?\` verbatim
  prefix, with backslash separators.
- CLI subcommands take the vault as an argument (14) and apply the same
  resolution and the same "does not exist" error.

## 4. State directory

The state dir holds everything derived (16). It is computed on every use:

1. If `ENIGMA_STATE_DIR` is set, that path is the state dir as given
   (relative paths are relative to the working directory).
2. Otherwise a vault root is required: the vault root of the operation, or,
   for callers that have none, `ENIGMA_VAULT_PATH`. With neither, fail with
   VaultError
   `Cannot determine state directory: pass vault_root or set ENIGMA_VAULT_PATH.`
3. The vault root is resolved to its canonical absolute path (as §3; a path
   that does not exist is made absolute lexically instead of failing). Its
   text, UTF-8 encoded, is hashed with SHA-256; the **digest** is the first
   12 lowercase hex characters.
4. The state dir is `<home>/.enigma-mcp/<digest>`.

In every case the directory and its missing parents are created before the
path is returned. Two different vault roots therefore get different state dirs
under the same parent, and the same root always gets the same dir.

## 5. Agent name

The agent name is computed on each use: take `ENIGMA_AGENT_NAME`, strip it;
if the result is non-empty the agent name is safe_filename of it (03 §7.1),
otherwise `claude`. Examples: ` scout ` → `scout`; `a/b` → `ab`; unset →
`claude`. Each agent runs its own server process, so identity is per process.

## 6. Tool profiles

Exactly one profile is active for the life of a server process. It is chosen
at start-up:

| Condition | Profile |
|---|---|
| `ENIGMA_READONLY` is truthy | read-only |
| otherwise | full |

There is no other profile. Enigma reads no
`ENIGMA_TOOL_PROFILE` variable: whatever value it holds is ignored and never
causes a start-up error.

Tools outside the active profile are not registered at all: they are absent
from `tools/list` and calling them yields the unknown-tool error (02 §3).
The membership tables are in 02 §4. The profile also appends a paragraph to
the server instructions (02 §2).

## 7. Refused tool apply

When `ENIGMA_NO_TOOL_APPLY` is truthy at the moment of a call, these calls
fail before doing anything (no lock taken, nothing read or written):

| Call | {what} | {cli} |
|---|---|---|
| `organize_vault` with `apply=true` | `organize_vault(apply=True)` | `enigma-mcp organize <vault>` |
| `index_brain` with `force=true` | `index_brain(force=True)` | `enigma-mcp index <vault> --force` |
| `consolidate_graph` with `apply=true` | `consolidate_graph(apply=True)` | `enigma-mcp consolidate <vault> --apply` |

The error is VaultError with this message template (one line; the two
backticks around {cli} are literal characters of the message):

```text
{what} is disabled for MCP calls on this server (ENIGMA_NO_TOOL_APPLY). Ask the human to run `{cli}` after reviewing the dry-run.
```

For example, for organize:

```text
organize_vault(apply=True) is disabled for MCP calls on this server (ENIGMA_NO_TOOL_APPLY). Ask the human to run `enigma-mcp organize <vault>` after reviewing the dry-run.
```

(The `<vault>` text is literal; it is not replaced by the vault path.) Dry
runs and `index_brain(force=false)` are never refused.

## 8. Embedding model

Enigma embeds text with the sentence-embedding model
**`TaylorAI/bge-micro-v2`**, the model Smart Connections uses, so local and
Smart Connections vectors share one space. Vectors from any other model MUST
NOT be mixed in. The model key string `TaylorAI/bge-micro-v2` is also the key
under which Smart Connections stores vectors (04 §6).

Facts confirmed from the model's own files (snapshot
`3edf6d7de0faa426b09780416fe61009f26ae589`):

| Item | Value | Source file |
|---|---|---|
| Pipeline | Transformer (module 0) then Pooling (module 1, folder `1_Pooling`); no Normalize module | `modules.json` |
| Architecture | BERT (`BertModel`), 3 layers, hidden size 384, 12 heads, intermediate 1536, GELU, layer-norm epsilon 1e-12, absolute positions, 512 positions, vocabulary 30522, type vocabulary 2, float32 weights | `config.json` |
| Pooling | mean over tokens (`pooling_mode_mean_tokens: true`; CLS, max, mean-sqrt-len, weighted-mean, last-token all false); dimension 384 | `1_Pooling/config.json` |
| Max sequence length | 512 tokens including `[CLS]` and `[SEP]` | `sentence_bert_config.json` (`max_seq_length: 512`) |
| Extra lowercasing by the pipeline | none (`do_lower_case: false` at pipeline level) | `sentence_bert_config.json` |
| Tokenizer | BERT WordPiece, `do_lower_case: true`, `do_basic_tokenize: true`, `tokenize_chinese_chars: true`, `strip_accents: null` (so accents are stripped because lowercasing is on), truncation on the right, padding on the right with `[PAD]` (id 0) | `tokenizer_config.json`, `tokenizer.json`, `vocab.txt` |
| Special tokens | `[PAD]` 0, `[UNK]` 100, `[CLS]` 101, `[SEP]` 102, `[MASK]` 103 | `added_tokens.json` |
| Normalisation of the output | **none**: vectors are not L2-normalised | `modules.json` (no Normalize module) |

Embedding procedure for one text:

1. Strip the text (00 §3.3).
2. Tokenize with the tokenizer above: `[CLS]` + WordPiece tokens + `[SEP]`,
   truncated on the right so the total is at most 512 tokens; token type ids
   all 0; attention mask 1 for every real token.
3. Run the BERT encoder; take the last hidden states.
4. Mean-pool: sum the hidden states of positions whose attention mask is 1 and
   divide by the number of such positions (at least 1e-9).
5. The result is 384 float32 values, returned without normalisation.

Batching (any batch size, padding to the longest text in the batch) MUST NOT
change the per-text result beyond float rounding noise. Vectors are stored and
compared as the float32 values widened to 64-bit floats.

Loading:

- Model files are read from the Hugging Face hub cache
  (`$HF_HUB_CACHE`, else `$HF_HOME/hub`, else `~/.cache/huggingface/hub`),
  repository folder `models--TaylorAI--bge-micro-v2`, using the snapshot named
  by `refs/main`.
- If the files are absent and `HF_HUB_OFFLINE` is not truthy, Enigma MAY
  download them from the Hugging Face hub into that cache.
- The server loads the model before it starts serving MCP requests (warm
  start). If loading fails, the server writes one line to stderr starting
  with `enigma-mcp: cannot load embedding model TaylorAI/bge-micro-v2: ` and
  exits with status 1.
- CLI commands that embed (index, librarian, organize, consolidate, evals)
  load the model when they first need it.

## 9. MCP identity

| Item | Value |
|---|---|
| Transport | stdio, newline-delimited JSON-RPC 2.0 |
| Server name (`serverInfo.name`) | `enigma` |
| Server version (`serverInfo.version`) | the Cargo package version |
| Protocol versions | MUST accept `2024-11-05`; MAY accept newer ones |
| Capabilities | tools only |
| Instructions | the text of 02 §2 |
