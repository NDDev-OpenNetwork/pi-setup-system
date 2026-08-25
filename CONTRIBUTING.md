# Contributing

## Before a change

```bash
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs exactly these, on Linux, macOS and Windows.

## The toolchain, and one way it goes wrong quietly

`rust-toolchain.toml` pins the compiler, and rustup honours it — but only if the
`cargo` you run is rustup's shim. A `cargo` from somewhere else earlier on
`PATH` does not read that file, so it builds with whatever version it happens to
be.

That produces a failure worth recognising, because it looks like a defect in the
code and is not:

```
error[E0514]: found crate `serde` compiled by an incompatible version of rustc
```

It happens when `cargo` and `rustdoc` resolve to different releases — a
distribution or a package manager can install one and not the other. Check with:

```bash
command -v cargo rustc rustdoc
cargo --version && rustdoc --version
```

If they disagree, put rustup's shims first for the shell you build in:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

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
