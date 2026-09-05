---
name: nddev-builder
description: Build, review or validate a Pi Coding Agent setup for pi-setup-system -- its owned surfaces, the components it carries, the lifecycle it performs, and the checks it must pass. Use when changing pi-setup-system or the native artifacts a setup writes.
---

# NDDev Builder

The entry point for work on `pi-setup-system`. Keep changes
target-explicit, reversible, and backed by this tree's checks.

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
5. **Run the checks in `references/validation.md`**, and report what each one
   said rather than that it passed.

## Routing

- **What this harness owns, declines, and why** — `references/surfaces.md`
- **The commands, the invariants, and the software half** — `references/lifecycle.md`
- **The ai-stp CLI lifecycle: scaffold, compose, install, release, publish** — `references/ai-stp-lifecycle.md`
- **The checks this tree's CI runs, a disposable lifecycle smoke, and the consumer** — `references/validation.md`
- **Writing this harness's configuration file** — `references/authoring-settings.md`
- **Writing this harness's instruction file** — `references/authoring-instructions.md`
- **The second target this harness declares, and how a component reaches it** — `references/second-target.md`
- **Writing a skill this harness will actually load** — `references/authoring-skills.md`
- **Writing a command this harness will actually load** — `references/authoring-commands.md`
- **Writing a plugin this harness will actually load** — `references/authoring-plugins.md`

## Boundaries

- **The published trees are rendered, never authored.** Fix the source and the
  renderer; a hand edit to a public tree is overwritten by the next render and
  the check that would have caught it says nothing about why.
- **`provider-kit/` is vendored and byte-bound.** It is never edited here; a
  problem in it is an issue on the consumer's repository.
- **Own a path only with its companions.** Owning one half of a pair the product
  reads together is worse than owning neither -- a signed policy without its
  signature reads as tamper evidence, and the product refuses the session.

