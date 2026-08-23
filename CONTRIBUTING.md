# Contributing

## Before a change

```bash
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs exactly these, on Linux, macOS and Windows.

## What the review looks for

- **A new guard is observed failing first.** A test that has never failed on the
  defect it describes has not been shown to detect it.
- **No fact is restated that the code or the provider kit owns.** Point at the
  owner instead; a vocabulary copied into prose diverges silently.
- **Refusals are typed.** A new failure mode gets a stable reason code, not a
  string only a human can read.
- **Nothing reaches a target except through `setup-core`.**

## Commits

Conventional Commits. The subject says what changed and why it matters, not
which files moved.

## Language

English, in code, comments, documentation and commits.
