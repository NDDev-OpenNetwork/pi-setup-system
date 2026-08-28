---
name: nddev-builder
description: Build, review or validate a Pi Coding Agent setup for pi-setup-system -- its owned surfaces, the components it carries, the lifecycle it performs, and the gate it must pass. Use when changing pi-setup-system or the native artifacts a setup writes.
---

# NDDev Builder

The entry point for work on `pi-setup-system`. Keep changes
target-explicit, reversible, and backed by the repository's own gate.

## Workflow

1. **Name the surface being changed**, and check this harness actually owns it:
   `references/surfaces.md`, which is generated from the baseline rather than
   written beside it.
2. **Prefer what the program answers over a copy of it.** Ask the binary:
   `list`, `status --target <dir>`, `provider-info`. In a checkout, read
   `crates/pi-setup-system/src/main.rs` and the baseline a test binds it to.
3. **Declare against the vendor, never against a routing table.** A path with no
   page behind it is a false statement in `provider-info`, and the consumer
   plans postconditions and target identity from that statement.
4. **A declaration can refute a route and cannot confirm one.** Reading finds a
   directory; only running the product says what it is read *as*. Where a run is
   impossible, confirm at the line in the product's own code -- a path literal
   alone is not evidence that the path is used.
5. **Run the gate**, and the render check too when the output could have moved.
   See `references/validation.md`.

## Routing

- **What this harness owns, declines, and why** — `references/surfaces.md`
- **The commands, the invariants, and the software half** — `references/lifecycle.md`
- **The gate, the render check, and the one rule** — `references/validation.md`

## Boundaries

- **The published trees are rendered, never authored.** Fix the source and the
  renderer; a hand edit to a public tree is overwritten by the next render and
  the check that would have caught it says nothing about why.
- **`provider-kit/` is vendored and byte-bound.** It is never edited here; a
  problem in it is an issue on the consumer's repository.
- **Own a path only with its companions.** Owning one half of a pair the product
  reads together is worse than owning neither -- a signed policy without its
  signature reads as tamper evidence, and the product refuses the session.

