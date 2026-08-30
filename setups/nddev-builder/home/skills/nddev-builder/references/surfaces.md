# What This Harness Owns

Generated from `references/pi-baseline.json` by
`tools/build_nddev_builder.py`. Do not edit: the next render overwrites
it, and the baseline is where a correction belongs.

Every row below was decided by a source, and the source is named. Where
this file and the binary disagree, the binary is right -- ask it with
`pi-setup-system provider-info`.

**Configuration home**: `~/.pi/agent`
**Environment override**: `PI_CODING_AGENT_DIR`

## The configuration file

`settings.json` is **json**, and the parser does not accept comments.

JSON. The vendor documents no comment support and publishes no schema; searched 2026-08-28 and none found on SchemaStore or the vendor's own site.

## Owned surfaces

| path | kinds | shape | decided by | exercised by |
|---|---|---|---|---|
| `AGENTS.md` | instruction | file | <https://pi.dev/docs/latest/sdk> | read its bytes |
| `settings.json` | setting | file | <https://pi.dev/docs/latest/settings> | **ran it** |
| `skills` | skill | directory | <https://pi.dev/docs/latest/skills> | read its bytes |
| `extensions` | plugin | directory | <https://pi.dev/docs/latest/extensions> | read its bytes |
| `prompts` | command | directory | <https://pi.dev/docs/latest/prompt-templates> | read its bytes |
| `themes` | *(routes no kind)* | directory | <https://pi.dev/docs/latest/themes> | read its bytes |

**A citation is not a measurement.** `decided by` says where a row came from; `exercised by` says whether anybody made the product demonstrate it. Where a row records no method the answer is a page and nothing else, because absence of a record of measurement is not evidence of measurement.

Here that is **1 run**, **5 read from the product's own bytes**, and **0 resting on a page alone**. The last number is the one worth acting on: a row in it is not wrong, it is untested, and the two are indistinguishable from here.

A surface that routes no kind is owned deliberately: a backup captures
it and a restore returns it, and no component is routed there because
the kind it would carry already routes somewhere else. One kind on two
surfaces makes a consumer's route ambiguous, and the guard in
`harness_runtime::surfaces` refuses it by name.

## A second target: `target_scope: user_root`

Rooted at `~/.agents`, which is **not** this product's configuration
home. A consumer reaches it by naming the scope on the request, and
every path below is relative to that root rather than to the home
above -- writing the root into the path again would nest it twice.

| path | routes | shape | decided by | exercised by |
| --- | --- | --- | --- | --- |
| `skills` | skill | directory | measured from the pinned bundle, digest verified before reading (pi 0.84.4, package/dist/core/package-manager.js) | read its bytes |

**Under a scope the namespace is the permission and the recorded
files are the inventory.** A root like this one is read by several
products at once, so `remove`, the capture and a restore all act on
the files this provider recorded writing -- never on the namespace
whole, which would take or revert a neighbour's work.

## Considered and not owned

16 rows. Each records what was searched, so the next reader does not repeat the search:

- **`.pi-setup-system`** — This provider's own control directory: the target lock, the backup slots and their payloads. Kept out of the declaration for the same reason as the state file, and recorded here because the declined list is where a reader looks before opening a file to find out what it is.
- **`AGENTS.override.md`** — Pi loads this instead of AGENTS.md or CLAUDE.md from the same directory, so a home holding one ignores the instruction file this provider installs. Not owned, for the reason an override exists at all: it is how a person overrides, and owning it would let `remove` take that away.
- **`NDDEV-PI-PROVIDER.json`** — This provider's own state file: which setup is applied, the identity it recorded, and which slot reverses the last operation. Written by every operation and excluded from target identity, because counting it would leave a target different from the identity the operation just wrote. Not a projection surface and never ownable as one.
- **`auth.json`** — Authentication credentials. Pi joins it against its agent directory -- `agentDir, "auth.json"` in the 0.84.3 bundle, beside `keybindings.json`, `models.json` and the owned `settings.json`. Never owned and never captured, for the reason the antigravity list gives: a backup of someone else's credentials is a leak with a schedule. Recorded here because the declaration's `never_touch` is checked against this block.
- **`git`** — Package checkouts the product clones and manages: `~/.pi/agent/git/<host>/<path>` for a global install. Its own shipped docs say what happens to anything a person leaves there -- *"When reconciliation changes the checkout, pi resets and cleans the clone"* -- so it is not a surface a setup can promise, and a backup of it would capture somebody else's repository.
- **`keybindings.json`** — A keymap file, joined against the agent directory beside the owned `settings.json`. Not owned, for the reason claude's row gives: no component kind describes a keymap.
- **`managed-config`** — Not a path in the target, and recorded because **there is no such path** -- an absence somebody has to measure once so the next reader does not spend the search again. Four of the seven harnesses here carry a system-wide managed policy that overrides everything a user writes; this product carries none.
- **`models-store.json`** — A cache of remote model catalogs, persisted so a later run can restore them without a network request, refreshed on a four-hour throttle. A cache the product regenerates is never a configuration surface.
- **`models.json`** — Named by the consumer's catalog as a second setting surface. The settings documentation does not describe it, and a row nobody can source is not owned.
- **`npm`** — Where user package installs go, beside the git checkouts above and managed the same way. Declined for the same reason: the product puts content here and takes it away again.
- **`pi-debug.log`** — Written by the hidden `/debug` command and holding rendered TUI lines with ANSI codes. A log, never owned and never captured.
- **`trust.json`** — A person's saved decisions about which project folders may load project-local settings and resources and **execute project extensions**. The product's own shipped documentation names it five times and is explicit about what turns on it: *"pi asks before trusting a project folder that contains project-local settings, resources, or project `.agents/skills` and has no saved decision for the folder or a parent folder in `~/.pi/agent/trust.json`. Trusting a project allows pi to load `.pi/settings.json` and `.pi` resources, install missing project packages, and execute project extensions."*
- **`working-directories`** — One row for `git`, `npm` and `tmp`, joined against the agent directory. Scratch space the product manages for its own operations.
- **`mcp_config.json`** — **The vendor says this product has none.** Its own shipped documentation, `usage.md`, under the pinned 0.84.4 bundle's own `package/docs/`, and confirmed against the live page 2026-08-29: *"It intentionally does not include built-in MCP, sub-agents, permission popups, plan mode, to-dos, or background bash. You can build or install those workflows as extensions or packages…"*
- **`agents`** — No sub-agents, from the same sentence as the MCP row above: *"It intentionally does not include built-in MCP, **sub-agents**, permission popups…"* The `agent` kind is therefore not declared for this harness, and nothing under this home is read as one.
- **`hooks.json`** — Hooks here are an **extension API concept, not a configuration surface**. `extensions.md`, under the pinned bundle's own `package/docs/`, documents `session_start`, a `spawnHook` around tool execution and session-scoped teardown hooks -- all of them functions inside an extension module. There is no `hooks.json` and no `hooks` key in `settings.json`, so a hook reaches this product through `extensions/`, which is owned and routes `plugin`.
