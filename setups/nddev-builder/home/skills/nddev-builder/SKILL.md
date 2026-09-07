---
name: nddev-builder
description: Create, improve or review a complete Pi Coding Agent setup -- a native collection of tools for the user's tasks. Use for selecting and authoring components, composing setups, explaining their capabilities, adapting them to this harness, and validating installation and recovery through pi-setup-system.
---

# NDDev Builder

Build a complete native tool collection for the user's tasks. Start with
`references/ai-stp-lifecycle.md` for outcome, component selection, composition,
evaluation, installation and delivery. Keep changes target-explicit and
reversible. The provider's implementation is changed only when that is the task.

## Workflow

1. **Name the user outcome and required capabilities.** Inventory and reuse
   existing components, then compose one setup for this harness through
   `references/ai-stp-lifecycle.md`.
2. **Name the surface being changed**, and check this harness actually owns it:
   `references/surfaces.md`, which is generated from the baseline rather than
   written beside it.
3. **Prefer what the program answers over a copy of it.** Ask the binary:
   `list`, `status --target <dir>`, `provider-info`. In a checkout, read
   `crates/pi-setup-system/src/main.rs` and the baseline a test binds it to.
4. **Declare against the vendor, never against a routing table.** A path with no
   page behind it is a false statement in `provider-info`, and the consumer
   plans postconditions and target identity from that statement.
5. **A declaration can refute a route and cannot confirm one.** Reading finds a
   directory; only running the product says what it is read *as*. Where a run is
   impossible, confirm at the line in the product's own code -- a path literal
   alone is not evidence that the path is used.
6. **Exercise the setup's acceptance scenarios and recovery.** For provider
   implementation changes also run `references/validation.md`. Report observed
   results, exact versions and unmeasured cases.

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

