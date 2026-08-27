# Changelog

This file is a release ledger: every heading below is a real release.

The project follows Semantic Versioning. `0.0.x` says plainly that the wire
surface is proven against the consumer's conformance but has not yet been run
against a real installation on every platform it claims.

An entry is never edited after its release. It says what that release was,
including claims a later release made false.

## [Unreleased]

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
