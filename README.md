# pi-setup-system

Installs, reselects, restores and removes a complete Pi harness configuration, and owns the program lifecycle.

A *setup* here is the complete harness state — the system-prompt components and
the whole configuration — not a pointer to somewhere the content really lives.
That is what makes restore mean something: it returns the instructions, skills,
agents, commands, hooks, MCP entries and settings together, in one step.

> **Status: usable.** The commands below work end to end. What is not here yet
> is reading a `HarnessBundle` over the wire, so `install` and `replace` arrive
> from the local catalog rather than from ai-stp; the wire forms of those two
> refuse and say so. The software lifecycle and `launch` are not declared,
> because this build does not perform them.

## Using it

```bash
pi-setup-system list
pi-setup-system install baseline --target ~/.tool-config
pi-setup-system status              --target ~/.tool-config
pi-setup-system select minimal      --target ~/.tool-config
pi-setup-system diff                --target ~/.tool-config
pi-setup-system reinstall           --target ~/.tool-config
pi-setup-system backups             --target ~/.tool-config
pi-setup-system restore --backup slot-000000000002 --target ~/.tool-config
pi-setup-system remove              --target ~/.tool-config
```

Every command takes an explicit `--target`. There is no default and no fallback
to a configuration home: a change aimed at a guessed path is a change aimed at
someone else's state. The documented home is printed by `--help` so it can be
copied, not resolved.

**A backup is captured before every change**, so `restore` always has something
to return to. `restore` with no reference means the most recent backup that
existed when you asked — not the one the restore itself just took.

**Selecting a setup reaches its complete state, not a merge.** If the setup you
leave owned a file the one you choose does not, that file goes. A target is
always exactly one setup plus whatever this provider never claimed.

Point `PI_SETUP_SYSTEM_SETUP_CATALOG` at a directory to use setups of your own.

## What it manages

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
