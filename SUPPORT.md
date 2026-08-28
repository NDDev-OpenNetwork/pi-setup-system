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

The remaining one is not a defect in this build. `0.0.7` carries the field but
its scope enum is `["project"]` alone, while the provider kit this program
vendors and verifies byte-for-byte -- kit `0.2.4`,
`provider-info.schema.json` -- gives `["project", "user_root"]`. The kit is the
artifact a provider is told to build against, so a build that declares
`user_root` is right by the document it was handed and wrong by the checker
shipped beside it. Raised with the consumer, who owns both.

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
| `AGENTS.md` | `instruction` | [source](https://pi.dev/docs/latest/sdk) |
| `settings.json` | `setting` | [source](https://pi.dev/docs/latest/settings) |
| `skills` | `skill` | [source](https://pi.dev/docs/latest/skills) |
| `extensions` | `plugin` | [source](https://pi.dev/docs/latest/extensions) |
| `prompts` | `command` | [source](https://pi.dev/docs/latest/prompt-templates) |
| `themes` | -- | [source](https://pi.dev/docs/latest/themes) |

A path routing no component kind is owned so a setup can carry it;
nothing compiles a component to it.

### Considered and not owned

Everything named here is left exactly as it was found, like any
other file beside a target.

**`models.json`** -- Named by the consumer's catalog as a second setting surface. The settings documentation does not describe it, and a row nobody can source is not owned. ([source](https://pi.dev/docs/latest/settings))

**`AGENTS.override.md`** -- Pi loads this instead of AGENTS.md or CLAUDE.md from the same directory, so a home holding one ignores the instruction file this provider installs. Not owned, for the reason an override exists at all: it is how a person overrides, and owning it would let `remove` take that away. ([source](https://pi.dev/docs/latest/sdk))

**`NDDEV-PI-PROVIDER.json`** -- This provider's own state file: which setup is applied, the identity it recorded, and which slot reverses the last operation. Written by every operation and excluded from target identity, because counting it would leave a target different from the identity the operation just wrote. Not a projection surface and never ownable as one. ([source](this provider's own contract; no vendor page is involved))

**`.pi-setup-system`** -- This provider's own control directory: the target lock, the backup slots and their payloads. Kept out of the declaration for the same reason as the state file, and recorded here because the declined list is where a reader looks before opening a file to find out what it is. ([source](this provider's own contract; no vendor page is involved))

**`$HOME/.agents/skills`** -- Pi is a second product that reads the user-level convention root, and this is measured from the product rather than from a page. In the pinned 0.84.3 bundle, package/dist/core/package-manager.js:1976 builds `userAgentsSkillsDir = join(getHomeDir(), ".agents", "skills")` and line 2017 loads from it: `addResources("skills", collectAutoSkillEntries(userAgentsSkillsDir, "agents"), ...)`. Line 2012 names the root itself as `userAgentsBaseDir = dirname(userAgentsSkillsDir)`. A neighbouring use in trust-manager.js:160 *excludes* this directory while walking up for a project-scoped one, which is what a first reading of the variable name would have mistaken for the read -- so the line that matters is 2017, not 1976.

Not declared as a user_root scoped profile, though Codex declares exactly that shape (codex/native-files/user-root/1, namespace `skills`, kind `skill`) for the same directory. Two providers owning one path is not two owners: `remove_managed` walks native_namespaces and calls remove_dir_all on each, so a namespace goes whole, and either provider's remove under this scope would take the other's skills. Recoverable -- the capture runs first and restore returns them byte-exact -- but a shared root wants one answer about who owns it, and that answer is the consumer's and the owner's rather than this provider's to assume. Raised. ([source](measured from the pinned 0.84.3 bundle (sha256:d07dc417...), package/dist/core/package-manager.js:1976,2012,2017))

## Response

One maintainer. Defects are triaged as time allows; security reports are
acknowledged first.
