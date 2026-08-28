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
| `settings.json` | setting | file | <https://pi.dev/docs/latest/settings> | read its bytes |
| `skills` | skill | directory | <https://pi.dev/docs/latest/skills> | read its bytes |
| `extensions` | plugin | directory | <https://pi.dev/docs/latest/extensions> | read its bytes |
| `prompts` | command | directory | <https://pi.dev/docs/latest/prompt-templates> | read its bytes |
| `themes` | *(routes no kind)* | directory | <https://pi.dev/docs/latest/themes> | read its bytes |

**A citation is not a measurement.** `decided by` says where a row came from; `exercised by` says whether anybody made the product demonstrate it. Where a row records no method the answer is a page and nothing else, because absence of a record of measurement is not evidence of measurement.

Here that is **0 run**, **6 read from the product's own bytes**, and **0 resting on a page alone**. The last number is the one worth acting on: a row in it is not wrong, it is untested, and the two are indistinguishable from here.

A surface that routes no kind is owned deliberately: a backup captures
it and a restore returns it, and no component is routed there because
the kind it would carry already routes somewhere else. One kind on two
surfaces makes a consumer's route ambiguous, and the guard in
`harness_runtime::surfaces` refuses it by name.

## Considered and not owned

12 rows. Each records what was searched, so the next reader does not repeat the search:

- **`models.json`** — Named by the consumer's catalog as a second setting surface. The settings documentation does not describe it, and a row nobody can source is not owned.
- **`AGENTS.override.md`** — Pi loads this instead of AGENTS.md or CLAUDE.md from the same directory, so a home holding one ignores the instruction file this provider installs. Not owned, for the reason an override exists at all: it is how a person overrides, and owning it would let `remove` take that away.
- **`NDDEV-PI-PROVIDER.json`** — This provider's own state file: which setup is applied, the identity it recorded, and which slot reverses the last operation. Written by every operation and excluded from target identity, because counting it would leave a target different from the identity the operation just wrote. Not a projection surface and never ownable as one.
- **`.pi-setup-system`** — This provider's own control directory: the target lock, the backup slots and their payloads. Kept out of the declaration for the same reason as the state file, and recorded here because the declined list is where a reader looks before opening a file to find out what it is.
- **`$HOME/.agents/skills`** — Pi is a second product that reads the user-level convention root, and this is measured from the product rather than from a page. In the pinned 0.84.3 bundle, package/dist/core/package-manager.js:1976 builds `userAgentsSkillsDir = join(getHomeDir(), ".agents", "skills")` and line 2017 loads from it: `addResources("skills", collectAutoSkillEntries(userAgentsSkillsDir, "agents"), ...)`. Line 2012 names the root itself as `userAgentsBaseDir = dirname(userAgentsSkillsDir)`. A neighbouring use in trust-manager.js:160 *excludes* this directory while walking up for a project-scoped one, which is what a first reading of the variable name would have mistaken for the read -- so the line that matters is 2017, not 1976.
- **`auth.json`** — Authentication credentials. Pi joins it against its agent directory -- `agentDir, "auth.json"` in the pinned 0.84.3 bundle, beside `keybindings.json`, `models.json` and the owned `settings.json`. Never owned and never captured, for the reason the antigravity list gives: a backup of someone else's credentials is a leak with a schedule. Recorded here because the declaration's `never_touch` is checked against this block.
- **`keybindings.json`** — A keymap file, joined against the agent directory beside the owned `settings.json`. Not owned, for the reason claude's row gives: no component kind describes a keymap.
- **`working-directories`** — One row for `git`, `npm` and `tmp`, joined against the agent directory. Scratch space the product manages for its own operations.
- **`git`** — Package checkouts the product clones and manages: `~/.pi/agent/git/<host>/<path>` for a global install. Its own shipped docs say what happens to anything a person leaves there -- *"When reconciliation changes the checkout, pi resets and cleans the clone"* -- so it is not a surface a setup can promise, and a backup of it would capture somebody else's repository.
- **`npm`** — Where user package installs go, beside the git checkouts above and managed the same way. Declined for the same reason: the product puts content here and takes it away again.
- **`models-store.json`** — A cache of remote model catalogs, persisted so a later run can restore them without a network request, refreshed on a four-hour throttle. A cache the product regenerates is never a configuration surface.
- **`pi-debug.log`** — Written by the hidden `/debug` command and holding rendered TUI lines with ANSI codes. A log, never owned and never captured.
