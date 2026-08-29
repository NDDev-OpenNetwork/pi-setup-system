# Support

## Before opening anything

`--help` states what this build does and does not do. `status --target <dir>
--json` reports what it found in a target without changing it, and its output is
safe to share: it carries identities and digests, never secret values.

## Where to go

| You have | Go to |
| --- | --- |
| A defect | [Issues](../../issues) — use the defect template |
| A question about behaviour | [Issues](../../issues) — a blank issue is fine |
| A vulnerability | [Security advisories](../../security/advisories/new), privately |

Never open a public issue for a vulnerability, and never paste credentials,
tokens, or the contents of a backup slot anywhere in this repository. A backup
slot holds whatever the target held when it was captured.

## What this build does, and what it does not

The software lifecycle — installing, updating and removing the product
itself — is declared and does work. `plan` names the exact bytes offline,
whoever holds the network fetches them, and `apply` verifies and installs
with the network gone.

`launch` is declared. It starts the exact executable a software install
placed under `--prefix`, never a name found on `PATH`, and points the
product at `--target` through the environment variable its own
documentation names.

A provider that advertised an operation it cannot perform would let a caller ask
for something that cannot be honoured, which is worse than not offering it.

All five core operations do work: `backup`, `restore`, `remove`, `install` and
`replace`, both from the local setup catalog and from an `ai-stp-bundle/1`
arriving over the wire.

## Using this against a home you already have

**An owned namespace is removed whole.** The table below says what this build
owns; `remove` deletes each of those paths entirely, and a backup slot holds
what was there first. That includes content this build never wrote -- if the
product itself put a key in a configuration file this provider owns, `remove`
takes the file, not the keys this provider added to it.

Measured, with the real product: launching Codex through `launch` and running
`mcp add` writes `~/.codex/config.toml` with an `[mcp_servers.*]` entry; a
later `install` captures that file into a slot and replaces it; a later
`remove` deletes it. The entry is not lost -- `backups` lists the slot as
*before install, setup none*, and restoring it returns the file byte for byte
-- but it is not in the target either.

So: point `--target` at a home you are willing to have managed. `backups
--target <dir>` names every earlier state and which setup each preceded, and
`restore --backup <ref>` returns any of them exactly.

## When conformance says this provider is malformed

`ai-stp provider conformance --protocol-version 3` reports each case by name.
If the one that fails is `provider_info_v3_closed`, with a detail about fields
differing from the closed schema, **check the version of the checker before
suspecting this build**.

The v3 capability schema is compared as an exact field set, so a provider that
declares a field the checker predates is reported as malformed rather than as
newer. `scoped_projection_profiles` (`ADR-0125`) is the field this applies to,
and it is omitted entirely when empty -- so a build that declares no scope
satisfies an older checker by accident, and a build that declares one does not.

Two versions, two different answers, both measured:

| checker | result |
| --- | --- |
| `ai-stp-cli` 0.0.3 | five pass; Codex and Antigravity report `conforms=false`, detail *fields differ from the closed v3 schema* |
| `ai-stp-cli` 0.0.7 | six pass 23 of 23; Codex reports `conforms=false`, detail *a scoped projection profile names an unknown target scope* |
| `ai-stp-cli` 0.0.8 | **all seven pass**, 27 to 29 cases each |

The middle row was never a defect in this build, and the third row is how that
was settled: **it closed with no change on this side.** `0.0.7` carried the
field but its scope enum was `["project"]` alone, while the provider kit this
program vendors and verifies byte-for-byte gave `["project", "user_root"]`. The
kit is the artifact a provider is told to build against, so a build declaring
`user_root` was right by the document it was handed and wrong by the checker
shipped beside it. `0.0.8` shipped the enum, and a declaration that had been
correct for a month started being read as correct.

**Withdrawing a correct declaration to make a lagging instrument print green is
never the answer here.** The three rows above are the argument for that, and
they are also the argument for the rule this section exists for.

Which is the general rule this section exists for: **check the version of the
checker before suspecting this build**, and prefer the newest, because an older
one reports a wider failure than the one it found.

## What `status` reports, and what it does not

`state` answers **who manages this target**, and never *whether a setup is
installed*. Three values, and the distinction matters most for the fourth
situation, which is not a fourth value:

| | |
| --- | --- |
| `missing` | the directory is empty |
| `unmanaged` | it holds content, none of it this provider's |
| `managed` | this provider's state file is present and current |

`missing` used to be looser -- it asked whether this provider owned anything,
so a directory full of another product's files reported `missing`. A consumer
reads this to decide what it is looking at, and being told a populated
directory is empty invites it to treat the place as free. Emptiness is about
the directory, not about us.

**After a `remove`, `state` stays `managed`, and that is the honest answer.**
The setup is gone -- no file a product reads survives it -- but the control
directory and a backup slot remain, and that slot is what makes the removal
reversible: `restore` brings the setup back. A target reported as `missing`
while a restore is pending would be a lie in the direction that costs someone
their data.

Whether a setup is installed is carried by `setup_stable_id`, which is `null`
exactly when none is. That is the field to test, not this word.
`target_identity_digest` corroborates it -- after a remove it is the digest of
an empty tree -- but the field is the direct answer and the digest is not.

## The network, stated exactly

**This artifact does not link the network, and no local phase can spawn
anything that could.** Two lints hold it rather than a promise: `std::net` is
refused outright, and `std::process::Command` is refused everywhere but two
named places -- the `launch` command, which is declared in `provider-info` and
absent from builds that do not declare it, and a lifecycle probe that drives
this binary's own executable. Adding a `tar` shell-out to ordinary code fails
the build with *only `launch` may spawn, and it is declared*. Every crate that
may be linked is named in `deny.toml`, so a transitive dependency cannot arrive
unread.

Those are claims about the source, and a lint can be wrong, bypassed, or simply
disbelieved. So `ci` reads the shipped binary too: a `boundary` job asks the
import table of the artifact this build produces whether any network symbol is
present, and whether a build declaring no `launch` imports anything that could
spawn. You can run it yourself against a downloaded release --
`nm -D --undefined-only <binary>` on Linux, `nm -u` on macOS -- and it needs no
part of this repository to be trusted.

**What that does not buy, said plainly because the stronger claim is the
tempting one.** This is a dynamically linked program: it imports `syscall` from
libc like any other, so no property of the binary can prove a socket is
unreachable to code that is determined to open one. What is proven is narrower
and still worth having: no code path here reaches for the network, none can be
added without the build refusing, and no local phase can hand the job to a
child process. If your threat model needs the guarantee rather than the
absence, run `plan` and `apply` under whatever sandbox you already trust; both
phases are offline by design, and `apply` verifies the digests it was given
with the network gone.

## What this build owns inside a target

Everything else in the target is a sibling overlay and is preserved
verbatim. Each row cites the vendor page it was read from, and the same
table is bound to the declaration by a test, so this cannot drift from
what `provider-info` publishes.

Configuration home as the product documents it: `~/.pi/agent`.

| Path | Component kinds routed here | Decided by |
| --- | --- | --- |
| `AGENTS.md` | `instruction` | [source](https://pi.dev/docs/latest/sdk; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs) |
| `settings.json` | `setting` | [source](https://pi.dev/docs/latest/settings; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs; named on screen by `pi config` against the pinned 0.84.3 package) |
| `skills` | `skill` | [source](https://pi.dev/docs/latest/skills; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs) |
| `extensions` | `plugin` | [source](https://pi.dev/docs/latest/extensions; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs) |
| `prompts` | `command` | [source](https://pi.dev/docs/latest/prompt-templates; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs) |
| `themes` | -- | [source](https://pi.dev/docs/latest/themes; confirmed against the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs) |

A path routing no component kind is owned so a setup can carry it;
nothing compiles a component to it.

### A second target: `target_scope: user_root`

Rooted at `~/.agents`, which is not the configuration home
above. A consumer reaches it by naming the scope on the request, and
every path below is relative to that root.

| Path | Component kinds routed here | Decided by |
| --- | --- | --- |
| `skills` | `skill` | measured from the product's own bytes |

This root is read by several products at once, so under this scope
`remove`, the backup and a restore act on the files this program
recorded writing rather than on the directory whole. A neighbour's
files are never captured into a backup slot here, and never reverted
by a restore.

### Considered and not owned

Everything named here is left exactly as it was found, like any
other file beside a target.

**`.pi-setup-system`** -- This provider's own control directory: the target lock, the backup slots and their payloads. Kept out of the declaration for the same reason as the state file, and recorded here because the declined list is where a reader looks before opening a file to find out what it is. ([source](this provider's own contract; no vendor page is involved))

**`AGENTS.override.md`** -- Pi loads this instead of AGENTS.md or CLAUDE.md from the same directory, so a home holding one ignores the instruction file this provider installs. Not owned, for the reason an override exists at all: it is how a person overrides, and owning it would let `remove` take that away. ([source](https://pi.dev/docs/latest/sdk))

**`NDDEV-PI-PROVIDER.json`** -- This provider's own state file: which setup is applied, the identity it recorded, and which slot reverses the last operation. Written by every operation and excluded from target identity, because counting it would leave a target different from the identity the operation just wrote. Not a projection surface and never ownable as one. ([source](this provider's own contract; no vendor page is involved))

**`auth.json`** -- Authentication credentials. Pi joins it against its agent directory -- `agentDir, "auth.json"` in the pinned 0.84.3 bundle, beside `keybindings.json`, `models.json` and the owned `settings.json`. Never owned and never captured, for the reason the antigravity list gives: a backup of someone else's credentials is a leak with a schedule. Recorded here because the declaration's `never_touch` is checked against this block. ([source](measured from the pinned 0.84.3 bundle, package/dist/core))

**`git`** -- Package checkouts the product clones and manages: `~/.pi/agent/git/<host>/<path>` for a global install. Its own shipped docs say what happens to anything a person leaves there -- *"When reconciliation changes the checkout, pi resets and cleans the clone"* -- so it is not a surface a setup can promise, and a backup of it would capture somebody else's repository. ([source](measured 2026-08-28 in the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs))

**`keybindings.json`** -- A keymap file, joined against the agent directory beside the owned `settings.json`. Not owned, for the reason claude's row gives: no component kind describes a keymap. ([source](measured from the pinned 0.84.3 bundle, package/dist/core))

**`managed-config`** -- Not a path in the target, and recorded because **there is no such path** -- an absence somebody has to measure once so the next reader does not spend the search again. Four of the seven harnesses here carry a system-wide managed policy that overrides everything a user writes; this product carries none.

Measured 2026-08-29 against the pinned 0.84.4 package, whose bytes match this baseline's own sha256. Searched for the three shapes the other four use -- an `/etc/<product>` literal, a `%ProgramData%\\<product>` literal, and a `/Library/Application Support/<product>` literal -- across the whole of `dist/`. Zero hits of any kind. The product's own shipped `docs/settings.md` agrees by omission: it documents exactly two locations, `~/.pi/agent/settings.json` global and `.pi/settings.json` per project, with project overriding global and nothing above either.

**What that means for the `full-auto` posture**: nothing sits above it. On the four harnesses with a managed layer, a permissive posture can install, verify and restore cleanly while an administrator's policy quietly overrides it. Here the keys this provider writes are the last word, which is a stronger statement than it looks and is the reason the absence is worth recording rather than leaving as a gap in the table.

Absence of a literal is not proof a path cannot exist -- a future release may add one -- so this row says what was searched rather than that none will ever exist. ([source](measured in the pinned 0.84.4 package; docs/settings.md shipped inside it))

**`models-store.json`** -- A cache of remote model catalogs, persisted so a later run can restore them without a network request, refreshed on a four-hour throttle. A cache the product regenerates is never a configuration surface. ([source](measured 2026-08-28 in the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs))

**`models.json`** -- Named by the consumer's catalog as a second setting surface. The settings documentation does not describe it, and a row nobody can source is not owned. ([source](https://pi.dev/docs/latest/settings))

**`npm`** -- Where user package installs go, beside the git checkouts above and managed the same way. Declined for the same reason: the product puts content here and takes it away again. ([source](measured 2026-08-28 in the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs))

**`pi-debug.log`** -- Written by the hidden `/debug` command and holding rendered TUI lines with ANSI codes. A log, never owned and never captured. ([source](measured 2026-08-28 in the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs))

**`trust.json`** -- A person's saved decisions about which project folders may load project-local settings and resources and **execute project extensions**. The product's own shipped documentation names it five times and is explicit about what turns on it: *"pi asks before trusting a project folder that contains project-local settings, resources, or project `.agents/skills` and has no saved decision for the folder or a parent folder in `~/.pi/agent/trust.json`. Trusting a project allows pi to load `.pi/settings.json` and `.pi` resources, install missing project packages, and execute project extensions."*

Never owned. It is a security decision a person made, not configuration a setup can carry: installing one would grant execution to folders nobody approved, and a restore returning an older copy would silently re-grant a trust that had been withdrawn. Recorded here because it sits inside the home this provider configures and every other file there is accounted for.

**This product has no sandbox of its own**, which is what makes the row matter. Its documentation says so plainly -- *"Pi does not include a built-in sandbox. Built-in tools can read files, write files, edit files, and run shell commands with the permissions of the pi process"* -- and points at containers, VMs and micro-VMs instead. So trust is the only gate between a project folder and code running as the person who started the product, and it is a gate this provider must not touch. ([source](the product's own shipped documentation, read from the pinned 0.84.3 package at node_modules/@earendil-works/pi-coding-agent/docs; https://pi.dev/docs/latest/security))

**`working-directories`** -- One row for `git`, `npm` and `tmp`, joined against the agent directory. Scratch space the product manages for its own operations. ([source](measured from the pinned 0.84.3 bundle, package/dist/core))

**`mcp_config.json`** -- **The vendor says this product has none.** Its own shipped documentation, `usage.md`, under the pinned 0.84.4 bundle's own `package/docs/`, and confirmed against the live page 2026-08-29: *"It intentionally does not include built-in MCP, sub-agents, permission popups, plan mode, to-dos, or background bash. You can build or install those workflows as extensions or packages…"*

So there is no MCP surface at any scope, and the capability arrives through `extensions/`, which this provider owns and routes as `plugin`. A stated absence is worth more than a missing file: it says the next release will not quietly add one under a name nobody guessed. ([source](the product's own shipped documentation, usage.md under the pinned bundle's package/docs/; https://pi.dev/docs/latest/usage))

**`agents`** -- No sub-agents, from the same sentence as the MCP row above: *"It intentionally does not include built-in MCP, **sub-agents**, permission popups…"* The `agent` kind is therefore not declared for this harness, and nothing under this home is read as one. ([source](the product's own shipped documentation, usage.md under the pinned bundle's package/docs/; https://pi.dev/docs/latest/usage))

**`hooks.json`** -- Hooks here are an **extension API concept, not a configuration surface**. `extensions.md`, under the pinned bundle's own `package/docs/`, documents `session_start`, a `spawnHook` around tool execution and session-scoped teardown hooks -- all of them functions inside an extension module. There is no `hooks.json` and no `hooks` key in `settings.json`, so a hook reaches this product through `extensions/`, which is owned and routes `plugin`. ([source](the product's own shipped documentation, extensions.md under the pinned bundle's package/docs/))

## Response

One maintainer. Defects are triaged as time allows; security reports are
acknowledged first.
