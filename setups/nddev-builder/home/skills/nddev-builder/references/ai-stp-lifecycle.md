# The ai-stp CLI lifecycle

Native install, select and restore of this provider are the lifecycle section
of this toolkit. This page is the consumer path: author a component, compose a
setup, install it, read it back, release an immutable version, and request
publication.

Resolve every flag from `ai-stp help --agent --json`. Do not invent options.
Start with `ai-stp doctor --json`.

## Exercise from a blank authoring directory

1. **Scaffold** a real skill with `ai-stp component scaffold plan` then
   `ai-stp component scaffold apply`. Replace every draft marker before
   compose or release.
2. **Passport.** `ai-stp component passport validate` and
   `ai-stp component skill validate` on the package directory (the directory
   with `SKILL.md` at its root), not the whole tree.
3. **Harness semantics.** This harness's surfaces table says where the kind
   lands. Do not invent a path the declaration does not carry. Adaptations are
   those native files, not a second copy of the passport.
4. **Compose a setup.** `ai-stp select propose` then `ai-stp select confirm`,
   or `ai-stp setup compose plan`. Confirm only the proposal just returned.
5. **Install and read back.** `ai-stp install plan`, then `ai-stp install apply`
   with that plan's digest, then `ai-stp target status` with the same provider.
   Trust `pending_authorization`, not the apply exit code.
6. **Immutable release.** `ai-stp component version release`.
7. **Requested publication.** `ai-stp component publish` or
   `ai-stp setup publish plan`. Publicity is a separate user decision.

## What this page does not name

- Private authoring gates, repository coordinates, or unpublished tools.
- Flags other than `ai-stp doctor --json` and `ai-stp help --agent --json`.
