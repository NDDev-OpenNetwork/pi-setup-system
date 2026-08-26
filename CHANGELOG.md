# Changelog

This file is a release ledger: every heading below is a real release.

The project follows Semantic Versioning. `0.0.x` says plainly that the wire
surface is proven against the consumer's conformance but has not yet been run
against a real installation on every platform it claims.

An entry is never edited after its release. It says what that release was,
including claims a later release made false.

## [Unreleased]

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
