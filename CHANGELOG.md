# Changelog

This file is a release ledger: every heading below is a real release.

The project follows Semantic Versioning. `0.0.x` says plainly that the wire
surface is proven against the consumer's conformance but has not yet been run
against a real installation on every platform it claims.

An entry is never edited after its release. It says what that release was,
including claims a later release made false.

## [Unreleased]

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
