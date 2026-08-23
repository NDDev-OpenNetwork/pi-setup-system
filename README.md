# pi-setup-system

Installs, reselects, restores and removes a complete Pi harness configuration, and owns the program lifecycle.

A *setup* here is the complete harness state — the system-prompt components and
the whole configuration — not a pointer to somewhere the content really lives.
That is what makes restore mean something: it returns the instructions, skills,
agents, commands, hooks, MCP entries and settings together, in one step.

> **Status: skeleton.** The kernel is implemented and tested. The provider wire
> surface and the human commands are not yet wired to it, and the binary says so
> rather than reporting a capability it does not have.

## What it manages

| | |
| --- | --- |
| Product | Pi Coding Agent (Earendil Works) |
| Documented configuration home | `~/.pi/agent` |
| Environment override | `PI_CODING_AGENT_DIR` |
| Configuration lifecycle | owned |
| Program lifecycle | owned |

The configuration home above is documentation. Every mutation takes an explicit
absolute `--target`; nothing is inferred from a home directory or the working
directory.

## The two surfaces

This is one binary with two callers.

**Provider (ai-stp protocol v3).** `provider-info`, `validate-bundle`,
`plan-operation`, `apply-operation`, `recover-operation`, `status`, and `launch`
where the capability is declared. The vocabulary is owned by
`provider-kit/v3/manifest.json`, vendored here and verified against its
`SHA256SUMS`.

**Human.** `list`, `status`, `install`, `reinstall`, `select`, `backups`,
`restore [--backup <ref>]`, `remove`, `diff`.

Both go through `crates/setup-core`. A human command that reached the target
directly would bypass the guarantees the wire surface owes its consumer, so it
does not exist.

## How a mutation is made safe

```text
resolve target -> acquire lock -> re-check preconditions
  -> journal(prepared) -> capture backup -> stage -> promote
  -> journal(committed) -> verify -> clear
```

Each step is durable before the next begins, so an interrupted mutation leaves
evidence rather than ambiguity:

- a journal in `prepared` means the effect may be partial — recovery restores
  the exact pre-operation target;
- a journal in `committed` means the effect is complete — recovery verifies the
  result and clears the tails.

While any journal, transaction directory or half-written backup slot is present,
planning refuses with `recovery_required` instead of guessing. Only
`recover-operation` may resolve that state.

## Restore

`restore` with no reference restores the most recent backup. `restore --backup
<ref>` restores a chosen one. Slots are numbered by a monotonic sequence rather
than a timestamp, so "the last backup" does not change meaning when a clock does.

## Building

```bash
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all --check
```

The toolchain is pinned in `rust-toolchain.toml`. CI runs the same commands on
Linux, macOS and Windows.

## Licence

AGPL-3.0-or-later. See `LICENSE`.
