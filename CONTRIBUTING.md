# Contributing to Enigma MCP

Thanks for helping. This page says how to build, test and propose a change.

## Build and test

```bash
cargo build --release
```

```bash
cargo test && cargo clippy --all-targets
```

The tests load the real embedding model. Put it in your Hugging Face cache once:

```bash
pip install -U huggingface_hub && hf download TaylorAI/bge-micro-v2 config.json tokenizer.json model.safetensors
```

A clean test run is 504 passing tests and no clippy warnings. CI runs the same on macOS and Ubuntu.

## How a change is made

1. **Specification first.** Describe the behaviour in the matching file under `spec/` and add an acceptance case to
   `spec/acceptance/`.
2. **Then the code**, as small as it can be while still handling errors.
3. **Then a test** in `tests/` named after the acceptance case.
4. One change per commit.

## Rules

- Never point a test, an example or a script at a real vault. Use a temp folder or `spec/fixtures/vault`.
- When you run the binary by hand, set `ENIGMA_VAULT_PATH` and `ENIGMA_STATE_DIR` yourself. A shell opened by an
  agent may already carry values that point at a live vault.
- The Markdown stays the source of truth. Anything derived must be rebuildable from the vault.
- Do not add a dependency for a few lines of code.
- Do not lower an eval floor to make a test pass.
- The `fault-injection` feature is for tests only and must never be enabled in a release build.

## Reporting a problem

Open an issue with the command you ran, what you expected and what happened. Leave out private note content;
a small example vault that shows the problem is ideal.
