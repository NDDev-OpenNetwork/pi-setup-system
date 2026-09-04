# Changelog

This file is a release ledger: every heading below is a real release.

The project follows Semantic Versioning. `0.0.x` says plainly that the wire
surface is proven against the consumer's conformance but has not yet been run
against a real installation on every platform it claims.

An entry is never edited after its release. It says what that release was,
including claims a later release made false.

This repository ships `scripts/evidence.py` beside the Cargo workspace. Other
paths an older entry names may describe checks that ran when that release was
cut and that this clone does not carry.

## [Unreleased]

## [0.0.61] - 2026-09-04

Installed nddev-builder toolkits now name only validation commands
present in this repository. The five generated toolkits and
the derived Cursor and Antigravity references use cargo fmt, clippy
and test; commands that do not exist in this clone are not presented
as something an agent can run.

Codex software pins move to 0.153.1 across all six platform artifacts. Grok
Build 1.0.18 skill metadata is aligned with the current product: allowed-tools
is descriptive rather than a permission grant, and user-root command Markdown
is discovered as a skill rather than a separate component kind. All seven
catalogues are rebound to the resulting setup bytes.

## [0.0.60] - 2026-09-03

Provider profiles are adaptation-bound and accept only
`ai-stp-bundle/2`. Claude and Codex add evidence-backed project profiles;
scoped restore derives its inventory and promised digest from the exact
BackupRef payload without adopting neighbouring files. Vendor pins move to the
current seven-product set, Grok Build gains its current monitoring/background
workflow, and every generated nddev-builder is synchronized with the measured
native surfaces.

The same source now produces a self-contained crates.io package whose name
matches this repository, PyPI distribution and executable. Packaging, local
installation and provider-info execution are release gates; later publications
use short-lived crates.io OIDC credentials.

## [0.0.59] - 2026-09-03

Public commit and pull-request text is repository-local. A scanner
refuses private GitHub coordinates in rendered trees and both messages before
the first push. Historical commits remain immutable.

## [0.0.58] - 2026-09-02

A removal answers three ways when nothing records what this build
wrote. `remove` takes the declared namespaces whole, which is exact for a
target this provider wrote and is guessing at one it never touched: measured on
released 0.0.57, a target holding only a person's own configuration answered
*"Removed everything <provider> owns"* and took it, recoverable from the
capture and under a sentence that did not describe what happened.

Now: a record removes, as before. A target with nothing this provider declares
on it is silent and unchanged, because "already removed" must stay a no-op or
a repeat becomes an error where nothing happened. A target with declared
entries and no record is refused by name -- `state: refused`,
`unsupported_operation`, exit 0 -- with the entries it would have taken in the
detail, so a consumer can show a person what it declined to take. The same
three answers on the surface a person types and on the wire, and the question
is asked again under the lock, because the state file is outside the target's
identity on purpose and a record can be deleted between a plan and its apply.

The shape was agreed with the consumer before it shipped, and their half went
first: their reader now carries a refusal's reason and detail through to a
person rather than collapsing every non-planned answer into one sentence about
shape.

Every declared surface of all seven products is re-measured in the bytes each
current pin names, with an invented control absent in all of them: 72 of 72
present.

## [0.0.57] - 2026-09-02

A kind declared only by a scoped profile validates and plans under
that scope. `validate-bundle` compared a bundle's component kinds against the
global profile whatever the surface, so a kind a provider implements only
elsewhere — codex's `skill`, which lives under `~/.agents` — was refused, and
every scoped plan carrying it with it. Now `validate-bundle` accepts a kind
any declared profile implements, the question it can answer with no scope in
its argv, and a plan under a scope checks kinds against that scope's profile;
a global plan carrying a scoped-only kind refuses by name. Found by the
consumer's user_root slice: four providers passed because they declare the
kind globally too, codex had never passed.

A target the system cannot canonicalize is taken as given. Inside the
consumer's Windows AppContainer, `GetFinalPathNameByHandle` cannot map a
volume back to a drive letter for any path, so every `status` refused with
"cannot be canonicalized" while `provider-info` answered — measured by the
consumer four ways. Now, once the directory has been inspected and its final
component is not a link, a failed canonicalization falls back to the lexical
absolute path; the verbatim `\?\` prefix is dropped everywhere so one
directory carries one `canonical_target` string inside and outside a
container; and the operating system's own error travels in the refusal
detail. The seven public clones the render check makes now identify as the
job's token rather than as a shared address.

Antigravity is pinned at 1.1.24, published since the previous release. One
Cursor citation moved and is re-cited.

## [0.0.56] - 2026-09-02

`status_request_fields` is declared: `["target_scope"]`, the same
vocabulary as `plan_request_fields`. A consumer that reads the declaration
may send `--target-scope` to `status` and measure the inventory the plan it
is about to bind will measure, instead of the global namespaces at a
workspace root. The runtime has honoured the flag since 0.0.55; this release
adds the sentence that lets a consumer send it, in the order every
`provider-info` member follows: kit 0.2.9 names it, `ai-stp-cli 0.0.15`
accepts it, then the seven declare it. Kit 0.2.9 is vendored. Nothing else
moves.

## [0.0.55] - 2026-09-02

A plan whose scope contradicts the target's record is refused by
name. A target managed under `project` used to accept a plan naming the
global profile, measure the global inventory at a workspace where those
namespaces are simply absent, and — had it been applied — rewrite the record
with the wrong ownership. The consumer met the first half as a bare
`expected_target_digest` mismatch when their remove plan carried no scope;
the refusal now says which scope to send (`unsupported_operation`). One
direction only: a home managed globally may still be asked about a scope.

`status --target-scope <scope>` is honoured: asked, `status` measures the
inventory the plan will, instead of the global namespaces at a workspace
root that a repository may spell for its own reasons. Absent the flag,
`status` is exactly what it was. The `provider-info` member that lets a
consumer send it, `status_request_fields`, is present in the build and
empty until the kit names it and a released consumer accepts it; this
release publishes the same thirteen names 0.0.54 did.

## [0.0.54] - 2026-09-02

`remove` reads a bundle, and the plan says per path what stays.
The consumer's ADR-0129 case — a component that owns one key of a file the
person also writes — was inexpressible on the wire: a remove plan was built
without bytes, so "this path outlives me at bytes-without-the-key" had no
carrier. Now `plan-operation --operation remove` takes the same five bundle
arguments `replace` takes, and the plan gains an `end_state` member only when
one rides — per touched path, `removed`, or `final_bytes` with the member,
sha256 and byte_length copied from the bundle's own manifest — so a plan without a
bundle is byte-identical to what 0.0.53 produced. The apply refuses a bundle
the plan never described, a plan with survivors fed no bundle, and a bundle
whose members are not the ones the plan bound, all before the lock. After a
remove with survivors the record names no file: the bytes are the person's.

Declared through `plan_request_fields` in the ADR-0125 order, measured at each
step: kit 0.2.8 names the field, `ai-stp-cli 0.0.14` on PyPI accepts it, then
this release declares it. Kit 0.2.8 is vendored.

Cursor declares a `project` scope — `.cursor/rules`, `.cursor/commands`,
`.cursor/hooks.json`, `.cursor/mcp.json`, `.cursor/agents`, `.cursor/skills`,
each a literal workspace join in the 2026.08.31-4057e58 bytes. The home
`agents` question (cursor#94) was re-measured and the answer stayed no; the
global profile and its digest do not move.

Five products moved overnight and are pinned at what they publish today:
Claude Code 2.1.258, Codex 0.152.1, OpenCode 1.18.26, Cursor CLI
2026.08.31-4057e58, Antigravity 1.1.23. Grok 1.0.13 and Pi 0.84.4 stand.

## [0.0.53] - 2026-09-01

The other two readers of the exposed name, and the boundary that
stops a third. `0.0.52` moved the wire path — plan, apply's answer, launch,
remove, rollback — onto this platform's member; the human `software` and
`rollback` surfaces still derived the name from the table's first row, so on
Windows they answered *"Nothing is exposed"* over a prefix holding a working
cursor install. Found by this repository's own evidence matrix within the
hour of `0.0.52` shipping: the wire legs went green and the human legs went
red, which is the two halves of one defect being fixed one wave apart.

Both surfaces read this platform's member now, and the first-row hint is
**private to the kernel crate**: the fallback inside the platform accessor
and the deliberately-ordered candidate list for older trees are its only
callers, and a new reader reaching for it does not compile. A grep sweep
missed two call sites once; a crate boundary cannot.

## [0.0.52] - 2026-09-01

An operation that reads no bundle refuses one by name. Only
`install` and `replace` read a bundle, and the plan bound the five bundle
names into its artifact for every operation — so a `remove` plan carrying a
fully-named bundle answered `planned, valid: true` with the digests echoed,
and the apply removed everything with the bundle bytes untouched. Accept and
ignore, measured on the released 0.0.50; the only loud refusal was the argv
parser's, on a partial flag set.

A plan that echoes inputs its apply will never read lies about what approving
it means. The consumer's `end_state` design for `remove` (their ADR-0129)
assumed the loud refusal existed for providers that do not yet declare the
field; their declaration gate was in fact the only net. This release is the
second: `unsupported_operation`, naming the two operations that do take a
bundle. When `remove` learns to read one — the agreed `end_state` extension,
kit 0.2.8+ — the refusal narrows to the operations that still read none.

No released consumer sends a bundle on `remove`; their remove plans are built
with no bundle bound, so nothing anyone runs changes behaviour under this
release except the request that was already a contradiction.

And the plan's software entry point names the member **this platform**
actually gets. It was derived from the table's first row — a Unix member —
so on Windows a cursor plan promised `bin/agent` while the apply, resolving
this host's artifact, wrote `bin/agent.cmd`; `remove` looked for the wrong
name too and left the launcher behind. Found by the consumer's six-leg
matrix on `0.0.50`, Windows only, both architectures, cursor only — the one
harness whose Windows member is a batch launcher while its Unix members are
extensionless. Every reader of the exposed name — plan, apply's answer,
launch, remove, rollback — now derives it from this platform's artifact,
with the first-row hint kept only for a platform the vendor never published
for.

## [0.0.51] - 2026-09-01

`remove` names both of its cases. It said *"anything you put under
those"*, which is directory language: a person's own keys live **in**
`config.toml`, not under it, so the file half of the sentence read as not
applying to them. The consumer measured that these lines reach their approval
surface verbatim, so the sentence is the contract. It now says: your own keys
in a file it names go, your own files in a directory it names go, and the
backup slot captured first holds all of it.

The marker between a report and its reader is held from both sides now. Every
tool prints a machine `RESULT` line; a test binds each tool to the exact keys
it prints, and — since one rename cost a scheduled run — also requires the
workflow that parses a marker to parse the key that is actually printed. The
conformance report's third state is part of the same change: a sweep that
could not run says `unmeasured=` apart from `refused=`, and nothing downstream
may read its silence as a pass. The kit check's count is `differs=` rather
than `behind=`, because a byte comparison cannot say which side moved — and
the day it was renamed, the side that had moved was the vendor's.

The vendored provider-kit README follows the consumer's current copy, which
dropped a passage superseded on their side.

## [0.0.50] - 2026-08-31

Each repository now tracks what its default branch is meant to
enforce, in `.github/rulesets/branch-main.json`, and the GDS anchor states the
same required contexts. Until this release the live ruleset was the only
statement of that anywhere: a required check added or dropped through the API
changed what could merge and left no diff for anyone to read.

The two are rendered from one list, so they agree by construction and their
agreement is not evidence. The pair that can disagree is a repository and
GitHub, and that comparison is reported rather than gated, because it reaches
an API.

The release path gained the check that matters most to a consumer. `provider-info`
is compared by exact equality: a name too many and a name too few fail
identically, and the whole document is refused, taking fetch, conformance, plan,
apply and status with it. The seven are now compared with the field set read out
of the *installed released* consumer before any tag moves, so a drift is caught
here rather than as somebody's refusal. With no consumer installed the release
refuses to tag: an unasked question is not a passed one.

The GDS anchor line names the schema by its digest rather than by a version that
matched no release tag, with a behavioural probe kept beside it as the control
on that digest.

## [0.0.49] - 2026-08-31

A software operation applied through `ai-stp` now answers with the
plan digest it was handed. It did not before, in any of the seven, so `harness
install`, `harness update` and `harness remove` through the consumer refused
after the program had already been installed: the effect landed and the
operation stayed unverified over a prefix holding a working build. The
configuration operations always carried the echo; the program lifecycle owes the
same one, and the contract says so.

The consumer released tolerance ahead of this, as 0.0.12: a missing echo is
accepted for the three program operations, a mismatched one is still refused,
and a configuration operation must still carry both. That tolerance exists in
0.0.12 and in no earlier published version, so 0.0.11 and before refuse this
release's predecessor and 0.0.12 accepts both.

Nothing here could have found it. Every test asked whether the provider does
what its own answer says, and it did -- the answer was consistent with itself and
identical across all seven. The consumer found it by running the released
0.0.48 through its own install path. The new test asserts the echo against the
digest the wire was handed, for all three program operations.

Beside it, the rendered lock file is now projected from this workspace's own
committed resolution rather than re-resolved inside each tree, so a published
tree no longer depends on which machine rendered it; twenty-one recorded product
measurements were re-asked against the artifacts their baselines pin, three of
which returned a false negative because the search term was a label this project
invented rather than a string the product carries; and the schema survey was
completed for the four harnesses that had never been asked, including one that
publishes a manifest schema this provider must not declare.

## [0.0.48] - 2026-08-31

Antigravity's official updater publishes native Windows x86_64 and
arm64 executables for both releases this provider can move between. Those exact
PE files, their lengths and their digests were already present in the provider,
but stale availability metadata still named both Windows platforms as
unpublished, so a valid software plan was refused before it could name the
artifact.

The stale refusal is gone. Install, update, status, rollback and removal now use
the same measured two-release lifecycle on all six native platform pairs. A
generated invariant also rejects any current or previous release that calls a
platform both published and unpublished, preventing the artifact table and the
capability declaration from silently disagreeing again.

## [0.0.47] - 2026-08-31

A target caught part-way through a change can be recovered from the daily
command again, and the reason it could not is that neither side of this pair
had anything to be wrong against.

**An interrupted operation was reported, in a name nothing reads.** Every one
of these systems has kept a durable record of a mutation in flight since the
beginning, and published it. The tool that installs and updates them decides
whether recovery is owed by looking at two other things entirely -- a state
value none of these systems emits, and a key none of them had. So a target
holding an unsettled record answered *managed*, both of those questions came
back no, and the recovery command that exists to settle it was never reached
from the path anybody uses.

The fact was in the answer the whole time, under a name the reader does not
know. It is now also under the name the reader does know, saying which of the
two things is owed: a restore, when the effect may be partial, or a tail to
clear, when the effect landed and only cleanup remains. A settled target says
so explicitly rather than staying silent, because silence is what a system
that does not speak this looks like -- and that is exactly the state these
were in.

It is a separate field rather than a fourth value of the one that says what is
in the directory. That one is read by everything, and overloading it would
make every existing reader wrong about a target that is merely mid-operation,
which is the failure being closed rather than a new place to put it.

**And the reason this was not catchable.** The kit these systems are built
against declares the shape of one answer and no others. The response this
defect lived in has no schema at all, and the case file names none of its
thirty-three fields. Both sides invented it independently; it works because
the names happened to agree, and where they did not, nothing failed.

So there is now a record of what that answer actually contains, taken by
running every one of the seven against a directory that is empty, one holding
somebody else's files, and one this provider manages. Checked both ways: a
field published and not recorded is one the far side cannot know about, and a
field recorded and no longer published is a promise that stopped being kept.
It does not make the contract -- that belongs to the tool these are built for
-- but it makes the published set a fact with a reader, so the next field is a
difference somebody sees rather than a discovery months later.

**Closed before this release published.** The consumer turned that measured
shape into provider-kit `0.2.7`: a checksummed closed status schema and a
conformance case carrying the required, verified and enum sets. These trees
vendor it in the same release. The local status record remains only as a
derived readable projection, so there is one contract rather than the two this
section was written about.

**One more name a target can carry that this provider does not own.** Reported
beside a target still described as clean, for one system whose product reads a
second spelling of its settings file and both spellings of every component
directory. Clean is a statement about the bytes this provider wrote and never
was a statement about what the product obeys.

**A posture stopped emptying directories it could never fill.** Selecting a
setup replaces everything this provider owns, which is what makes switching
between them predictable -- and it was doing that to places nothing here could
ever put anything: no component routes to them and no posture ships files
there. Every posture agreed they were empty, so the emptiness was not a
statement any of them made, and the only thing ever in such a directory was
somebody else's. Twelve of them, across five of these systems; one held a
person's key bindings and a plain switch of posture took them.

They are still owned, which is the point. A backup captures them, the recorded
identity notices when they change, and removing the setup takes them --
returning a directory to unmanaged is a different act from changing posture
inside it. Only the emptying stopped.

**And the list of things this provider promises never to touch now means it.**
It named three effects of ownership and prevented two: those paths were not
copied into a backup and not folded into the recorded identity. The third went
through, because replacing a directory removed it whole and never asked. One
product writes a person's marketplace sources inside a directory this provider
owns, and a change of posture took the file.

**One system stopped installing a program that cannot start.** It was installed
from a package whose entry point is a script needing an interpreter the host
supplies, so on a machine without a recent enough one the install succeeded and
the program could not run, with nothing in between saying so. The vendor also
publishes complete standalone builds for every platform declared here, and
those carry their own runtime: the whole lifecycle was exercised with that
interpreter deliberately unreachable. Two different layouts inside one release,
read from the archives rather than from the platform names.

**Two files that shape what the model is told** are now owned by that same
system. Its own documentation names both, and this record had neither -- not
owned, not declined, absent, which is the one state a surface must not be in.
They are kept rather than replaced, for the reason above.

**And every one of these products was asked whether it can be told to stop
updating itself.** Three can, and one of those had been told to since an
earlier release without the answer ever being written down where anything
checks it. Four cannot, and that is now recorded as a measured absence rather
than as an empty field, which reads the same and means something else.

**And one system's record was re-asked at the version it pins.** Every row of
it described a release eight versions back, of a product that moved those eight
in a few hours. Re-measured against the pinned bytes rather than re-read: seven
surfaces hold and now say which release they are about, and the one covering
plugins gains the shape it was missing -- a plugin there is a directory holding
a manifest, and a single file with the right name is silently nothing.

That correction went the other way first. The probe that found "nothing loads
here" was itself malformed, and the control is what said so; without it a
working namespace would have been changed because an instrument answered about
itself. The vendor's own installer also writes a pair this record had no row
for -- an installed-plugin directory and the registry that lists it, useless
without each other -- and that pair is now named as something never read,
captured or removed, alongside the credentials.

**The development setup no longer downgrades the development agent.**
`nddev-builder` used to start from each product's conservative baseline, so
installing the authoring toolkit brought approval prompts and sandboxes back on
the very path meant for autonomous implementation. It now starts from that
product's own `full-auto` posture and adds knowledge without changing authority.

**Every software lifecycle now has two real releases to cross.** Cursor and
Antigravity were the last two with no previous pin. Their immediately preceding
artifacts had already been measured in this repository before the pins moved;
the immutable identifiers still resolve, and the missing Windows bytes were
fetched and hashed before the records were completed. Cursor's no-launch
declaration also found a flaw in the evidence script: it tried to prove an
inactive removal by calling a command the provider intentionally omits. The
exposure is checked without inventing launch now, and the full two-release
sequence passes against both products on native Linux x86_64.

**Vendor-byte evidence names all six native hosts.** Exact hosted labels cover
Linux, Windows and macOS on x86_64 and arm64, and each job asserts the runner
architecture before it builds. The previous matrix covered three operating
systems and only half of their architecture pairs, while the release already
published all six provider binaries.

**And the estate table reads launch from the declaration that decides it.** It
used to infer from software plus an environment-variable name, which called
Cursor launchable after its own `LaunchBinding::Partial` had withdrawn the
capability. The generated README and SUPPORT pages now say five, not six, and
name Cursor and Antigravity separately.

## [0.0.45] - 2026-08-31

A posture that said it changed nothing was changing the thing it named, and
the check that should have caught it was asking a different question.

**A rule you can still see is not a rule that still governs.** One system's
full-auto posture said seven of the product's own rules could not be moved by
configuration -- prompts before reading secret files, before reaching outside
the working directory, before looping, and refusals around planning. Measured
against the pinned build: that product decides by the **last** matching rule,
and anything a posture writes is appended *after* the product's own. The
catch-all this posture installs therefore lands after all seven and overrides
every one of them.

The measurement behind the old sentence asked whether those seven rules were
still in the resolved list. They are, every one, and the product still prints
them. It never asked which one decides.

**The probe could not have disagreed.** The check that runs the real product
each week asked it to render its resolved *configuration*, and asserted the
posture's own keys appeared in it. They always did: that command renders back
what was written. It answered *was our file read* while the description claimed
*and it changes nothing* -- two questions, one green result, three releases.
It now reads the ordered rule list the product actually decides by, and a
posture marker may require its parts **in order**. The reason that is worth
keeping: the same product with no configuration at all prints both parts, so a
check that only asked whether they were present would pass there and prove
nothing.

**A provider can be clean about its own bytes and silent about what runs.**
That product reads a second spelling of its main settings file and keeps the
later one, and scans both the singular and plural spelling of every component
directory. A file this provider does not own can therefore decide, or replace a
component that it does -- measured in both directions, and which of a colliding
pair survives followed the order the two were written rather than which name
belongs to whom. `status` now lists those names when they are present, beside a
target it still reports as clean, because a clean digest is a statement about
the bytes this provider wrote and never was a statement about what the product
obeys. It reports and does not refuse: the file belongs to somebody, and this
provider does not know whether they meant it.

**Product self-update is switched off where that system offers a switch.** The
reason not to let a product replace its own bytes was written here three
releases ago -- this provider pins a version, records its digest and offers the
one beside it -- and nothing enforced it for that system. It sets the
documented variable at launch now. It stops the automatic path and not a person
typing an upgrade command, so it is not the same promise as the entry that
covers both, and the declaration says so.

**And an authoring guide told four systems a field did not exist.** Agents for
one product were documented as taking their identity from the filename, with no
name field. That field exists, and it replaces the filename entirely: an agent
carrying one is reachable only under it. The comparison table that says how the
same file differs across systems carried the wrong row into three trees the
claim was never written for.

## [0.0.44] - 2026-08-31

One system stops claiming it can start its product, and the reason had been
written down in this repository for three days.

**A launch declared from two questions that do not decide it.** The rule asked
whether a build installs its product and whether that product documents an
environment variable for its configuration home, and concluded that the product
could be started against any target a caller names. Both are necessary. Neither
is sufficient, and for one harness the answer was already recorded and disagreed:
its own baseline note says that of the eight things this provider owns there,
**one** follows that variable. The rest are built from a literal join to the
process home and reach no override at all.

So a launch against a chosen target assembled a session from the caller's own
rules, hooks and integration servers and the target's settings file. Hooks and
MCP servers are executable. None of them came from the setup anybody selected,
and the same target behaved differently for different people.

That system now declares no launch, and says why in the refusal rather than in a
generic sentence -- the previous one told callers *"this build installs no
software"*, which was false, because it installs and removes it.

Launch is a stated fact per system now, carrying **how** it was established. The
five that remain are not equally well established: three were measured by asking
the product which home it resolved, one by making it write into the target, and
one rests on a vendor page because no credential-free command of that product
writes or reports its home. The weakest is declared with its basis visible rather
than levelled up to look like the others or withdrawn on a technicality.

**A vendor command script exposed under a name Windows cannot run.** The rule
that names the stable command asked *is this member JavaScript* and answered the
bare command for everything else. One vendor ships a batch launcher, so the
stable command became an extensionless file holding batch text, hard-linked as
though it were a program. It classifies the member's kind now, and a command
script gets a wrapper that calls the vendor's script **where it lives** --
copying it out would leave its own directory reference pointing at nothing.

A native executable deliberately still has no extension, which looks
inconsistent and is the measured distinction: starting a program by an explicit
path reads its header rather than its name, so an `.exe` runs whatever it is
called. A batch file has no header. One of the two was broken.

**And a ninth surface nobody had a row for.** One product reads a separate
sandbox policy file from the process home, with its own filesystem and network
keys, which an administrator can relocate. It is declined rather than owned, for
routing reasons rather than reluctance, and the posture that switches sandboxing
off now says what it writes and what it cannot see.

## [0.0.43] - 2026-08-31

Three corrections, and two of them close the last open questions an outside
review of one harness raised about the program lifecycle.

**A launch checked that a file was there, not which bytes were in it.** It
verified that the exposed command existed, that it was a regular file, and that
the host could execute it. None of those says *which* program. So anything with
the prefix in reach -- the product replacing itself, a package manager writing
over the tree -- was started and reported as the pinned release, while the plan
that authorised the install, the digest recorded beside it and the rollback to
the version next door all went on saying otherwise.

Exposing a version now writes a record beside it: the version, the executable's
path, and its digest at that moment. A launch checks the digest and refuses when
it disagrees, naming both and starting nothing. A prefix with no record is
accepted, because one written by an earlier release has none and refusing those
would call every older installation tampered-with. What a record cannot do is be
present and disagree.

**An interrupted install was resolved by whatever ran next.** Configuration
mutations have a durable journal and a recovery that reads it; software
operations have neither and cannot, because that recovery names a configuration
target and this work happens under a program prefix. So the leftovers were
cleared as a side effect rather than by a decision, and nothing could say an
operation had been interrupted.

The filesystem is enough of a record here, because the promotion is two renames
in a known order. Three states, each with one right answer, and one of them is
resolved wrongly by luck: a missing version directory with a full quarantine
beside it reads as *nothing installed* to everything else, so the next install
would plan against a prefix whose real state was a version set aside. Recovery
runs under the lock before anything reads the prefix, and its answer travels in
the response -- empty on every ordinary run.

**And an instrument that named the asker and never the asked.** The conformance
report printed the checker's version and then seven verdicts, never the version
of the providers it drove. It reads them from the built binaries, a debug build
does not touch that directory, and so a day of ordinary work left it stale while
every run reported that everything conformed. Measured: binaries eight releases
behind a workspace, and verdicts published all day about a tree nobody had. The
conclusion survived a rebuild. The instrument did not, and the instrument is the
thing being sold.

## [0.0.42] - 2026-08-30

Three corrections, and all three are one defect seen from different angles: a
statement that is true and does not say what it is about.

**A marker write that could stop on another installed version.** The record of
which version a program prefix runs is a plain file holding a version string, and
a reader rejects a truncated one because a fragment is not an installed version
-- unless the truncation stops somewhere that *is* one. `1.2.3` cut short is
`1.2`, and where `1.2` is also installed the reader believes it, because 1.2
really is there. Nothing in the file separates *"1.2 because that is what runs"*
from *"1.2 because the write stopped there"*.

That accident was constructed by the consumer of these providers after both
sides had reasoned that only a person could produce a wrong-but-plausible
record. It is not a person: it is an interrupted write and a sibling version
whose string is a prefix of another, and the prefix relationship makes the
second half free. The write stages and renames now, so a reader sees the record
that was there or the one being put there and never a third thing. It cannot be
fixed in the reading, on either side, and the attempt would have to distrust the
record exactly where the record is right.

**A total published over two roots under one label.** The evidence report counted
sixty-five owned rows: fifty-five at the harnesses' own configuration homes and
ten under a scope's separate root, with nothing saying so. The arithmetic was
never wrong, which is why no check caught it -- a count has something recomputing
it and a label has nothing.

**A verification that did not say what it compared.** The render check reported
that all seven published trees match this source and named neither side. It
clones fresh from each remote, so a stale local copy could not fool it; what it
could not tell you is which commit of each tree the agreement was about. It says
so now.

Also: a documented reading that had outlived its own present tense, dated to the
day it was taken with what changed named.

## [0.0.41] - 2026-08-30

Four corrections, and the two that matter are in the kernel every one of
these systems shares.

**Removing a version nobody was running took the command that ran the other
one.** The removal took the version tree, then took the exposed command and the
version marker whenever that tree existed -- without asking whether either named
the version being removed. So the ordinary sequence after a bad release, install
then update then roll back then take the bad one off, deleted the command
pointing at the good one and left a complete, working installation that nothing
could start. The sentence above that function has said *"and the exposed command
if it pointed at it"* since it was written; the code never asked.

**An install that failed destroyed the install that was working.** The installer
cleared the version directory and extracted into that same path, so between those
two steps there was no installation at all -- and on a reinstall of the version
currently exposed, the command pointed into a directory that had just been
deleted. A crash, a full disk, or an archive that turns out not to carry its
declared executable took the working program with it. It stages in a sibling and
promotes with two renames now, in the order that leaves a complete tree at the
final path at every moment a reader could look.

**Neither could have been caught by the evidence run, and the reason is worth
more than the fix.** That run rolls back and then moves *forward* before
removing, so the version it removes is always the exposed one -- the case a
removal cannot get wrong. The case it did get wrong was the one the happy path
never enters. It does the other order now, against the vendor's real bytes.

**Six of these seven repositories promised a restore of components their provider
cannot install.** The sentence naming what a restore returns was written once, by
hand, and rendered into every tree. Measured against what each build actually
declares, it was true of one of them -- and it also understated, because six
route a plugin kind it never mentioned. It is generated from each declaration
now, which is the repair the neighbouring "does this harness install its product"
flag already got after being wrong for three of the seven.

Also: a launch no longer hands a product a way to replace the bytes it was pinned
to, where that product documents such a variable and this estate has read it in
the shipped artifact; a permission rule described as a boundary now says which
boundary it is, measured in the product rather than argued from a page; and three
system policy paths recorded as literals turn out to be none of them, joined at
runtime like every other path in that record.

## [0.0.40] - 2026-08-30

Two merges, and both came out of reading an outside review against the product
rather than against the review.

**A capability key that set nothing, in the posture whose whole job is
capability.** One harness's `full-auto` wrote `view_image` under `[tools]` for
two pins. That table has three members at the pinned release and this is not one
of them, and the product denies no unknown fields, so the key parsed, was
ignored, and left the file claiming a capability it never turned on. The control
is what makes it certain: the product reports the same feature count for an empty
file, for that key, and for an invented one, and a different count for the same
key spelled the way the product reads it. It is also enabled by default, so even
the correct spelling would have changed nothing. The posture now sets the three
stable features the build actually leaves off, and reads back as three overrides
where there were none.

**A component kind withdrawn on a control that could not have failed.** The same
harness declared no `agent` kind, on the reasoning that a role is irreducibly two
files. The product had been scanning the directory the whole time. The
measurement behind the withdrawal planted a Markdown file where the scan filters
on `.toml`, so the negative it produced was the only answer it could ever have
given, and a conclusion was written from it. Re-measured against the pinned
binary with both controls, the kind is declared again, and the toolkit ships its
own role as the one file the kind installs.

**Guards that had never been seen failing.** Eleven checks in this repository now
have a control that plants the exact defect each describes into a fresh copy of
the tree and requires a refusal. Building it found three things and only one was
a check: its own isolation restored the damage rather than the file, one planted
defect was legal under the vendor's schema, and three checks answer on a `RESULT`
line rather than an exit status -- which is deliberate and left alone, with a
`--strict` added for callers who want the status.

**And the vendor's own schema, asked about the file that cannot name one.** A
TOML configuration has no `$schema` line, so 143 of 152 shipped files sat outside
the schema sweep. One vendor publishes a JSON Schema of its configuration types
as a release asset; asked, it names the defect above in one line. A harness can
declare that schema now, by a url template filled from the version it pins, so a
pin refresh cannot leave the two disagreeing.

Also: a scoped operation named the global projection profile in its plan and in
the state written afterwards; a profile id promised a projection its build does
not declare; a note recorded a gap that had closed and could not notice; and a
posture restated two of its three keys as the product's own defaults, measured,
kept, and now said out loud.

## [0.0.39] - 2026-08-30

Eight changes, and the ones worth reading are the two where something
was checked by nothing and the one where a document told a reader to open a file
that had been deleted.

**A validator shipped into people's homes, wrong three ways, run by nobody.**
Every install of the authoring toolkit on one harness put a 192-line program
into a person's configuration directory. Nothing named it -- no entry point, no
command, no check -- and run against the toolkit it ships beside it reported 28
errors on a correct installation, because the layout had moved under it twice
and the pages it demanded had been replaced. Removed rather than repaired:
repairing would have put a second copy of the expectations into a file nothing
runs, which is what produced all three faults.

**Eleven pages a reader is never sent to.** The configuration file and the
instruction file are what a setup spends most of its bytes on, and the pages
teaching them were written into four harnesses and named by no entry point. A
check for exactly this already existed on the hand-written half of the
generator; the generated half had the same omission the whole time, because
there the generator writes the entry point itself and "it points at what it
writes" reads as true.

**And the direction of the question this estate did not have.** Every check here
asked whether a *named* file exists. None asked whether an *existing* file is
named. Both findings above are that one gap, and it is closed now.

**What a person reads before approving an install is now held.** The digest that
binds all 28 setups covered the payload tree and nothing else, so the name, the
sources and the description beside it could change with every check staying
green -- proved by rewriting one and watching that happen. The description is
where a posture says what it does and does not do, and on one harness it runs to
three thousand characters of exactly that.

Also: two limits that guard against hostile input and had never been driven at
their boundary, one of them a memory bound where the line after the check
allocates from a number the input supplies; three corrections to what this
estate teaches about one product, from re-reading the vendor's own pages; and
eight repository records that now say, in the estate's own vocabulary rather
than in prose, that they are generated rather than authored.

## [0.0.38] - 2026-08-30

A guard that was passing while it examined nothing, and the answer to
a question two projects had recorded as settled without running it.

**The guard.** Seven of these repositories run a check that walks a setup's
files, keeps the ones that are a component's entry point, and refuses any that
cannot describe itself. Each asserted that it found no problems. That assertion
is also true when the walk found **nothing at all** — and one of the seven says
so in its own words, because that harness genuinely ships no entry point of this
kind. So one was knowingly asserting a check with no subjects, six were assuming
subjects and never saying how many, and a change to the layout that removed
every entry point would have left all seven green.

The check now reports how many things it looked at, which forced every caller to
state the number its own tree carries — including the one whose number is zero,
and which now carries the reason it is zero. Confirmed by making nothing count
as an entry point: every previous assertion still passes and the new one fails.

**The question.** A manual experiment ships here asking whether a directory
carrying one particular kind of permission entry can be reached from inside an
isolated process, when the directory above it deliberately carries none. The
answer is that it can. It was recorded as settled the other way for four days,
on both sides, by nobody running it.

Three corrections to the instrument stood in the way and **every one of them
imitated the finding**: it could not start at all; then it failed while building
the permission entry, reported at the line that applies it; then it could not
enter the isolated process, on a directory whose parent grants nothing. Each of
those, read quickly, is the expected result. The experiment printed *"could not
be entered"* rather than *"was refused"* only because the two were kept apart
when it was written.

The debt that answer was justifying is **not** lifted — nothing here builds or
proves the thing it was blocking. What changed is that the reason recorded
against it was false, and a false reason is worse than none, because it is what
stops anyone looking.

Also records a rule that was measured and then **declined**: widening a
reference check beyond documents would flag 119 paths of which 117 are correct,
and two of the two real ones are the page that documents their removal.

## [0.0.37] - 2026-08-30

The manual Windows experiment, corrected a second time, and this
release exists only to carry it.

The experiment asks whether a directory carrying one particular kind of
permission entry can be reached from inside an isolated process, when the
directory above it deliberately carries none. Two projects had recorded the
answer as settled for four days without either of them running it.

Its previous run refused to answer, and said so plainly: it could not start the
isolated process at all, so nothing had been learned. The reason was in the
experiment's own setup. The child was told to start *inside* the directory under
test, and a working directory has to be reachable before a process begins — so
the run was asking whether the isolated process can walk the directory above,
which is exactly the thing being kept out of reach on purpose. It failed at the
door and would have failed there every time.

**That failure looks like the answer.** A not-found, from a call entering an
isolated process, on a directory whose parent grants nothing, has the same shape
as *"walking the parent is required"* — which is the belief this experiment
exists to test. It reported "could not start" rather than "was refused" only
because the two were kept separate when it was written, and that distinction is
the whole value of the run.

The child now stands in a neighbouring directory, granted access on purpose and
named so nobody mistakes it for part of the measurement, and the program it runs
is named by full path rather than searched for. What is being measured is
unchanged.

Nothing in these programs changed. This is the third correction to the
instrument and the question is still open — but it is open now for a reason that
can be stated, rather than because nobody looked.

## [0.0.36] - 2026-08-30

One line, in a workflow rather than in the program, and it is here
because the alternative was leaving seven repositories carrying a broken
experiment.

`0.0.35` shipped a manual Windows probe into these trees. It answers a question
two projects had carried as permanently settled without either of them having
run it — and it landed here because the workspace it was written in cannot start
a hosted job at all, so it had been dispatched twice there and never once begun.

Dispatched here it ran, created an AppContainer profile, printed its identifier,
and then failed placing the access rule this whole experiment is about:

    "Some or all identity references could not be translated."

The rule was handed an identifier as **text**. Given text it tries to resolve an
account behind it, and this kind of identifier has none. It takes an identifier
object instead. Nothing about the question being asked is involved.

**The reason this is worth a release of its own** is where the failure appears.
It is raised while the rule is being built and reported on the line that applies
it — so it reads as *the access rule was refused*, which is precisely the
outcome the experiment exists to detect. Somebody skimming that run would have
recorded the assumption as a measurement and closed the question.

Nothing in the program changed. What changed is that the instrument now fails
later, for a reason it names, and can be asked again.

## [0.0.35] - 2026-08-30

Four merges, and the two that matter were found by running this
program rather than by reading it.

**The line printed before a write did not say what the write takes.** Installing
a setup replaces every namespace this provider owns *whole* -- each is removed
and rewritten -- so anything a person kept under one goes into the backup slot
and out of the target. Until now only `remove` said so. The plan a consumer
renders before asking for approval said *"withdraw every file this provider
owns"*, which is not what happens, and the install preview enumerated the files
it would write while saying nothing about the removal preceding them.

The sharpest case is a harness that owns a global `rules/` directory, routes
instructions to it, and writes there from no setup, because its floor is
delivered as a plugin instead. Installing any setup emptied that directory, and
the sentence before it named three files.

Nothing about what an operation does has changed. What changed is whether the
person approving it was told: every surface now names each namespace, says they
go whole, and says the backup slot holds whatever else was under them. Under a
named scope the sentence is the opposite, because the behaviour is -- there the
verbs act only on files this provider recorded writing, so a neighbour sharing
the root is left alone.

**A rollback readback that was blind to the name it had just written.** On
Windows a JavaScript entry point is exposed as `<command>.cmd`, and the reading
that says which version is live looked for `<command>`. So a rollback succeeded
and then reported that the prefix still ran something else. The fix is the
accessor: the member-blind constructor is gone, which turned every caller into a
compile error and made fourteen tests state which member they meant. All
fourteen had used a native binary whose two names are identical -- which is why
three operating systems were green while nothing ever entered the branch that
was wrong.

**Every shipped file is now checked against something.** Six of a hundred and
fifty-three named a vendor schema; the rest were held only by a digest, which
refuses a silent edit and says nothing about whether the bytes are right. They
are now parsed in the format their product reads, checked against the required
keys their product documents, and required to resolve every reference they name.

**And every configuration key a setup writes must be sourced** -- by a page that
setup cites or by the harness's own baseline. Seven keys turned out to rest
entirely on a comment, and that comment named a release three behind the one
pinned beside it. They were measured in the pinned artifacts instead, each with
an invented control key that appears nowhere in the same bytes.

## [0.0.34] - 2026-08-30

Fourteen commits of corrections that no check could have made, and two
checks that now exist because of them.

Nothing in this release changes what a setup installs or where a component
lands. It is a pass over the *claims* this estate makes about itself, from a
day spent asking the artifacts to refuse things rather than reading what they
say.

**The revision every provider publishes was never recomputed.** Each of these
seven binaries reports a `kit_aggregate_digest` -- the revision of the ai-stp
provider kit it was compiled against, and the value the kit's own page says to
pin. The guard beside it iterated the lines of `SHA256SUMS` and asked one
question of each, which leaves three ways to be wrong: an empty file passes an
iteration over its lines, `KIT-IDENTITY.json` names its own file list and
nothing compared the two, and the aggregate itself was recomputed by nothing.
The value is correct today; the claim was unheld. The guard now asks in both
directions and ships a control that was observed refusing each of the four
defects, on a copy rather than on the kit it guards.

**A retry was applied to one of three call sites, and Windows found the
second.** A tree walk racing a writer meets a name that is gone by the time it
is stated. The first fix moved the refusal from `cannot stat` to `cannot open`,
which is how the second and third came to be measured -- by a race rather than
by reading.

**One reason was removed and three copies of it outlived the commit.** A
measurement corrected pi's exclusion from the program lifecycle and corrected
the declaration; the sentence survived in three more files, one of them stating
a tally of which harnesses install a program. A count in prose has nothing
holding it; a predicate does, and the runtime's paragraph now states the rule
instead of the tally.

**The page six siblings rewrote, and the seventh kept the old one.** Cursor's
builder toolkit told a model to run a command that does not exist in the tree
shipping it. Five of the seven toolkits are generated and two are written by
hand, and both hand-written ones have now drifted.

**Two gate checks CI never ran, beside a page saying it did.** Both are purely
local and finish in a tenth of a second, and neither was wired into any
workflow, while `CONTRIBUTING.md` said CI runs the same commands. Two other
checks reach vendor servers and stay outside the gate on purpose -- that is now
written down where a person reads it, so *not running them* is a decision
rather than an absence.

## [0.0.33] - 2026-08-29

Four things that were wrong while every file was valid.

A pass across all seven of these systems, asking each the same questions rather
than reading each on its own: does the builder toolkit teach every kind its
harness declares, is every file in the shape that product reads, do the three
postures mean one thing, and does each say what it does. Nothing here was
failing, and no guard could have caught any of it.

**A shipped instruction told a model not to create two things this build
declares.** The rule a model reads while working on Pi Coding Agent said the product
documents no global instruction file and no global command, and to create
neither. Both had been corrected in the declaration and not in the document. An
entire section of the primary skill said the same at greater length and ended
*"that is a fact about the product, not a gap to fill"* -- which is how a claim
stops being re-examined. The evidence against it was inside an artifact this
repository already pins and had already downloaded.

**A toolkit taught nothing about a kind its harness had just gained.** The
declaration landed and the document that exists to explain declarations did not
follow.

**A posture described itself in a neighbour's vocabulary.** One product has no
permission model at all -- its own documentation says so -- and its baseline
promised "conservative settings" while writing an enablement, because that is
the sentence the other six use.

**And one builder dropped a floor the other six carry.** Six take the same
working floor from their baseline into their builder setup, byte-identical
across all of them. The seventh replaced it. Both documents were real and
correctly shaped; the difference is only visible by asking the seven one
question and noticing one answers differently.

The shape they share: **a negative taken from the pages you happened to read,
and a sentence that survives because it reads as settled.**

## [0.0.32] - 2026-08-29

Two package families this program could not deliver, withdrawn.

A provider declares which *package families* it can unpack a component into.
Two of the seven declared families they had nowhere to put. One product's
plugins are drawn from a hosted directory with nothing under its home to hold
them. The other declared a marketplace while every path a marketplace is
registered in sat in that same declaration's **declined** list — one real half
and one naming nothing, which is worse than two wrong halves carrying a note,
because the note is the thing that gets re-read.

Nothing is stranded. The consumer counted its published corpus first: no
component anywhere requests either family from these two, so this narrows a
promise rather than refusing an install.

**And the rule is enforced now rather than remembered.** Every declared family
names the owned surface it unpacks into, and a family with no name, or one
naming a path the harness does not own, is refused before release. The
component half of that rule — *a declared kind is a promise of a rollback* — has
been enforced since this program had surfaces; the packaging half never was, in
the same file, one field along.

One surface where a personal marketplace really does exist stays unowned on
purpose, and the reason is worth stating because it is not a technical
limitation. It lives in a directory several products read. A skill installed
there is one product's file that others may also read; **a marketplace is an
instruction to all of them**. Putting this program's backup and restore in
charge of where a neighbouring product resolves its plugins is a larger claim
than configuring a home, and it is not one a routing table should make on
anybody's behalf.

## [0.0.31] - 2026-08-29

Three guards, each found by using this program rather than reading it.

**A walk written when a surface held one kind.** Pi Coding Agent's table gained a
second kind on one directory in `0.0.30`, and two checks here were keyed on that
directory believing everything under it was the first kind. One reported a
plugin as a component that had lost its entry point; the other never asked a
plugin to name itself, so a component with nothing to describe it would have
shipped unremarked. Both are the same cause arriving as a false positive and as
a gap, and a consumer of this protocol found the mirror of the first in its own
code the same afternoon.

The discriminator is the products' own: a manifest, not a location. Matched on
its suffix rather than against a list of the vendors met so far, so a vendor
nobody has seen yet works instead of being a silent miss.

**Two of the six harnesses that offer plugins have no manifest at all** — one
ships a module file exporting functions, another a package — and the comment
here claimed the manifest shapes covered every product. They cover four of
seven. Answering *no* for the other two is correct rather than a hole, and the
claim survived being written because no setup ships a plugin for those products,
so nothing could contradict it.

**And an absence with nothing behind it.** Driving the full software lifecycle
against a vendor's real bytes printed `reads -> not asked:` with an empty reason.
One of two neighbouring arguments required a measured reason for its absence and
the other defaulted to the empty string. The rendered workflow always supplies
one, so this was reachable only by hand — and that script documents itself as
runnable by hand.

The shape all three share: **a rule applied to one of two cases, and the check
written before the second kept answering for the first.**

## [0.0.30] - 2026-08-29

A plugin this provider can host in a surface it already owns.

Every blank in this program's component matrix was re-asked against the
vendor's current word rather than against the record. Thirteen of fourteen came
back correct. One did not.

**Pi Coding Agent's table gained a kind.** Where a product loads a plugin from a
folder inside a directory this provider already owns -- distinguished from a
plain component by a manifest the product itself reads -- that is a plugin this
program can install and roll back, and the kind is declared now. The reason it
was blank is worth keeping: the record said a plugin projects through the
settings file, which was true of *enabling* one and was never the whole
question. A negative taken from the pages that happened to be read.

**One blank stayed, and its evidence is now much stronger.** A vendor's own page
lists a user-level directory for a kind; the shipping binary resolves that
directory from the workspace path alone, with positive controls present and an
invented path absent. So the page documents something the product does not do.
This estate had built its habit around pages going *stale*; a page can also run
*ahead*, and declaring on one would have this provider claim a path nothing
reads. The rule is not "bytes over pages" but **the artifact decides, whichever
direction it disagrees in**.

Also here: a software pin moved the same afternoon the last release published,
and a citation naming the older build had quietly become a claim rather than a
measurement -- an artifact nobody can fetch cannot be checked. Re-asked on the
new build with the same controls, and it holds. **A measurement is reproducible
only while the thing it names can still be downloaded.**

## [0.0.29] - 2026-08-29

A verdict read without the subject that gives it meaning.

The consumer's conformance checker marks every case with a `subject`, and
computes its `conforms` over provider-subject cases alone. That is deliberate,
and its own source says why: a provider declaring a component kind the compiler
has no route for *"has satisfied every obligation v3 places on it; the gap is
ours, and calling it non-conformance would name the wrong party in the one
field people read."*

The reporting here collected every failed case regardless of subject. So a gap
that belonged to the consumer printed **REFUSED** against a provider the checker
had just passed, with zero provider-subject failures.

The cost was not a wrong line in a report. That number was written into a source
comment and into a baseline as a *measured fact about the protocol*, and it then
argued for reverting the declaration that had exposed the gap. A defective
instrument propagates further than a wrong belief, because a belief gets
challenged and a record does not.

The reader now separates the two subjects, and refuses a summary that
contradicts its own cases -- an instrument disagreeing with itself has not said
this provider is fine. It is checked by feeding it answers it must accept and
answers it must reject, including a provider failure hidden behind a passing
summary, and that check was seen failing on the original defect before it was
kept.

**Pi Coding Agent declares the `instruction` kind**, and the surface that routes it is
owned. Where the consumer carries no route yet, composition still refuses early
rather than late; what changes is that this side is no longer the thing being
waited on.

Also here: two Windows artifacts this product publishes were missing from the
artifact table, because the vendor ships an executable rather than an archive
and the guessed archive URLs answered 404. Each harness's toolkit now teaches
that harness's own components in that harness's own format. And every blank in
the software table carries the measurement that says why it cannot be filled
yet, rather than reading as a hole nobody looked at.

**Upgrading the Antigravity CLI setup system, and only that one:** if you keep
your own rules in `~/.gemini/config/rules/`, read this. That build now owns
`config/rules`, which the published `0.0.28` did not. Ownership in this program
means a namespace is managed whole, so on the next install your own files there
are captured into a backup slot. Nothing is lost -- `slot-000000000001` holds
them and a restore puts them back byte for byte -- but they will not be in place
afterwards, and no setup writes to that directory, so it is emptied rather than
replaced.

Measured rather than reasoned, by running both binaries against one target: a
hand-written `config/rules/MY-OWN.md` survives an install by the published
`0.0.28` and does not survive one by this release. Files outside the owned
namespaces are untouched by both, as they always have been.

It is the same rule that has always applied to `antigravity-cli/keybindings.json`
and to a skill directory you wrote yourself; `config/rules` simply joins them.
To keep your own rules across installs, a plugin's `rules/` is the customization
root that build's own setups use, and it is not emptied.

The other six setup systems are unaffected -- none of them owns a path of that
name. This entry is shared by all seven trees because they are rendered from one
source, which is why the paragraph above names the one it is about.

**Not new here, and worth saying because the version numbers disagree.** The
Windows digest fix -- a delete-pending path answers `PermissionDenied` rather
than `NotFound`, so the walk asks the path instead of trusting the error kind --
is already in the published `0.0.28`. The three-OS matrix in these trees caught
it on the `0.0.28` release pull requests, and the release branch was re-rendered
onto the fix before it merged. So the monorepo's `0.0.28` tag does not carry it
and every published `0.0.28` does. Nobody is waiting on this release for
Windows.

## [0.0.28] - 2026-08-29

A second target this program declared and could not operate.

Pi Coding Agent's provider publishes a projection profile per *scope* -- a second
target that is not the product's configuration home. The consumer gained the
request field that names one, so a scope could finally be asked for, and asking
for one found that every verb here answered about the **global** target
whatever scope it was handed.

Measured rather than read: a root holding two skills planned a backup whose
expected target digest was the digest of the empty string, and applying it
produced a backup slot with a record and no payload. **A backup that reports
success and captures nothing**, and therefore a restore that puts nothing back.

Seven places asked the same question and six had the wrong answer: the target
identity, the capture, the replace, the pre-flight that refuses what a capture
could not take, the ownership written into provider state, and the check that
decides whether a bundle writes inside the declared surface -- which refused a
component routed to a scope as *outside the surface*, so the scope this program
published could not be installed into. Recovery had no scope at all, because
`recover-operation` takes no arguments and nothing had written one down.

Two more the fixing turned up. A `backup` recorded an empty list of written
paths: globally harmless, because a removal reads the declared namespaces, and
under a scope destructive, because there the record *is* the inventory a later
removal acts on. And a scope this program declares no profile for was accepted
and silently treated as the global target; it is refused by name now, saying
which scopes are declared.

**Under a scope the namespace is the permission and the recorded files are the
inventory.** A root like `~/.agents` is read by several products at once, so the
capture takes this provider's own files and a restore leaves a neighbour's
exactly as they were. Taking the namespace whole would put somebody else's work
in this program's backup slot and revert it on the way out.

**Five of the seven setup systems declare that root now, where one did.** Each
measured against its own pinned product with the digest checked first, three of
them by running the product with a probe placed at the root and a control placed
at a sibling root no document names -- because a positive without a control
cannot be told from a program that scans everything. The four that had not
declared it were refusing on a sentence that was true when it was written and
had been false since the record of written paths shipped.

**Five readers in this repository had never seen that second target**, all of
them written before one existed: two checks that grade how a surface was
measured, the toolkit that teaches an author where a component goes, this
program's own support page, and the lifecycle probe. Repairing the first two
found six surface rows in shipped records that said nothing at all about what
had exercised them, under a check that had been green for weeks because it was
green about the rows it looked at.

A tree walk also stopped refusing when a file vanished between being listed and
being read. Two programs writing one target is ordinary; the second is refused
by a lock, and the reading that happens before that lock could meet a name that
was already gone. It said so by naming a filesystem instead of naming the lock,
and the race written to reproduce it found two further gaps of the same kind.

## [0.0.27] - 2026-08-29

A check that reports into an issue rather than failing had its
verdict recovered from an English sentence, and defaulted to *clean* when the
sentence did not match.

Two sweeps in this project's own workflow reach a vendor's server, so neither
fails the run -- a repository check depending on somebody else's uptime stops
being read, and that decision stands. What did not stand is how their results
travelled. The workflow matched the human summary with a regular expression and
substituted zero when the match came back empty, so **renaming one word in that
summary turns every failure into none**, opens no issue, and reports the sweep
as clean. Demonstrated rather than argued: one word, three failures, zero
reported.

Nothing anywhere said the prose was load-bearing. And the default was a copy of
a number the tool already had.

Both tools print a marker line for a machine and a sentence for a person now.
The workflow reads the marker, and **refuses when it is absent rather than
assuming zero** -- an absent measurement and a measurement of nothing are
different states, and only one of them is good news. A test binds all three
halves and was observed failing under each mutation separately: restoring the
default, renaming the marker, and softening the refusal.

**How it was found is the part worth carrying.** A peer project named a failure
mode neither of us had listed -- an instrument with no planted negative, whose
green therefore says nothing -- and the answer was to go through every check
here asking whether it had ever been handed something it must reject. One had
not.

The first attempt to hand it one was *wrong*: the document chosen as invalid was
something that schema happens to accept, and the checker was right to pass it.
Reading the schema to construct a genuine violation is what led to the line
above. **A failed injection is information about the injection first**, and the
pull to read it as information about the instrument is strongest exactly when
somebody is already hunting for a defect.

## [0.0.26] - 2026-08-29

One product shipped eight releases between one check and the next, and
the check's summary line nearly sent somebody looking for a broken instrument.

The pinned version moves from `1.0.5` to `1.0.13`, npm and the direct
distribution both, with the installer's own digest moving with them. Six
artifacts re-fetched and re-hashed against what the vendor publishes now.

**The summary said "7 pins behind" and that is seven *rows*, not seven
products.** One product's version appears in seven places in its record: the
artifact table, the package version, the installer's version argument, and four
native packages. Read as seven products it looks like every pin in the estate
went stale overnight; read correctly it is one vendor being busy. *A count is
only a count together with what it counts* — the same sentence as a remainder
reported without its denominator, which this project has now got wrong twice and
caught both times.

**Four of the seven can now cross a real version transition**, because a bump
assigns the outgoing pin to the second slot. `software_update` needs a version
to come *from* and a rollback needs a tree to return *to*; a product pinned once
has neither, so both operations were declared with nothing exercising them. The
remaining three fill on their next bump, which at the measured tempo is days.

Nothing else changes. The rest of this release is the version.

## [0.0.25] - 2026-08-29

A pass somebody did by hand, once per product, is an instrument now --
and the thing that makes it an instrument rather than a search is what it looks
for that must not be there.

Every declared surface in this record carries a value saying what exercised it:
a run, the product's own bytes, or a page alone. A page-only row is not
suspicious; it is **unfalsifiable from inside this repository**, which is worse,
and this project has shipped exactly one wrong fact of that kind -- a manifest
filename that had a citation, sat beside correct rows, and passed every check
here.

Thirty-one of those rows were moved off `page` by a person opening each artifact
and reading it. That pass now runs on a schedule: after installing a product
from its pinned bytes, the evidence job searches the bytes it just installed for
every namespace the provider declares, joined to the configuration home the
binary itself prints.

**Both of those are asked of their owner rather than copied.** The namespaces
come from the provider's own declaration and the home from its own first lines,
so a declaration that moves cannot leave this measuring the old one. It reads no
record at all -- which is the point, because a record checking itself is not a
check.

**A name nothing can own is searched alongside the real ones.** If it is found,
the run reports nothing else. *Everything matched* and *the search matches
everything* are indistinguishable from the outside, and feeding an instrument
something it must reject is the only cheap way to tell them apart.

**It reports and never promotes.** Recording a value still takes somebody
writing down what they measured, and a separate tool re-derives that value from
the row's own prose and refuses one stronger than the prose supports. Two
instruments sharing one opinion would be one instrument.

**A bare name is deliberately not reported as a count.** One directory name
appears sixteen hundred times in a product's bundle as identifiers, keys and
prose; printing that number beside an anchored eleven invites reading the larger
as the stronger. Where the anchored form is absent, the run says the bare name
proves nothing and gives the reason: the product may join that name to a
directory while it runs, in which case no literal exists to find. One product
reads zero of seven for exactly that reason and is not thereby suspect.

Four rows moved on the first run. One of them is a **single** occurrence and is
recorded as one -- weaker than eleven, stronger than a bare name, and rounding
it up is the promotion this column was added to prevent.

**And every tool a document names must now exist.** A sibling project found a
contract file asserting, in the present tense, that a validation script fails
when three lists disagree. That script had never been written, and the drift it
promised to prevent had already happened -- in the file that claimed to prevent
it. A reader cannot tell a described guard from a running one. This repository
had the rule for documents and now has it for executables, observed failing on
that exact sentence.

Its own first version compared the tail of a filename, and the linter refused:
a case-sensitive extension comparison answers differently on the three systems
this runs on. Another module here carries that correction, written for the same
reason. **A lesson recorded in one place does not travel to the next thing
somebody writes.**

## [0.0.24] - 2026-08-29

A field this build has accepted for a month is now declared, because
the consumer's released runner finally accepts it too -- and a second thing
went green with no change here at all.

**The one that changed nothing.** One harness's scoped profile has reported a
conformance failure since it was declared, and the cause was never on this
side: the vocabulary the consumer publishes for providers to build against
allowed the value, and the runner it released did not. The declaration was
correct throughout. It now reports twenty-seven of twenty-seven against a
release this build does not touch.

Withdrawing it to make a lagging checker print green would have meant
re-declaring it today and publishing a corpus in between that said something
false. That is the fourth time this ordering has settled a question between the
two projects, and the rule it produced is worth more than any of the four
occasions: **a published vocabulary blessing a field is permission to build
against it; a released runner accepting it is permission to emit it.** A key
that is merely recorded needs the first. A key the runner compares for exact
equality needs both.

**The one that took a line.** With the field declared against the previous
runner, all seven harnesses failed conformance to buy one -- so the constant and
the type sat in place and the declaration stayed empty. It ships now, and all
seven conform with it: twenty-seven to twenty-nine cases each, no failures.

**Two tests had to become more precise rather than looser**, and both had been
asking a narrower question than they looked like:

- One compared the published member set to the schema's required list for
  **equality**. That answers two questions at once -- are all required members
  present, and is every present member permitted -- but only while the build
  declares nothing optional. The new field is permitted and deliberately not
  required, so the equality broke the moment something legitimate was added. It
  asks both separately now, which is stricter: it was observed catching an
  injected member the closed schema does not allow, which the old form could not
  have told apart from a legitimate optional one.
- The other asserted that a declaration carrying no second scope contains no
  trace of a certain name, by searching the rendered text. **The new field's
  value is that name.** So the test found the thing it is about inside a field
  it is not about. It asks the structure now, and was observed failing when the
  scoped array is made to serialise unconditionally.

Neither was relaxed to pass, and the second is the near-miss worth recording:
the cheap repair is to delete the assertion, and deleting it would have silently
dropped the guarantee that adding a field does not move an existing profile's
digest. The wrong fix was cheaper than the right one and would have looked
identical in a diff.

## [0.0.23] - 2026-08-29

One vendor shipped Windows support and this estate had not noticed,
because the two places that would have said so both said the opposite and
neither was read by anything.

Its own installer fetches a Windows package at the version already pinned here.
Both architectures were fetched, hashed and extracted end to end -- 400 and 430
entries, 211 MB each, every entry's checksum verified against its own header.
The product now installs on all six hosts it publishes rather than four.

**The container is a property of an artifact now, not of a product.** That
vendor ships a ZIP on Windows and a gzip-tar on its five other hosts, in the
same release at the same version. A shape read off the product would have been
right for six harnesses and silently wrong for the seventh on one platform of
three -- which is the shape of both Windows defects this project has already
shipped, and the third time the same sentence has been the fix: the platform is
a parameter, not a compile-time constant.

**The launcher inside is not the one the vendor's installer implies.** That
script copies three names if each exists, and the first of them is not in the
archive at all. What actually runs is the batch file, which hands off to a
PowerShell script, which starts the runtime bundled beside it. The member
search picks it deliberately; the rule the tar search uses -- take the largest
match -- would have chosen the one launcher only PowerShell can start.

**A reader written to what was measured rather than to the format.** The
archive was examined before a line of it existed, and every refusal in it
refuses something that archive does not contain: the 64-bit extension,
encryption, deferred sizes, and any compression beyond the two it uses. Each is
refused by name, so the day a vendor starts using one the message says which.
Six tests, each observed failing under a mutation of the rule it describes and
no other.

**Two records were calling Windows unsupported while carrying a Windows
artifact.** Both were inherited from a retired line of repositories -- one names
it in its own prose -- and both had become false. The second was false about a
second product too, whose own package declares Windows among its systems and
publishes a binary for the architecture a third-party guide insists does not
exist. They stayed false because nothing read them. A check now compares every
such block against the artifact table beside it, and the rule is one sentence:
a record may not call an operating system unsupported while carrying an
artifact for it.

**That check's first version was wrong and the first run caught it.** It split
each entry at a hyphen and compared the head, so an entry naming a C library
read as naming an operating system, and two records were reported as
contradicting an artifact they agree with. The comparison is exact now, and the
rule says out loud that a C library, a distribution or an architecture is not a
claim about an operating system.

**Two products have no administrator policy layer, and finding that out took
looking.** Four of the seven carry one that overrides everything a user writes.
The record said nothing about the other three, which reads as a gap rather than
as a fact. Both were searched for the three shapes the other four use, in their
own pinned bytes, and neither has one. The second search is the one worth
keeping: that product does carry system-path literals, every one of them from
its runtime and its embedded browser rather than from its own configuration,
and a search that counted them would have invented a policy layer. So both
entries record what was searched rather than only what was concluded.

What that means is stronger than it sounds. On the four with such a layer, a
permissive posture can install, verify and restore cleanly while an
administrator's policy silently overrides it. On these two, the keys written
here are the last word.

Also advances one product's pinned version, with the outgoing one falling into
the second slot so a real version transition can be crossed.

## [0.0.22] - 2026-08-29

A posture that restates a product's own default grants nothing, and
two of the seven were doing it.

`full-auto` is meant to leave nothing asked, nothing sandboxed, and every
capability the product carries switched on. Three harnesses were brought to
that last week. This is the other four, each measured against the product's own
pinned artifact -- whose bytes were checked against this repository's recorded
digest before anything was read from them -- and the result was not four
products with tools left off. It was two of each.

**Two gained a capability that was genuinely switched off.** One product's own
default configuration object declares web-search auto-acceptance as false, so
the posture had been calling itself full auto while asking about a search every
time. Another names four inputs to its permission decision in its own runtime
log and this posture set two; the agent may now reach files outside the
workspace, see files the repository ignores, and continue rather than stop at
its invocation ceiling.

**Two were setting a value the product already used.** One product's permission
catch-all *is* its default -- with no configuration at all its resolved rule set
already allows everything -- and seven of its rules are the product's own, moved
by no key at any scope. Another's skill-command toggle defaults to on in
documentation shipped inside the package itself.

Both are kept. A posture that states what it wants survives the product changing
its mind, and removing them would leave the posture silent about the one thing
it exists to say. What changes is that each now records **which it is**, because
a setting that restates a default reads as a switch somebody threw, and the
reader stops looking for the switch that was not.

**How the two cases were told apart, since it is small and transferable:** write
the candidate keys into a configuration file, ask the product to print what it
resolved, and put an invented key in the same file. Without that control, a key
surviving proves nothing -- the command might be echoing the file rather than
parsing it. With it, eleven keys one product reads separated cleanly from six it
drops in silence, every one of the six a plausible spelling.

**One key was deliberately not written.** Its name is carried by the same
registry that produced the three that were, but its value is an enumeration and
which spelling the settings file takes could not be measured here without a
credential. A plausible value the product does not read is worse than an absent
one: it reads as configured and does nothing.

**Nine surface rows stopped being unfalsifiable.** The evidence column records
what exercised each declared surface -- a run, the product's own bytes, or a
page alone -- and a page-only row is not merely unverified, it is a row where a
wrong answer is invisible to every check in this repository. Six were answered
by literals in one product's pinned artifact, joined to the home its environment
variable resolves; two by driving products offline; one by a product naming its
own settings file on screen. Fourteen run, twenty-five bytes, fifteen page, from
twelve, nineteen and twenty-three.

**The gate stopped being red for a reason belonging to no branch.** Its render
check compares the published trees against this source, which is a property of
the default branch: a branch that legitimately changes code has published
nothing yet. The entry point asked that question unconditionally while telling
you to run it before opening a pull request, so it was red on every branch that
changed anything. It picks by reference now and says which question it asked
and why.

Also advances this product's pinned version by one release, and with it gains a
previous version to move back to -- so a second harness can now cross a real
version transition end to end rather than declaring an operation nothing
exercises.

## [0.0.21] - 2026-08-28

A rollback on Windows looked for the Linux executable and refused a
version tree that held the right file all along.

The lookup tried two shapes: the member this build's artifact table names, and
the bare command. The first comes from a helper that answers with the **first**
artifact's member whatever host is asking, and every table in this source lists
Linux first. So on Windows it looked for `package/bin/<command>` while the file
on disk was `package/bin/<command>.exe`, and the tree that had just been
installed successfully was reported as holding no executable.

Install, update and the byte-exact restore round trip all passed on the same
run. Only going *back* failed, and only on one of the three platforms.

**The platform is a parameter now and not a `cfg!`.** The same file already had
that rule for the exposed command's name, written after a mutation deleted its
Windows branch and left every Linux run green. The lookup did not follow it, and
this is what that costs: a defect that Ubuntu and macOS cannot see, in a code
path that only runs when somebody is already trying to undo something.

This host's own member is tried first now -- right by construction rather than
by the table happening to be ordered well -- and the older shapes after it, so a
tree written by an earlier build is still found.

**The other harness's managed configuration is recorded**, which the research
that found the rollback bug turned up. Its directory is a system path, one per
operating system, and an administrator's configuration there is loaded at the
highest priority tier and overrides everything. This provider can install,
verify and restore a permissive posture cleanly on such a machine and change
nothing about what the product permits. Recorded and never touched. Its *user*
configuration home, by contrast, has no platform branch at all -- one resolver,
three operating systems -- which is why only the managed row needed writing.

**The permissive posture now grants capability, not just silence.** It set
approvals and the sandbox and stopped there, which is half of what the name
promises: those decide what the product asks you and what it confines, while a
separate set of keys decides which tools exist at all. On one harness four of
them were off by default and the posture left them off, so *full auto* meant no
questions and no web fetch.

That harness now enables the six feature toggles its own binary lists in its own
table, plus subagents, cross-session memory and managed MCP servers -- and the
product's separate `yolo` switch, which is a real key beside the approval mode
rather than an alias of it. Another gains the most capable of its four web
search modes and its local-image tool. A third approves project MCP servers
without prompting.

**Two toggles were deliberately left alone.** Telemetry and feedback are
switches on the same list, and they send data outward rather than handing you a
tool; a posture that flipped them while claiming to grant capability would be
doing something else. And three feature flags the vendor's own reference
documents are not set, because the pinned build carries none of those
spellings -- writing them would set keys it ignores.

**What is claimed about these keys is what was checked.** Each name comes from
the product's own bytes, and each file is accepted by the product. That they
take effect is not demonstrated: neither harness reports its feature state, so
there is no instrument here to read it back from. The evidence column in the
declaration makes that distinction for surfaces, and it is the same distinction
here.

Pi Coding Agent


## [0.0.20] - 2026-08-28

This product's nine surfaces were a vendor page each. They are read out
of the product now, where the product says so.

The pinned artifact was fetched and its bytes matched this baseline's own
sha256 exactly. It carries an embedded reference table, the same way one other
harness in this estate does, and five rows are named in it: the skills
directory, the MCP configuration, the hooks file, the global workflows
directory, and the settings file. Those five say `bytes` now rather than
`page`.

**Four are not named there and keep the weaker value.** A path a program builds
by joining a directory to a name leaves no literal to find, so the absence
argues nothing in either direction -- and the rule for a row with no recorded
method is the weakest value, not a guess. Reporting them as confirmed because
the other five were would be the ranking error this column was added to remove.

The same reference names two paths this record had explained only inside a
neighbouring row's note: the workspace tier of the workflow surface, beside the
global one this provider owns. A note on another row is not where a reader
looks before opening a file to find out what it is. They are declined rows now,
with the reason on them.

Across the seven: **12 run, 19 read out of the product, 23 resting on a page
alone**, from 2 and 10 and 42 two releases ago.

Pi Coding Agent


## [0.0.19] - 2026-08-28

A citation is not a measurement, and this declaration had been
presenting them as one column.

`decided by` said where a row came from. Nothing said whether anybody had made
the product demonstrate it. A vendor URL sat beside a bare *measured in the
pinned binary* with no ranking between them, and a reader takes the URL as the
stronger -- **which is backwards.** The `agent` route this estate carried for
codex until the release before this one had a live vendor page behind it and
did not exist. Antigravity's `agents` route was correct throughout the weeks
its citation answered 404. The row that came out of a binary's own embedded
reference was right.

So every owned row now records `evidence`, on the only axis that predicts
whether a row is true:

- `ran` -- the product was run and the behaviour observed;
- `bytes` -- the product's own shipped bytes were read, an embedded reference
  or a path literal in the binary;
- `page` -- a vendor page, and nothing else.

**Where a row records no method the value is `page`**, because absence of a
record of measurement is not evidence of measurement. That rule is what makes
the column worth reading, and the first count it produced was unflattering: 2
rows run, 10 read out of the product, **42 resting on a page alone**, with two
harnesses carrying not one exercised row between them.

So the column was used before it shipped. Two of those harnesses were taken
apart against a temporary configuration home: one had its skill, agent and
command routes each confirmed by running it and reading back what it resolved;
the other refuses to start without a credential, and cannot be run here, but
ships its own documentation inside the pinned package -- which is the product's
bytes rather than its website, and confirms all six of its rows. Doing that
also turned up four paths its own documentation names under the home this
provider configures and that this record mentioned nowhere: two package
directories the product clones into and cleans, a model catalogue cache, and a
debug log. All four are declined now, with the reason.

A third was taken apart with the command it ships for exactly this -- *show the
configuration this product discovers* -- against a temporary home holding one
marker component per declared directory. Seven of its rows came back named, and
the eighth **found a fact this toolkit had been shipping wrong.** The plugin
manifest is `plugin.json`. This estate's authoring reference said
`.grok-plugin/plugin-index.json`, which is what a third-party page says and
what the product does not read: the same directory registers as a plugin with
the first file and as nothing at all with the second, and the product says why
in its own words -- *"no plugins found in the source (no plugin.json or
convention components)"*.

That is the column earning its place on the day it shipped. The wrong filename
had a citation, sat in a table beside correct rows, and passed every check in
this repository, because no check here can ask a product a question. Only
running it can.

The count is **12 run, 15 read out of the product, 27 on a page alone**, from 2
and 10 and 42. None of those 27 is known to be wrong. Every one is untested,
and from here the two are indistinguishable -- which is what the release before
this one demonstrated, and what a page-sourced manifest filename demonstrated
again this week.

The generated surfaces table carries both columns and counts them, so the
number is in front of whoever opens the toolkit rather than in a plan nobody
opens. A guard refuses an owned row that does not record one, and refuses a
value outside the three so the field cannot drift into free text.

**`scripts/check_citations.sh` asks the smaller question a person had been
answering by hand**: does every cited page still answer. It is deliberately not
in the gate -- a check that depends on seven vendors' websites being up goes
red for somebody else's outage, and a check that is red for reasons nobody can
fix stops being read. It reads citation fields only, because an earlier pass
that swept every URL in the baselines reported two dead and neither was one: a
URL template that cannot resolve by construction, and a URL quoted inside the
note recording that it had rotted. **A checker that reads prose reports the fix
as the defect.** Refusals that are not answers -- 401, 403, 405, 429 -- are
inconclusive rather than dead, or the one finding that matters is buried in
thirteen that do not.

What it cannot check is in its own output: a page that still answers may have
been rewritten to describe something else, and no fetch detects that. The
`exercised by` column is the answer to that question, which is why both shipped
together.

Pi Coding Agent


## [0.0.18] - 2026-08-28

The builder toolkit learns to teach component authoring, and in reading
seven vendors' formats side by side it found that one of its own routes did not
exist.

**A file in this product's agents directory may be loaded by nothing.** Codex
does not scan that directory. A role is declared in the settings file as an
`agents.<name>` table whose `config_file` points at a TOML layer, and the
pointer resolves from the declaring file. Measured by running the product
against a temporary home: a bad pointer is reported --

    Ignoring malformed agent role definition: agents.broken-role.config_file
    must point to an existing file at .../agents/missing.toml

-- while a Markdown agent sitting beside it was loaded by nothing and
complained about by nothing. `codex-setup-system` had declared `agent` for that
directory since it was written, and its own builder setup had never run.

The kind is withdrawn there, and the reason is arity rather than behaviour: a
role is two files, a component of one kind is one thing installed in one
namespace, and there is no way to say *and also add a stanza to the settings
file*. That reason survives a change of behaviour -- if the product started
scanning the directory tomorrow, a role would still be a stanza plus the layer
it points at. The directory stays owned,
so a backup still captures it and a restore still returns it, and it routes
nothing. This release's builder is delivered as the pair the product reads.

**Two more declarations were short a row, both measured against the pinned
artifact.** Grok reads `personas/` and `roles/` and neither was recorded as
owned or declined; both are now owned and route no kind, because a behavioural
overlay is not an agent and the closed kind set has no word for one. Opencode
accepts the singular and the plural spelling of four directories -- its own
embedded reference writes them as `agent(s)`, `command(s)`, `skill(s)` and
`plugin(s)` -- and this provider owns the plural only, so the singular is now
declined with the reason, rather than being a path the product reads and the
declaration never mentions.

**What the toolkit gained.** One authoring reference per kind each harness
routes -- skills, agents, commands, hooks and plugins -- generated from the
vendor's own reference and from the pinned binary, plus a table of the same
rows across every harness in the estate that routes that kind. The table is the part no vendor documents: `name` and `description`
travel everywhere, and `allowed-tools` is honoured on three products and read
past in silence on a fourth. A field absent from a column is not rejected there.
Where it was carrying a restriction, the restriction is gone and nothing says
so.

**Plugins are the kind where the products least resemble each other**, and the
references say so rather than flattening it. On one harness a plugin is a
manifest bundling components the product already understands and runs no code
of its own. On the two others it is a module loaded at startup -- one of them
documents plainly that an extension runs with full system permissions, and the
other loads every file in the directory with no manifest to opt one in and no
field to disable one. A setup that carries a plugin is carrying a program on
two of the three, and that belongs in its description. No table is drawn across
those two shapes: they route the same kind and share no row, and tabulating
them would invent a comparison the products do not have.

**The check that had to be written to add the seventh toolkit found two more.**
Two of these trees carry a hand-written entry point, so the generator writes
references beside it and cannot add the line that routes to them. It now
refuses unless that line is already there -- and on its first run neither entry
point named a single one of the files being written beside it. They had been
shipping unreachable. Every existing guard asks whether a named file exists;
none asked whether an existing file is named.

Correcting one of them turned up two more sentences that had outlived their
measurement: it said this product has no global command surface, which this
declaration itself refuted on 2026-08-28, and it described a plugin's shape
from the harness next door rather than from this product's own page, listing a
directory that is not read and omitting two files that are. Both now carry the
date they were corrected.

**A guard got stricter rather than looser to allow the new pair.** A surface
that routes no kind may now hold setup files if its row names the file that
points at them -- and the guard opens that file and requires the path to be in
it. A pointer claimed and absent reads as routed and is inert, which is the
defect the guard exists to catch.

Pi Coding Agent


## [0.0.17] - 2026-08-28

A guard that found three defects in the release before it, all of one
kind and all invisible to every other check.

**An instruction naming a file the setup does not ship.** A skill's routing
table sends a reader to `references/surfaces.md`; an agent names the document
beside it. If the setup does not carry that file the reader is sent nowhere --
and the reader is a model, which will not say so.

`0.0.16`'s own generator did it three times, and each was correct for six
harnesses and false for the seventh:

- codex's agent pointed at `references/surfaces.md`, and codex ships no skill at
  all: its `skill` kind routes only under `target_scope: user_root`, so a setup
  aimed at its own configuration home carries no `references/` directory. Its
  agent now says that, and sends the reader to the binary instead;
- codex's `nddev-surfaces` command did the same, and now omits the hint;
- and a path was wrapped across a line *inside backticks*, so the quoted text
  contained a newline and named `references/
lifecycle.md` -- a file that could
  not exist anywhere.

**And each owned surface now carries the page that decided it**, beside its path
and kinds, so a reader can check any row against the thing that settled it. A
table of paths without its sources is a list somebody could have guessed; the
declined rows already carried their reasons, and now the owned ones carry theirs.

**Narrowing it was most of the work, and the distinction is the useful part.**
Describing where a product reads is not naming a file to open, and the
difference is visible in the path: a glob or a placeholder names a *class* of
file, a `~`-rooted path is outside the setup, a leading dot-directory is the
product's own home, and an environment variable as the first segment is a root
the setup does not contain. Cursor's hand-written references are full of all
four, and the first version of this guard called every one of them dangling.

That is the same distinction the command guard already drew between an
invocation and prose, applied to paths instead of verbs.

**And the match is a suffix, deliberately.** A relative reference does not
resolve the same way in every product -- antigravity's plugin rule names a path
relative to the *plugin root*, a skill's routing table names one relative to
itself. A guard that picked one convention called the other broken, and the file
it named was shipped. So it asks the question it can answer without guessing:
**is this document in the setup at all?**

## [0.0.16] - 2026-08-28

A builder toolkit on every harness, a coupling nobody was checking, a
green that now names what produced it, and a correction to what 0.0.14 said
about cursor.

**`nddev-builder` on all seven.** Two harnesses carried a maintainer's toolkit
and five did not, so the components published for those five had no source in
these repositories at all. Each of the five now ships one, and *what it carries
differs because what each harness routes differs*:

- `codex` routes `skill` only under `target_scope: user_root`, so a setup aimed
  at its own home cannot carry one -- it gets an agent and prompts instead;
- `pi` routes no `agent`; `grok` owns a command surface that routes no kind,
  because slash commands there surface as skills.

Shipping an identical tree everywhere would mean writing components into
directories the product does not read, which is the defect `0.0.11` removed.
Being native to each harness *is* the consistency; an identical layout would
only look like one.

**Half of each toolkit is derived, not written.** *What this harness owns and
why* already exists, measured, in its baseline: every surface with the page that
decided it, every declined row with what was searched, and the configuration
file's grammar. `tools/build_nddev_builder.py` renders that into the toolkit and
`--check` runs in the gate, so a baseline edited without re-running it fails
rather than leaving a shipped toolkit describing surfaces the harness no longer
declares. A stale toolkit is worse than none, because a reader trusts it.

**Cursor and antigravity keep their hand-written toolkits and gain the derived
half.** Their prose is better than a template -- antigravity's names the one
problem unique to it, that the product is a *guest* in Gemini CLI's home and a
write one directory too high succeeds and corrupts another product's
configuration. So nothing of theirs is overwritten; only the derived surfaces
table is added, and for antigravity the lifecycle and validation references it
did not have.

**A guard written because this repository's own generator produced the defect.**
The first draft of that addition wrote references into a `skills/nddev-builder/`
directory of a harness whose skill is called something else -- a `references/`
folder no entry point reaches. Every existing guard passed it: the files are
documents, so the sourcing rule exempts them, and there is no `SKILL.md`, so the
description rule has nothing to check. The absence was invisible *because* the
thing that would have been checked was the thing missing.
`catalog::unreachable_references` refuses it now, and the generator refuses to
write there at all.

**One guard was wrong and is fixed.** `catalog::misdirecting` refuses a shipped
instruction that tells a reader to run a command the binary does not answer --
and it knew only the *human* verbs, so it called `provider-info` refused. That
is a wire command this build answers on demand and one a toolkit has every
reason to document. A guard naming a working command as refused is the same
false statement it exists to catch, one level up. It now asks both surfaces, and
still refuses a verb neither answers.

**One of eight, not all eight.** `0.0.14` added a line to `--help` saying that
`XDG_CONFIG_HOME` moves cursor's configuration home. Tracing which surfaces
actually reach that resolver shows **exactly one of the eight this build owns**
does: `cli-config.json`, built as `join(configRoot(), "cli-config.json")`.
`commands`, `rules`, `hooks.json`, `mcp.json` and the `plugins` pair come from a
literal `join(homedir(), ".cursor", ...)` and reach neither the config root nor
the data root, so no variable moves them.

The config root carries `acp-config.json`, `acp-sessions`, `chats`,
`permissions.json` and `statsig-cache.json`; the data root, which honours
`CURSOR_DATA_DIR` and is not XDG-aware, carries `projects` and `computer-use`.
None of those seven belongs to this provider.

So the line now says what it measured: XDG moves `cli-config.json` and nothing
else this build owns. `documented_config_home` stays `~/.cursor`, which was
right for seven of eight throughout. The defect was reading a resolver and
writing a note about a home -- a measurement of one thing stated as a fact about
another, which is the shape this estate has found four times this week.

The GDS anchor in each of these trees is generated, and its shape is fixed by a
schema in another repository that sets `additionalProperties: false`. That is a
real coupling in one direction: a **narrowing** there turns every anchor in this
estate invalid at once, and nothing here would have noticed. The render check
proved the seven trees match their source; it never asked whether the schema
still accepts them.

Each published GDS anchor is validated with `gds validate repository`,
beside the `actionlint` and `zizmor --persona=auditor` passes already run
over the workflows. Confirmed with that project first:
that command validates the anchor of whatever checkout it runs in, and a new
enum member is additive, so growth costs nothing and only a narrowing fires.

**It prints the validator's version, and that is not decoration.** The `gds` on
the machine that wrote this is a development build predating the schema release
it validates against -- and so was the one on the other side of that
conversation. A green that does not name its validator is worth less than one
that does. Following the same convention as the two checks above it: an absent
tool is said on stderr, never skipped quietly, because a check that silently
passes is worse than no check.

## [0.0.15] - 2026-08-28

Three things that were correct and held correct by nothing, found by
sweeping the format axis across seven products and three operating systems.

The sweep itself produced almost no defects, and that is the result: what each
product's configuration file *is*, what its parser accepts, and what this
repository writes into it all agree. All 45 configuration files across the 23
setups parse; none carries a byte-order mark; no two owned paths and no two
files in a setup collide on a case-insensitive filesystem; every component entry
point carries the frontmatter its kind needs, and every file that does not is
one that should not.

What it found was three rules the estate follows and does not enforce.

- **Two owned paths, or two files in one setup, that fold together.** macOS and
  Windows fold case by default, so `skills` and `Skills` are one path there and
  two on Linux -- the same declaration meaning different things per platform,
  and a setup installing different content per machine. The bundle reader has
  refused exactly this for an *arriving* bundle since 0.0.11; these are the same
  rule applied to what this repository authors. Scoped namespaces fold against
  global ones, because a filesystem does not know about scopes.

- **A component entry point that cannot describe itself.** A `SKILL.md` whose
  frontmatter lost its `description` installs, verifies and restores cleanly,
  and the product then names it after its directory and gives the model nothing
  to choose on. Which files are entry points is measured rather than assumed,
  and the negative half took the measuring: files under `references/` are
  documents, and files under `commands/` are exempt because cursor's loader
  names a command after its **filename**, with `.md` stripped, and reads no
  frontmatter at all. Requiring it there would be inventing a rule the product
  does not have.

- **What a file at an owned path actually is.** The baselines recorded paths,
  kinds, shapes and sources, and never the grammar. Each now carries a
  `configuration_format` block: the file, whether it is JSON, JSONC or TOML,
  whether the parser accepts comments, the vendor's schema URL where one exists,
  and how it was measured. opencode is JSONC at both spellings; cursor's own
  reference says its file admits no comments; five of the seven vendors publish
  no schema at all. Bound to the declaration -- a grammar recorded for a file
  this build does not own is a measurement about somebody else's file.

## [0.0.14] - 2026-08-28

One line a person reads before choosing a target, which for one
harness was conditionally false.

`--help` prints the product's documented configuration home. Six of the seven
resolve one home from one variable and that line is simply true. Cursor does
not: its resolver reads `CURSOR_CONFIG_DIR`, then falls through to
`XDG_CONFIG_HOME` joined with **`cursor`** -- not `.cursor` -- and only then to
`~/.cursor`. Confirmed at the line in the pinned bundle.

So on a Linux machine with XDG set, a person reading that line would point
`--target` at a directory the product does not read. The line now carries its
condition, and a guard binds it to the baseline that measured it, the same way
the home and its environment variable are bound.

Nothing operational changed. The variable a `launch` sets is the first branch of
that resolver, so it already won; this was a true statement missing two thirds
of itself.

## [0.0.13] - 2026-08-28

A defect that could stop the product from starting, found by running
this build against a machine an administrator manages.

- **An install deleted an administrator's signed policy and kept its
  signature.** Measured on the shipped `0.0.12` binary, against a grok target
  holding a managed home: `install` removed `managed_config.toml` and
  `requirements.toml` and **kept** `managed_config.sig.json`,
  `managed_identity.sig.json` and `managed_config_cache.json`.

  That is exactly the state the product's own gate refuses -- its
  `managed_cache` carries *"refusing session -- the signed is-managed claim
  requires an authentic policy sidecar and none is present"* and *"refusing
  session on tamper evidence"*. A `restore` brings the policy back, measured;
  the person's next run happens before that.

  The harm was the **split**: the policy was owned and its proof was not, so an
  operation took one and left the other. All five names are in `never_touch`
  now -- not deleted, not captured into a backup slot, not hashed into an
  identity. An organisation's signed policy in a backup slot is the same shape
  as a credential in one.

  **The previous reason for owning it was circular** and is worth stating
  because it read as careful: *owned so a backup returns it byte-exact after an
  operation touches the home* -- when the only reason an operation touched it
  was that it was owned.

  Held by a guard that refuses to own anything whose name reads as an
  administrator's policy or the signature over one, matched on the path and
  never on the row's prose.

- **Three products write outside their configuration home, and only one was
  recorded.** Measured by running each in a clean `HOME` with the XDG variables
  cleared: opencode creates `~/.local/share/opencode`, `~/.local/state/opencode`
  and `~/.cache/opencode/bin`; cursor creates `~/.cache/cursor-compile-cache`.
  Neither had a row anywhere. Recorded as prose, because every recorded path is
  relative to the target and these are not.

- **Cursor's configuration home has two overrides, and the second renames the
  leaf.** `CURSOR_CONFIG_DIR`, then `XDG_CONFIG_HOME` -- which resolves to
  `$XDG_CONFIG_HOME/cursor`, not `.cursor`. The variable this build sets is the
  first branch of that resolver, so a launch still wins; the record now says the
  whole thing rather than a third of it.

- **An enterprise policy layer can silently defeat a posture**, on claude and on
  grok. `managed-settings.json` sits at three per-OS system paths and *"cannot
  be overridden"*; `full-auto` writes correct keys at a correct path that a
  higher layer outranks, so on a managed machine it installs, verifies and
  restores cleanly and changes nothing. Nothing to own and nothing to check --
  written down where a person reads the posture.

## [0.0.12] - 2026-08-28

Two operations that were declared and had never run, and two kinds a
run corrected in opposite directions.

- **A second software pin, so `software_update` and `rollback` can be
  exercised.** A build pinning one version has nothing for an update to move
  *from* and nothing for a rollback to return *to*, and this repository recorded
  that as a measured absence rather than testing them against a fixture.

  The second pin is not a second choice: a bump assigns `previous = current` and
  then sets `current`, so one value still moves per bump and the pair is always
  two consecutive real releases -- differing in whatever the vendor actually
  changed, which is the transition a person really performs.

  Run end to end against opencode's own bytes: `1.18.24` installed, updated to
  `1.18.25`, rolled back, and forward again, both trees kept throughout. The
  evidence job crosses the pair on every run on all three operating systems, and
  prints a reason and skips for a harness not yet bumped.

  `apply` resolves which release the bytes are from **the digest of the file it
  was handed**, not from a flag, so a relabelled argument cannot install one
  version under another version's name.

- **`written_paths` in provider state, and a shared root's removal scoped to
  it.** `native_ownership` records namespaces -- what a backup captures and what
  a remove takes -- and under a root several products read, those namespaces are
  several products' worth. Measured on a real install of each: `grok-build` owns
  twelve namespaces and wrote two files; `antigravity` owns nine and wrote one.

  So `remove` under `target_scope user_root` now takes the files this provider
  recorded writing and leaves everything else, including a neighbour's file
  inside an owned namespace. Directories are left standing. The refusal stays
  for a state file that is absent or at an older schema -- *this build does not
  know what it wrote* -- because widening to the namespace there is exactly the
  removal the branch prevents. The state schema moves to 4 with the field, so a
  record from before it cannot be read as one that wrote nothing.

- **grok's `command` kind withdrawn, one week after it was declared.** A file at
  `~/.grok/commands/<name>.md` is loaded and `grok inspect` lists it under
  **Skills**; a file under a directory nothing routes to is not listed, and
  removing this one removes the entry. The product's own reference puts
  `skills/` and `commands/` in one row, *"Personal skills for all projects"*.
  The namespace stays owned because it is read; what comes out is the promise
  that a component routed there stays a command.

- **cursor's `skill` kind declared.** It had been declined on a vendor page
  about the plugin-manifest key that does not mention the directory. The
  product's bundle carries a skill-root table joining `.cursor/skills` to the
  home directory at user scope, and its own ignore file calls it *"User's
  personal skills"*. `skills-cursor` and `cloud-skills` are recorded as declined
  instead -- the first is the product's own built-in set, the second is filled
  from the account.

Both corrections have one shape: **a declaration can refute a route and cannot
confirm one.** Reading found the directories; only running said what they were
read *as*.

## [0.0.11] - 2026-08-28

The release where seven products were read rather than their pages,
and the record lost an argument to every one of them.

- **Nine surfaces declared that the products have and this provider did not.**
  Each had been declined on the strength of a vendor page that does not mention
  it. cursor's own rule picker offers a *User Rule* scope at
  `join(homedir(), ".cursor", "rules")`; it calls
  `loadCommandsFromDirectory(join(userHomeDirectory, ".cursor", "commands"))`;
  it resolves user paths for `hooks.json` and `mcp.json`. That harness declared
  two kinds and now declares six. grok's own embedded reference lists
  `~/.grok/commands/` at User tier, beside the already-owned `skills/` in the
  same row. codex's `~/.codex/agents/` is vendor-documented and was undeclared.
  antigravity's `config/global_workflows/` holds Markdown invoked as
  `/workflow-name` across all workspaces -- which its own declaration comment
  had said the product does not have.

  Widening is the safe direction: a consumer matches a route by membership, so
  a larger set makes more routes valid and none that were valid invalid.

- **`plugin` withdrawn from claude-code, because it was destructive rather than
  merely unroutable.** It was the only kind across the seven routed to a *file*
  -- `enabledPlugins` and `extraKnownMarketplaces` are keys in `settings.json`.
  There are no merge semantics here: `write_bundle_files` calls
  `remove_managed` and then writes bytes verbatim, so a `plugin` bundle
  carrying that file would replace it *and* delete `CLAUDE.md`, `skills`,
  `agents`, `commands` and `rules` on the way in. Declaring it again means
  building settings-merge first.

- **Two credentials files nobody disclaimed.** Five of seven named theirs;
  grok's `auth.json` (*Authentication credentials (auto-managed)*, in its own
  reference) and pi's were not in `never_touch`. No live leak -- a capture walks
  the owned namespaces and neither file is inside one -- but that is safety
  resting on a namespace never widening. A guard enforces it now, and it caught
  nothing until both baselines recorded the files it reads.

- **A scope that reaches removal, and a removal that refuses rather than
  guesses.** `--target-scope` is accepted a release ahead of anything reading
  it, travels in the plan, and is read back from the plan by `apply` -- never
  from argv, because a flag on both would be two statements of one fact. Under
  `user_root` a removal is **refused**: `ADR-0127` requires it be scoped to what
  provider state records, and state records *namespaces, not files*. Four of the
  seven products read `~/.agents/skills`, so a whole-namespace removal there
  takes three neighbours' content. A refusal a person can read is smaller than
  that, and far smaller than a removal scoped by a guess.

- **The container base was named by a tag.** Everything else here is pinned;
  `gcr.io/distroless/cc-debian12:nonroot` was not, and a republished tag leaves
  no trace, unlike a stale pin which shows as a version going backwards. Pinned
  to the digest the registry computed.

## [0.0.10] - 2026-08-28

The release that makes a convention's own root reachable, and the one
where two validators stopped being two.

- **`user_root`, and codex declares it.** Codex reads user-level skills from
  `$HOME/.agents/skills` -- a *sibling* of `~/.codex`, not a child -- so nothing
  declared against a product's configuration home could reach it. The kit at
  `0.2.4` carries the scope; this build declares `skills` under `~/.agents`,
  kind `skill`. The path is `skills` and not `.agents/skills`, because the root
  is what the scope names.

  `.agents` is named for being shared and an owned namespace is removed whole,
  so this was weighed rather than assumed: measured across all seven baselines,
  only Codex documents reading from the user-level root, and Antigravity's
  `.agents` surfaces are workspace-level with its global configuration
  elsewhere. The declaration says what to re-read if a second product adopts it.

  Four of the seven do, and the sweep that found them read products rather than
  pages -- pinned artifacts, digests verified before reading. Grok's own embedded
  reference scans `.agents/skills/` at each tier; OpenCode's vendor lists it as
  *Global agent-compatible* and the binary carries the literal; Pi loads from it
  at `package-manager.js:2017`, where no Pi page says so.

  Codex's declaration stands, and none of the other three is given the same
  scope: a namespace is removed whole, so providers declaring one path are not
  several owners. The same sweep found `~/.claude/skills` read by Grok and
  OpenCode as well as by Claude Code. All recorded in the baselines, and raised
  with the consumer, whose scope this is.
- **One digest is one installability.** A bundle whose paths cannot be written
  on Windows installs on two systems out of three, and nothing in its digest
  says which two. Refused now: a segment whose stem is a reserved device
  (`NUL.tar.gz` is `NUL`, and so is `nul`), a trailing space or period on any
  component, a colon anywhere in a segment, and the characters Windows reserves
  inside a name. Taken from the consumer's own predicate rather than written
  beside it -- a provider stricter than the compiler refuses bundles the
  platform already blessed.
- **`software_remove` is exercised.** Every build declares four software
  operations and the evidence job ran two of them: it installed a real product
  and left it there. It takes the program back off now, on every path, and asks
  the prefix to agree. `software_update` and `rollback` stay unexercised for a
  measured reason -- each harness pins exactly one version, so there is no
  second tree to move a command between.

## [0.0.9] - 2026-08-28

One product could not be installed on one platform, and the check
that found it had been running for a day.

- **A pax extended header is read rather than refused.** The extractor accepted
  regular files and directories and refused every other type flag, on the
  ground that no entry should be able to redirect a later write. True of hard
  links, symlinks, devices and GNU long link names. Not true of `x` and `g`:
  they are records of metadata for the entry that follows, and they create
  nothing. Refusing them made Cursor's macOS package unreadable, so
  `software_install` could not work there at all.

  What can move a write is a record that *overrides* something the reader acts
  on, and `path`, `linkpath`, `size` and `GNU.sparse.*` are still refused by
  the key they carry. Everything else is skipped. Measured against the real
  package: 120 directories, 406 files, two pax headers holding Apple's
  code-signing xattrs and no `path` at all -- and it now extracts to 526
  entries.

  Records are parsed by length, not by line. The two real ones hold DER, which
  is full of newline bytes; a parser that split on newlines would read a
  different archive than the one it was handed, and would do it quietly.
- **`evidence` tells an honest refusal from a failure.** Cursor publishes no
  Windows build and the provider says `unsupported_platform` by name rather
  than planning something it could not apply. The job reported that as a red,
  which would have taught people to ignore its reds.

## [0.0.8] - 2026-08-27

The release that opens a window, and asks the shipped bytes a question
the source was already being asked.

- **`plugins` and `plugins/local` are both declared, on purpose.** Cursor reads
  a local plugin from `~/.cursor/plugins/local/<name>/`, which is where these
  setups write and what `0.0.7` corrected. The consumer validates a route by
  exact membership in `native_namespaces`, so there is no order in which one
  side can move alone: whichever moves first refuses every install against the
  other. One release naming both opens the window, and the older name goes
  after the route has moved.
- **A declaration and a cover were sharing one name.** Declaring both was
  refused by this repository's own invariant -- a namespace inside another
  would be hashed twice -- which was right about the hash and wrong about the
  list. `digest::of_owned` now reduces the declaration to a cover before
  walking it, so identity no longer depends on how a declaration is phrased.
  Measured: a target installed against a build declaring `plugins` alone reads
  the same digest, with no drift, under a build declaring both.
- **A `boundary` job asks the artifact what `clippy.toml` asks the source.** No
  network symbol is imported, and a build that declares no `launch` imports
  nothing that could spawn. Both observed failing first: against `curl` the
  network step prints `socket`; given a reachable `Command`, the binary that
  declares no launch grew `execvp`, `fork` and `posix_spawnp` at once.
- **`SUPPORT.md` states the boundary and what it does not buy.** This program
  imports `syscall` like any other, so no property of the binary proves a
  socket is unreachable to code determined to open one. The honest claim is
  narrower: nothing here reaches for the network, nothing can be added without
  the build refusing, and no local phase can hand the job to a child.
- **The surfaces guard states its own limit.** Both sides of that comparison
  are written in this repository, so it catches drift and never shared error --
  which is what produced every defect it was built after.

## [0.0.7] - 2026-08-27

The release where the declaration became true, and a round trip
started proving it on the systems that had only ever compiled.

- **Every surface a provider owns now cites the document that decided it.**
  `native_namespaces` and `component_kinds` had been assembled from a
  consumer's routing table rather than from the products. Measured against the
  vendors: cursor owned six paths its CLI does not read and declared five kinds
  it could not install; claude-code owned a `.mcp.json` that does not exist,
  because user-scope MCP lives in `~/.claude.json`; codex owned `.agents/skills`,
  which is a *sibling* of `~/.codex` and resolved to a directory nothing reads;
  grok owned a `commands` directory the product does not have, because its slash
  commands are its skills. In the other direction, pi was missing `prompts` and
  `themes`, grok was missing `workflows`, and opencode was missing `tui.json`.
  Each baseline now carries a `native_surfaces` block -- one row per surface with
  the URL that decided it, and every path considered and not owned with its
  reason -- and a test binds the declaration to it in both directions.
- **A round trip against the built binary, on whichever system runs it.** The
  three-OS matrix proved the code compiled and its unit tests passed on macOS
  and Windows. It never proved a *target* survived install, select, backup,
  hold, restore, restore-to-a-named-slot, release and remove there. It does now,
  in every tree, with a file the provider does not own present throughout and
  compared byte for byte after every command.
- **A file deleted in the source survived in the published tree forever.** The
  render merged and never removed. Measured: this repository had been publishing
  1053 lines of source nothing compiles since `0.0.1`.
- **Antigravity owns a second target.** Five workspace surfaces under `.agents/`,
  each read from a vendor page. `command` and `instruction`, which the consumer
  asked for, are *not* declared: no page names either path, and a declared kind
  is a promise of a rollback. `projection_profile` is unmoved -- measured by
  building the previous release and this one and comparing the digest.
- **`status` says which backup slots retention cannot take, and whose they
  are.** Held slots shipped in `0.0.6` and there was no way to ask, so a
  consumer planning a long series could only learn its baseline was unprotected
  by watching it evicted -- the failure a hold exists to prevent, discovered the
  same way. Every entry of `backups[]` now carries `held` and `hold_reason`. A
  field rather than a wire command, because what a consumer needs is not to
  *hold* but to *know*, and retention is this pool's policy.
- **Three postures on every one of the seven, and `full-auto` is the new one.**
  `baseline` is a working floor, `minimal` is the product's own defaults, and
  `full-auto` asks nothing and sandboxes nothing -- in each product's own keys:
  `permissions.defaultMode` and `sandbox.enabled` here, `approval_policy` and
  `sandbox_mode` for codex, `[ui] permission_mode` and `[sandbox] profile` for
  grok, `approvalMode` and `sandbox.mode` for cursor, the documented `"*"`
  catch-all for opencode, `toolPermission` and `enableTerminalSandbox` for
  antigravity, and project trust for pi, which documents no sandbox to turn off.
  A caller who learns the three on one product knows them on all seven, and a
  test refuses a harness that offers fewer -- or two setups with the same bytes,
  which would be a posture in name only.
- **Two setups wrote configuration their product does not read.** opencode's
  `permission` took a bare string where the product documents an object, and
  antigravity's set `toolPermissions` where the product reads `toolPermission`
  with four values, none of them the one written. Both were valid JSON at the
  right path, both installed and restored cleanly, and neither changed anything
  about the product. Every key in every setup now cites the page it came from,
  and a test refuses a setup that writes configuration and names no source.
- **One condition, one sentence.** A target restored to a state that predates
  any setup reads the same as one nothing has touched.
- The provider kit moves to `0.2.3`, and nine vendor pins advance.

## [0.0.6] - 2026-08-27

Two defects a consumer found, one this build could not see, and the
last product that was an exception.

- **`provider_plan_digest` was null after every operation of every kind.** It
  was read out of the plan *object*, which never carries it: the digest is
  taken over the plan and travels beside it in the planner's envelope. Reported
  as an empty-setup defect and never about emptiness at all. It stayed
  invisible for four releases because `status` did not publish the field, and a
  consumer skips what is absent -- publishing what is persisted is what made it
  a value that could be compared and refused.
- **A backup refused a link while copying rather than before.** The slot was
  created, files were written into it, and the walk then stopped -- a partial
  operation and control artifacts for a shape that was knowable for free. Owned
  paths are read before planning and again before any capture now, every
  unsupported entry is named at once, and nothing is followed.
- **On Windows nothing was ever exposed, so no update was ever an update.**
  Reading which version a prefix exposes resolved the command's path, and
  Windows writes a hard link or a copy there rather than a link -- so the answer
  was always "nothing is exposed" and every `software_update` refused as an
  update of nothing. Shipped in 0.0.4 and 0.0.5. The version is recorded beside
  the command and read back now; a dangling link still exposes nothing.
- **Pi installs like the other six.** It declared no software lifecycle on the
  stated ground that npm resolves its closure at install time. The vendor ships
  `npm-shrinkwrap.json`, so the closure is fixed -- and it does not matter,
  because the published bundle imports only Node built-ins and runs with no
  `node_modules` at all. All seven now declare the same four optional
  operations. Its entry point is JavaScript, so Windows exposes `pi.cmd`
  rather than a copy no platform would run.
- **A backup can be held.** The pool rolls at ten slots, so a long series of
  captures evicted the baseline it meant to return to. A held slot is not
  reclaimed and is not counted against the bound, the reason is recorded beside
  it so a full pool says who would lose what, and the last reclaimable slot
  cannot be held -- a target that can never be backed up again is worse than
  the eviction.

Vendor versions advanced where they moved: claude 2.1.246, cursor
2026.08.25-3e8eec8, antigravity 1.1.21.

## [0.0.5] - 2026-08-26

The catalog now travels with the program, and a target's identity is
what this provider owns.

- **The published first command did not work.** The release ships binaries and
  a `SHA256SUMS`, `install.sh` places one file, and `setups/` existed only in
  the git tree -- so `list` and `install <setup> --target` refused for everyone
  who installed the documented way, on all three operating systems, for four
  releases. The catalog is compiled into the binary and materialized on use.
  `<PROVIDER>_SETUP_CATALOG` and the on-disk search still win wherever they find
  something: a caller's own setups are as legitimate a source as these.
- **A target's identity was a denylist over its whole directory**, while backup,
  restore, remove and materialization were scoped to the declared namespaces. A
  neighbour writing to its own files moved the identity a plan was made against,
  and on a shared configuration home it made `status` unanswerable -- measured
  at 1.5 s against 12,602 unrelated files, and reported from a real Windows
  target of 124,065 where it exceeded two minutes. Identity is now the owned
  projection and costs nothing per unrelated file.

  **This changes every existing target's identity.** A target recorded by an
  earlier release now reports drift; `reinstall` or `restore` settles it. That
  is the honest answer to a provider that has started measuring a different
  thing, and a compatibility mode would have carried the defect forever to
  avoid saying so.
- **`status` returned six of the twenty-five provenance fields it persists.**
  All of them are published now, so a consumer can bind the target in front of
  it to the installation it approved. The nested `provider_state` is unchanged;
  a drifted or unmanaged target publishes nothing flat, because its record
  describes bytes that are no longer there.

## [0.0.4] - 2026-08-25

Two operations that were one, and three provenance fields that were null.

- `software_install` and `software_update` produced byte-identical plans. The
  plan phase may read the local disk, so what is already under `--prefix` is in
  it now: installing over the same version says so, updating from an older one
  says which, and updating a prefix that holds nothing is refused rather than
  quietly installing. `software_remove` names the versions it leaves.
- `component_refs`, `setup_stable_id` and `setup_version` were written empty for
  every bundle install, and the bundle carried all three. `setup-passport.json`
  is a required member of the format that this reader required and then
  discarded, so a target configured over the wire reported no applied setup
  while the same target from the local catalogue reported one.
  `setup_version_passport_digest` stays null on purpose: the passport does not
  carry its own digest and the contract does not define how one is taken.
- Antigravity ships a first-party `nddev-builder` setup: one native plugin
  carrying one skill about where this product's configuration lives inside a
  home it shares with Gemini CLI, and which neighbouring files are not its own.

Linux, macOS and Windows; x86_64 and arm64.

## [0.0.3] - 2026-08-25

Starts Pi Coding Agent, and takes over a target the estate before this one
still claims.

- `launch`, where this build installs the product and the product documents an
  environment variable for its configuration home. It starts the exact
  executable a software install placed under `--prefix`, never a name found on
  `PATH`, points the product at `--target` through that variable, passes
  anything after a bare `--` through verbatim, and replaces this process so the
  caller's stdio and exit status become the product's.
- `adopt`, a command someone types, for a target still carrying the stamp file
  the frozen Python estate wrote. Nothing is deleted: the stamp moves into this
  provider's control directory, and a backup is captured first. Every file it
  claims is accounted for as intact, changed or missing before anything is
  taken over.
- A populated configuration home now runs through install, backup, restore and
  remove in the test suite on all three systems, compared against a fingerprint
  computed with `std::fs` and `sha2` rather than against the digest this
  provider uses to decide a target is unchanged.
- `--help` describes what each build actually does rather than what they once
  all did.

Linux, macOS and Windows; x86_64 and arm64.

## [0.0.2] - 2026-08-25

Installs Pi Coding Agent itself, not only its configuration.

- `software_install`, `software_update` and `software_remove`, in the shape
  agreed with the consumer on `ai-engineers-guild/ai_stp#414`: `--target` is
  the configuration directory and `--prefix` is the program directory, the plan
  carries an array `software_artifacts`, and `apply` receives one repeated
  `--software-artifact` per element in the plan's order.
- The provider never opens a socket. The contract gives software a download
  phase and gives a provider no command to run it in, so `plan` names one url,
  one length and one digest while offline, whoever holds the network fetches
  exactly that, and `apply` re-checks it offline and installs.
- Software lands under `--prefix`, never the configuration target, and spends
  no backup slot: there are ten, and they hold configuration.
- Reads the one archive shape every vendor ships -- a gzip-compressed tar, or
  plain bytes. POSIX `ustar` and GNU tar with long-name headers; regular files
  and directories only. Every other entry type is refused by name.
- The vendored provider kit moves to 0.2.1. A permission profile this build
  never advertised now answers `unsupported_permission_profile` instead of the
  nearest thing the previous closed set had.

Linux, macOS and Windows; x86_64 and arm64.

## [0.0.1] - 2026-08-24

First release. Installs, reselects, restores and removes a complete
Pi Coding Agent harness configuration in a caller-named target directory.

- All five core operations of the ai-stp provider protocol v3 -- `backup`,
  `restore`, `remove`, `install` and `replace` -- from the local setup catalog
  and from an `ai-stp-bundle/1` arriving over the wire.
- Every mutation captures a backup first, so `restore` always has somewhere to
  return to, and an interrupted mutation is recovered from its own durable
  journal rather than left half-applied. A backup captures only what a restore
  can put back.
- Commands for a person: `list`, `status`, `install`, `select`, `reinstall`,
  `diff`, `backups`, `restore`, `remove`. Every one takes an explicit
  `--target`; nothing is inferred from a home directory.
- Reads do not write. `status` and `backups` report without creating anything
  in the directory they are reporting on.
- The software lifecycle and `launch` are optional in the contract and are not
  declared, because this build does not perform them.

Passes `ai-stp provider conformance --protocol-version 3` at 20/20.

Linux, macOS and Windows; x86_64 and arm64.
