//! What one harness is, stated once as data.
//!
//! Every setup system runs the same commands over the same kernel; what differs
//! is which directory it owns, which files inside it are its own, and which
//! files belong to the product and must be left alone. Those differences are
//! *facts about a product*, verified against its official documentation and
//! recorded in a baseline — not behaviour, and not code.
//!
//! Holding them as data rather than as five copies of a dispatcher means a
//! change to the shared logic lands in one place, and a change to a product's
//! surface lands in exactly one struct with a test binding it to that product's
//! baseline.

use provider_v3::{
    Command, ComponentKind, Declaration, Operation, ProjectionKind, ProjectionProfile,
    ProviderInfo, TargetScope,
};
use setup_core::digest;
use setup_core::software::{Delivery, Software};

/// One harness, as the runtime needs to know it.
#[derive(Debug, Clone, Copy)]
pub struct Harness {
    /// The harness identity on the wire.
    pub harness_id: &'static str,
    /// The provider identity on the wire, matching the crate name.
    pub provider_id: &'static str,
    /// The build version.
    pub version: &'static str,
    /// The product being configured.
    pub product: &'static str,
    /// Who publishes the product.
    pub vendor: &'static str,
    /// The documented configuration home. Documentation, never a fallback.
    pub documented_config_home: &'static str,
    /// The environment variable a product documents for its configuration home.
    ///
    /// Empty when the product documents none. That is a real state -- not every
    /// product offers an override -- and it is worth saying rather than
    /// inventing a plausible variable name that nothing reads.
    ///
    /// Documentation either way: nothing here resolves a path from it, because
    /// every command takes an explicit target.
    pub config_home_env: &'static str,
    /// Whether pointing the product at a target actually moves what it reads.
    ///
    /// This used to be inferred: `can_launch` asked whether the build installs
    /// software and whether `config_home_env` is non-empty, and concluded that
    /// the product could be started against any target. Neither question is the
    /// one that decides, and for one harness the answer was already written
    /// down and disagreed. Cursor's own baseline note, from 2026-08-28: *"one of
    /// the eight this build owns"* follows the variable -- `cli-config.json` --
    /// while `rules`, `commands`, `hooks.json`, `mcp.json` and the plugin pair
    /// are built from a literal join to the process home and reach no resolver.
    ///
    /// So a launch there assembled a session from the caller's rules, hooks and
    /// MCP servers and the target's settings file: a harness nobody selected,
    /// and one that can execute code the chosen setup never carried.
    ///
    /// A fact rather than a conclusion, carrying how it was established, because
    /// the five that are complete are not equally well established: three were
    /// measured by asking the product what it resolved, one by making it write,
    /// and one rests on a vendor page because no credential-free command of that
    /// product writes its home.
    pub launch_binding: LaunchBinding,
    /// The variable that stops this product replacing the bytes we installed.
    ///
    /// Empty where the product has none, which is the ordinary case: of the two
    /// artifacts this estate has read for it, claude carries `DISABLE_UPDATES`
    /// nine times and codex carries no such literal at all. Measured rather than
    /// assumed, with an invented variable searched in the same bytes and absent
    /// from both, so the search discriminates.
    ///
    /// It is set at launch, and it is the one exception to the rule stated at
    /// that call site -- *nothing else in the environment is touched, because
    /// only the vendor knows what its program needs*. The exception is narrow
    /// and is not about what the product needs: this provider pins a version,
    /// records its digest, and offers a rollback to the version beside it. A
    /// product that replaces those bytes while running makes all three false,
    /// and the vendor documents this variable for exactly the case of a
    /// distribution channel somebody else controls.
    ///
    /// Empty means the launch environment is untouched, not that the product
    /// updates itself.
    pub updates_off_env: &'static str,
    /// The condition under which [`Self::documented_config_home`] is not where
    /// the product looks, printed beside it in `--help`.
    ///
    /// Empty for six of the seven, because six resolve one home from one
    /// variable. Cursor does not: its resolver falls through
    /// `CURSOR_CONFIG_DIR` to `XDG_CONFIG_HOME`, and the second **renames the
    /// leaf** -- `$XDG_CONFIG_HOME/cursor`, not `.cursor`. So on a Linux
    /// machine with XDG set, the single string above is wrong, and a person
    /// reading it would point `--target` at a directory the product does not
    /// read.
    ///
    /// A sentence rather than a second path, because this build does not
    /// resolve either one: every command takes an explicit `--target`, and the
    /// job here is to tell a person the truth about their own machine.
    pub config_home_note: &'static str,
    /// The provider-owned control directory inside a target.
    pub control_directory: &'static str,
    /// The provider-owned state file inside a target.
    pub state_file: &'static str,
    /// The state file the frozen estate's program wrote in this same target.
    ///
    /// Empty when that estate had no module for this product. It is a fact per
    /// harness and not derivable: this build's `NDDEV-GROK-PROVIDER.json` had a
    /// predecessor called `NDDEV-GROK-BUILD-SETUP.json`, and cursor's and
    /// antigravity's differ from their stems too. Deriving the name would have
    /// been wrong for three of the seven.
    ///
    /// Read only by `adopt`, which is a command someone types. Nothing else
    /// looks at it, and no automatic path acts on its presence.
    pub predecessor_state_file: &'static str,
    /// The projection profile identity a compiler builds against.
    pub profile_id: &'static str,
    /// The top-level entries this provider owns inside a target.
    ///
    /// Everything else is a sibling overlay preserved verbatim.
    pub native_namespaces: &'static [&'static str],

    /// Names the product reads that this provider does not own.
    ///
    /// Ownership decides what `remove` takes and what the target digest
    /// covers. It does not decide what the product obeys, and for one harness
    /// here those are different sets: measured against the pinned 1.18.25
    /// binary, `opencode` reads `opencode.jsonc` after `opencode.json` and
    /// keeps the later one, and globs `{skill,skills}` where only the plural
    /// is owned. So a target whose owned bytes are clean can be running a file
    /// this provider never wrote, and `status` answered `managed` with nothing
    /// beside it to say so.
    ///
    /// **This does not make the name owned, and must not.** Owning both
    /// spellings would let one setup install two files that disagree, and
    /// deleting a file somebody else put there is not this provider's call. It
    /// makes the name *visible*, which is the part that was missing: the
    /// answer was true about what it examined and silent about what decides.
    ///
    /// Empty for six of the seven, and empty because they were asked -- a
    /// product whose alternate spellings nobody has measured belongs here as
    /// nothing rather than as a guess.
    pub shadowing_names: &'static [Shadow],
    /// Product-owned paths this provider never reads and never writes.
    ///
    /// Excluded from backups so a slot never holds credentials, and excluded
    /// from target identity so the product's own traffic cannot strand a plan.
    pub never_touch: &'static [&'static str],
    /// What a *neighbour's* configuration home looks like from inside a target.
    ///
    /// Every command here takes an explicit `--target` because a change aimed at
    /// a guessed path is a change aimed at someone else's state. That rule stops
    /// this program guessing; it does nothing about a caller who names the wrong
    /// place confidently.
    ///
    /// Pi is where that bites. Oh My Pi is a separate product descended from the
    /// same code, and the two are near-identical in shape: `~/.pi/agent` against
    /// `~/.omp/agent`, one directory name apart. But Pi reads `settings.json`
    /// and Oh My Pi reads `config.yml`, so a setup written into the wrong one is
    /// not a broken installation — it is an **ignored** one, and the target
    /// looks configured.
    ///
    /// Empty for the six that have no near neighbour. A marker is only listed
    /// where it was measured; guessing one would refuse a legitimate target.
    pub foreign_homes: &'static [Foreign],
    /// The permission profiles this provider can apply.
    pub permission_profiles: &'static [&'static str],
    /// The component kinds this provider projects.
    pub component_kinds: &'static [ComponentKind],
    /// The projection kinds this provider performs.
    pub projection_kinds: &'static [ProjectionKind],
    /// Second targets this provider owns, if any.
    ///
    /// Empty for six of the seven. Antigravity is the exception because the
    /// product genuinely keeps a workspace copy of five of its surfaces, and
    /// `ai_stp#424`/`#425` are the consumer asking for exactly that route.
    pub scoped_projections: &'static [Scoped],
    /// The largest file count a bundle may carry.
    pub max_files: u64,
    /// The largest byte count a bundle may carry.
    pub max_bytes: u64,
    /// The exact provider-kit revision this build was compiled against.
    pub kit_identity: &'static str,
    /// This harness's setup catalog, compiled in, as relative path and bytes.
    ///
    /// The release ships binaries and nothing else, so a catalog that only ever
    /// existed on disk made `list` and `install` refuse for everyone who
    /// installed the documented way. Carrying it here means the program is the
    /// whole thing rather than a pointer at a checkout.
    ///
    /// It is the *floor*. `<PROVIDER>_SETUP_CATALOG` and the on-disk search
    /// still win wherever they find something, because a caller's own setups
    /// are as legitimate a source as these.
    ///
    /// Paths are relative and slash-separated, and the build script refuses to
    /// put a symbolic link or an executable file in here — a setup's digest
    /// records both, and bytes alone cannot carry either.
    pub embedded_setups: &'static [(&'static str, &'static [u8])],
    /// How the product's own software is installed, when this build can do it.
    ///
    /// `None` means the software lifecycle is not offered at all. So does a
    /// [`Delivery::Manager`], which is a different statement -- the product is
    /// installable, but not by fetching bytes whose digest was fixed in advance
    /// -- and the refusal says which.
    pub software: Option<Software>,
}

/// A second set of ownings, for a target that is not the product's own home.
///
/// A provider invoked against a workspace owns different paths than one invoked
/// against the configuration home: `config/skills` means nothing in a project
/// and `.agents/skills` means nothing in the global home. One declaration
/// cannot honestly describe both, and *the declaration is the authority* is the
/// property backup, restore, remove and target identity all read.
///
/// So a harness that owns a second target says so here, and `provider-info`
/// publishes a profile per scope with its own digest. `projection_profile` is
/// untouched by this — byte for byte what it was — so every bundle compiled
/// against a declaration published before this field stays valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scoped {
    /// Which target this profile owns.
    pub target_scope: TargetScope,
    /// The profile identity a compiler builds against for that target.
    pub profile_id: &'static str,
    /// The component kinds this provider projects there.
    pub component_kinds: &'static [ComponentKind],
    /// The projection kinds this provider performs there.
    pub projection_kinds: &'static [ProjectionKind],
    /// The top-level entries this provider owns inside such a target.
    pub native_namespaces: &'static [&'static str],
}

/// How completely a product follows the target it is pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchBinding {
    /// Every owned surface at the target is what the product reads there.
    ///
    /// `how` records what established it, so a reader can tell a product that
    /// was asked what it resolved from one that was only read about.
    Complete {
        /// What established it, so a reader can tell a product that was asked
        /// what it resolved from one that was only read about.
        how: &'static str,
    },
    /// Some owned surfaces follow the target and the rest resolve elsewhere.
    ///
    /// `unbound` names the ones that do not, because that list is the whole
    /// reason a launch here would be a different harness from the one selected.
    Partial {
        /// The owned surfaces that do *not* follow the target, because that list
        /// is the whole reason a launch here would be a different harness.
        unbound: &'static str,
    },
    /// The product documents no way to be pointed at a target at all.
    Undocumented,
}

/// A name the product reads that sits beside one this provider owns.
///
/// See [`Harness::shadowing_names`]. Each of these is measured by running the
/// pinned product, not read off a page: the question is what the product does
/// when both names are present, and only the product answers that.
#[derive(Debug)]
pub struct Shadow {
    /// The name as it appears in the target, relative to its root.
    pub name: &'static str,
    /// The owned namespace it can take precedence over.
    pub over: &'static str,
    /// What the product does when both are there, in the words of the run that
    /// established it.
    pub effect: &'static str,
}

/// One sign that a target is a different product's configuration home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Foreign {
    /// A path, relative to the target, whose presence indicates the other one.
    pub marker: &'static str,
    /// The product it indicates, named as its own documentation names it.
    pub product: &'static str,
    /// Where that product's configuration actually lives, so the refusal is
    /// something a caller can act on rather than only a stop.
    pub home: &'static str,
}

/// How many backup slots a target keeps.
pub const BACKUP_SLOTS: usize = 10;

/// The bundle format every setup system reads.
pub const BUNDLE_FORMAT: &str = "ai-stp-bundle/1";

impl Harness {
    /// Whether one relative path falls inside a namespace this harness claims.
    ///
    /// A namespace is not always a single path component. Codex routes skills
    /// to `.agents/skills` while owning nothing else under `.agents`, and
    /// Antigravity is a guest inside `~/.gemini` where every namespace is
    /// nested. Comparing only the first component reads those as `.agents` and
    /// `config` -- directories holding another product's files -- and refuses
    /// every write to the deeper path this harness genuinely owns.
    ///
    /// A path is owned when it *is* a namespace or lies beneath one. The
    /// trailing separator matters: without it `skills-experimental` would match
    /// the namespace `skills`, and a neighbour would be swallowed by a prefix.
    ///
    /// This lives here, not beside either caller, because the wire surface and
    /// the local catalog must answer this question identically. They did not
    /// once, and the catalog refused every setup the wire would have accepted.
    #[must_use]
    pub fn owns(&self, path: &str) -> bool {
        Self::within(self.native_namespaces, path)
    }

    /// Whether a path falls inside the namespaces the named scope owns.
    ///
    /// The check a bundle install has to make once a scope is known. `owns`
    /// answers for the global target and answered for every target: a bundle
    /// routing a skill to codex under `user_root` writes `skills/<name>`, codex
    /// **declines** `skills` under its own home, and so the install was refused
    /// as writing outside the surface — a scope the provider declares and
    /// cannot be installed into.
    #[must_use]
    pub fn owns_at(&self, path: &str, scope: Option<TargetScope>) -> bool {
        Self::within(self.owned_projection(scope), path)
    }

    /// Whether *any* target this provider declares owns a path.
    ///
    /// `validate-bundle` is handed a bundle and a target and no scope — that is
    /// the argv contract — so the question it can answer is "could this
    /// provider install this bundle at all", not "at this scope". Plan and
    /// apply know the scope and ask the exact question with [`Self::owns_at`].
    #[must_use]
    pub fn owns_anywhere(&self, path: &str) -> bool {
        self.owns(path)
            || self
                .scoped_projections
                .iter()
                .any(|scoped| Self::within(scoped.native_namespaces, path))
    }

    fn within(namespaces: &[&str], path: &str) -> bool {
        namespaces.iter().any(|namespace| {
            path == *namespace
                || path
                    .strip_prefix(namespace)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// Whether this build can put a program on disk from bytes it can verify.
    ///
    /// Not the same question as "does this harness have a `software` field". A
    /// `Delivery::Manager` product is installable, just not by fetching an
    /// artifact whose digest was fixed in advance, and offering it `software`
    /// and `rollback` would be offering commands that can only refuse.
    ///
    /// **No harness in this estate is manager-delivered today.** Pi was the
    /// example this comment named until `7180648` measured its tarball; the
    /// distinction is kept because the type admits it and `human.rs` exercises
    /// it with a synthetic fixture, not because one of the seven is waiting to
    /// be found by it.
    #[must_use]
    pub const fn installs_a_program(&self) -> bool {
        matches!(
            self.software,
            Some(Software {
                delivery: Delivery::Artifacts(_),
                ..
            })
        )
    }

    /// The scoped profile a named scope selects, when this harness declares one.
    ///
    /// `None` for the global target — including for an explicit
    /// `TargetScope::Global`, which *is* the global block rather than a fourth
    /// declaration of it.
    #[must_use]
    pub fn scoped_for(&self, scope: Option<TargetScope>) -> Option<Scoped> {
        let named = scope?;
        self.scoped_projections
            .iter()
            .find(|scoped| scoped.target_scope == named)
            .copied()
    }

    /// The projection this provider owns **at the target a scope names**.
    ///
    /// It is a method rather than a field access so that every caller asking
    /// "what is this target's identity" goes through one name — the defect
    /// behind `ai_stp#417` was that the one caller answering that question
    /// answered it differently from the four that mutate.
    ///
    /// It took a scope for the same reason, one release later and for a worse
    /// version of the same defect. `scoped_projections` gave two harnesses a
    /// second target — codex's `~/.agents` under `user_root`, antigravity's
    /// workspace under `project` — and this method kept answering with the
    /// *global* namespaces. Measured on the shipped `0.0.27`: a `~/.agents`
    /// holding two skills planned a backup whose `expected_target_digest` was
    /// the digest of the empty string, and applying it produced a slot with a
    /// `slot.json` and no payload. A backup that reports success and captures
    /// nothing.
    ///
    /// The declaration was scope-aware and so was `remove`. The identity, the
    /// capture, the replace and the recorded ownership were not: five facts,
    /// one compared.
    #[must_use]
    pub fn owned_projection(&self, scope: Option<TargetScope>) -> &'static [&'static str] {
        match self.scoped_for(scope) {
            Some(scoped) => scoped.native_namespaces,
            None => self.native_namespaces,
        }
    }

    /// Paths that are not part of this target's identity even so.
    ///
    /// The control directory and the state file are this provider's own
    /// bookkeeping; counting them would make an applied operation leave the
    /// target different from the identity it just recorded. The never-touch
    /// paths are the product's own — it rewrites credentials and session
    /// history constantly, and letting that traffic move the identity would
    /// strand a plan for a change no effect of ours would have overwritten.
    ///
    /// Since identity became the owned projection this is mostly belt and
    /// braces: a never-touch path outside every namespace is already excluded by
    /// not being owned. It still matters for one that is declared *and*
    /// disclaimed, which a harness test forbids, and it costs nothing to keep
    /// the two statements agreeing.
    #[must_use]
    pub fn not_our_identity(&self) -> Vec<&'static str> {
        let mut names = vec![self.state_file];
        names.extend_from_slice(self.never_touch);
        names
    }

    /// Top-level entries a backup never captures.
    ///
    /// Credentials belong to the product. A slot that held them would put them
    /// on disk in a second place, which is a worse outcome than an incomplete
    /// restore of files this provider never wrote anyway.
    #[must_use]
    pub fn never_captured(&self) -> Vec<&'static str> {
        let mut names = vec![self.control_directory];
        names.extend_from_slice(self.never_touch);
        names
    }

    /// A digest of this build's own manifest.
    ///
    /// The contract is explicit that the release digest must not come from
    /// `provider-info` — an artifact hashing itself proves nothing. This is a
    /// different value: an independent statement of what this build *is*, which
    /// the consumer records beside the release digest it verified separately.
    ///
    /// # Errors
    ///
    /// Returns a declaration error if the vendored kit identity is unreadable.
    pub fn build_digest(&self) -> provider_v3::Result<String> {
        let kit: serde_json::Value = serde_json::from_str(self.kit_identity).map_err(|source| {
            provider_v3::Error::declaration(format!(
                "the vendored kit identity is unreadable: {source}"
            ))
        })?;
        let manifest = serde_json::json!({
            "provider_id": self.provider_id,
            "provider_version": self.version,
            "protocol_version": provider_v3::PROTOCOL_VERSION,
            "harness_id": self.harness_id,
            "kit_aggregate_digest": kit["aggregate_digest"],
        });
        digest::of_canonical_json(&manifest)
            .map_err(|source| provider_v3::Error::declaration(source.detail()))
    }

    /// The projection profile this build declares.
    ///
    /// # Errors
    ///
    /// Propagates a declaration refusal.
    pub fn projection_profile(&self) -> provider_v3::Result<ProjectionProfile> {
        ProjectionProfile::new(
            self.profile_id,
            self.component_kinds,
            self.projection_kinds,
            self.native_namespaces,
            &[BUNDLE_FORMAT],
            self.max_files,
            self.max_bytes,
        )
    }

    /// The projection profile **at the target a scope names**.
    ///
    /// A method rather than a second call site, for the reason
    /// [`Self::projection_profile`] gives one level up and this is the next face
    /// of: `projection_profile` answers with the global block whatever it is
    /// asked, and its digest was written into a scoped plan and into the state a
    /// scoped operation persists. So a consumer that compiled a bundle against
    /// the scoped profile this build publishes in `provider-info` was handed a
    /// plan naming a different profile, and the state afterwards recorded that
    /// one too.
    ///
    /// Built the way `provider-info` builds it -- `ProjectionProfile::scoped`
    /// with the same seven inputs -- rather than assembled again here, because
    /// two constructions of one identity is how the digests came apart in the
    /// first place.
    ///
    /// `None` is the global target, which is what the human surface passes: a
    /// person at a terminal chose no scope and the global block is the answer.
    ///
    /// # Errors
    ///
    /// Propagates a declaration refusal.
    pub fn projection_profile_for(
        &self,
        scope: Option<TargetScope>,
    ) -> provider_v3::Result<ProjectionProfile> {
        match self.scoped_for(scope) {
            Some(scoped) => ProjectionProfile::scoped(
                scoped.profile_id,
                scoped.component_kinds,
                scoped.projection_kinds,
                scoped.native_namespaces,
                &[BUNDLE_FORMAT],
                self.max_files,
                self.max_bytes,
                scoped.target_scope,
            ),
            None => self.projection_profile(),
        }
    }

    /// The complete `provider-info` answer for this build.
    ///
    /// Only the five core operations are declared. The software lifecycle and
    /// `launch` are optional in the contract, and this runtime does not
    /// implement them — declaring one would let a consumer call an operation
    /// that cannot be honoured, which is worse than not offering it.
    ///
    /// Whether this build can start the product it installed.
    ///
    /// Two things have to hold. It must have installed one -- launching a name
    /// found on `PATH` starts whatever else shares that spelling, which is not
    /// this product and not this build's business. And the product must
    /// document an environment variable for its configuration home, because
    /// every command in this contract takes a `--target` and a launch that
    /// could not point the product at it would be answering a different
    /// question than the one asked.
    ///
    /// Antigravity documents no such variable. It installs and does not launch,
    /// and that is the honest pair rather than a launch that ignores its target.
    #[must_use]
    pub fn can_launch(&self) -> bool {
        // Three conditions, and the history is in the order. This used to be the
        // last two: a variable exists and software is installed. Both are
        // necessary and neither is sufficient -- what decides is whether the
        // variable moves what this provider owns, which is the first.
        //
        // The variable stays in the conjunction rather than being folded into
        // the binding, because a product that documents none cannot be pointed
        // at a target at all, and that is a different sentence from a product
        // that can be pointed at one and only half follows.
        matches!(self.launch_binding, LaunchBinding::Complete { .. })
            && !self.config_home_env.is_empty()
            && matches!(
                self.software,
                Some(Software {
                    delivery: Delivery::Artifacts(_),
                    ..
                })
            )
    }

    /// Why this build does not start its product, for a caller that asked.
    ///
    /// Named rather than left to a generic refusal: *"cursor-setup-system does
    /// not declare launch"* tells a person nothing they can act on, and the
    /// thing they can act on is which surfaces would have come from somewhere
    /// else.
    #[must_use]
    pub fn why_no_launch(&self) -> String {
        if self.config_home_env.is_empty() {
            return format!(
                "{} documents no environment variable for its configuration home, so a \
                 launch could not point it at the target this command was given",
                self.product
            );
        }
        match self.launch_binding {
            LaunchBinding::Undocumented => format!(
                "{} documents no way to be pointed at a target",
                self.product
            ),
            LaunchBinding::Partial { unbound } => format!(
                "{} follows {} for some of what this provider owns and not for the rest: \
                 {}. A launch would assemble a session from this target and the caller's \
                 own home, which is a different harness from the one selected",
                self.product, self.config_home_env, unbound
            ),
            LaunchBinding::Complete { .. } => {
                "this build installs no software, and launching a name found on PATH would \
                 start whatever else shares it"
                    .to_owned()
            }
        }
    }

    /// The commands this build answers.
    #[must_use]
    pub fn commands(&self) -> &'static [Command] {
        if self.can_launch() {
            Command::ALL
        } else {
            Command::CORE
        }
    }

    /// The operations this build actually performs.
    ///
    /// The software lifecycle is optional in the contract, and declaring an
    /// operation a build cannot perform lets a consumer ask for something that
    /// cannot be honoured. So it appears here only when this harness carries an
    /// artifact table -- never when the product is delivered by a package
    /// manager this provider does not run.
    #[must_use]
    pub fn operations(&self) -> &'static [Operation] {
        match (self.can_launch(), self.software) {
            (
                true,
                Some(Software {
                    delivery: Delivery::Artifacts(_),
                    ..
                }),
            ) => Operation::ALL,
            (
                false,
                Some(Software {
                    delivery: Delivery::Artifacts(_),
                    ..
                }),
            ) => Operation::CORE_AND_SOFTWARE,
            _ => Operation::CORE,
        }
    }

    /// # Errors
    ///
    /// Propagates a declaration refusal.
    pub fn provider_info(&self) -> provider_v3::Result<ProviderInfo> {
        let build_digest = self.build_digest()?;
        ProviderInfo::declare(Declaration {
            provider_id: self.provider_id,
            harness_id: self.harness_id,
            provider_version: self.version,
            provider_build_digest: &build_digest,
            commands: self.commands(),
            operations: self.operations(),
            supported_os: &["linux", "macos", "windows"],
            supported_arch: &["x86_64", "arm64"],
            permission_profiles: self.permission_profiles,
            projection_profile: self.projection_profile()?,
            scoped_projection_profiles: self
                .scoped_projections
                .iter()
                .map(|scoped| {
                    ProjectionProfile::scoped(
                        scoped.profile_id,
                        scoped.component_kinds,
                        scoped.projection_kinds,
                        scoped.native_namespaces,
                        &[BUNDLE_FORMAT],
                        self.max_files,
                        self.max_bytes,
                        scoped.target_scope,
                    )
                })
                .collect::<provider_v3::Result<Vec<_>>>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    /// A variable existing is not the same as a variable moving what matters.
    ///
    /// `can_launch` asked two questions -- does this build install software, and
    /// is `config_home_env` non-empty -- and concluded that the product could be
    /// started against any target. Neither question is the one that decides, and
    /// for cursor the answer was already written down and disagreed: its own
    /// baseline note has said since 2026-08-28 that one of the eight surfaces it
    /// owns follows the variable.
    ///
    /// Both directions, because only the pair says anything: a build that
    /// declared launch for every binding would pass the first assertion, and one
    /// that declared it for none would pass the second.
    #[test]
    fn a_partial_binding_never_becomes_launch_capable() {
        // `SAMPLE` declares no software, so the fixture supplies some: the rule
        // is a conjunction and a test that forgot one half would pass for the
        // wrong reason.
        const ARTIFACT: setup_core::software::Artifact = setup_core::software::Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/x.tgz",
            bytes: 1,
            sha256: "sha256:0",
            shape: setup_core::software::Shape::GzipTar,
            member: "package/x",
        };
        const DELIVERED: Software = Software {
            command: "x",
            version: "1.0.0",
            delivery: Delivery::Artifacts(&[ARTIFACT]),
            unsupported: &[],
            previous: None,
        };
        let with_software = Harness {
            software: Some(DELIVERED),
            ..SAMPLE
        };

        let complete = Harness {
            launch_binding: LaunchBinding::Complete { how: "measured" },
            ..with_software
        };
        assert!(
            complete.can_launch(),
            "a complete binding with artifacts cannot launch"
        );

        // The other half of the conjunction, so neither can carry the rule alone.
        let complete_without_software = Harness {
            launch_binding: LaunchBinding::Complete { how: "measured" },
            software: None,
            ..SAMPLE
        };
        assert!(
            !complete_without_software.can_launch(),
            "a build with nothing installed declared launch"
        );

        let partial = Harness {
            launch_binding: LaunchBinding::Partial {
                unbound: "rules, hooks.json",
            },
            ..with_software
        };
        assert!(
            !partial.can_launch(),
            "a partial binding declared launch because a variable exists"
        );
        assert!(
            partial.why_no_launch().contains("rules, hooks.json"),
            "the refusal does not name the surfaces that would come from elsewhere: {}",
            partial.why_no_launch()
        );

        let undocumented = Harness {
            launch_binding: LaunchBinding::Undocumented,
            config_home_env: "",
            ..with_software
        };
        assert!(!undocumented.can_launch());
    }

    use super::*;

    /// A harness that offers no software lifecycle, which is most of what the
    /// declaration tests are about.
    pub(crate) const SAMPLE: Harness = Harness {
        launch_binding: LaunchBinding::Undocumented,
        software: None,
        predecessor_state_file: "",
        embedded_setups: &[],
        harness_id: "sample",
        provider_id: "sample-setup-system",
        version: "0.1.0",
        product: "Sample",
        vendor: "NDDev",
        documented_config_home: "~/.sample",
        config_home_env: "SAMPLE_CONFIG_DIR",
        updates_off_env: "",
        config_home_note: "",
        control_directory: ".sample-setup-system",
        state_file: "NDDEV-SAMPLE-PROVIDER.json",
        profile_id: "sample/native-files/1",
        native_namespaces: &["AGENTS.md", "settings.json", "skills"],
        shadowing_names: &[],
        never_touch: &[".credentials.json", "sessions"],
        foreign_homes: &[],
        permission_profiles: &["default"],
        component_kinds: &[ComponentKind::Instruction, ComponentKind::Skill],
        projection_kinds: &[ProjectionKind::NativeFiles],
        scoped_projections: &[],
        max_files: 4096,
        max_bytes: 1024,
        kit_identity: r#"{"aggregate_digest":"sha256:aa","protocol_version":3}"#,
    };

    #[test]
    fn identity_excludes_provider_bookkeeping_and_product_traffic() {
        let excluded = SAMPLE.not_our_identity();
        assert!(excluded.contains(&"NDDEV-SAMPLE-PROVIDER.json"));
        assert!(excluded.contains(&".credentials.json"));
        assert!(excluded.contains(&"sessions"));
    }

    #[test]
    fn a_backup_never_captures_credentials_or_the_control_directory() {
        let excluded = SAMPLE.never_captured();
        assert!(excluded.contains(&".sample-setup-system"));
        assert!(excluded.contains(&".credentials.json"));
    }

    #[test]
    fn nothing_is_both_owned_and_never_touched() {
        // Claiming a path the product owns would make an effect of ours
        // overwrite state we promised not to read.
        for name in SAMPLE.never_touch {
            assert!(
                !SAMPLE.native_namespaces.contains(name),
                "{name} is claimed and disclaimed"
            );
        }
    }

    #[test]
    fn the_declaration_offers_only_operations_this_runtime_performs() {
        let info = SAMPLE.provider_info().unwrap();
        for operation in Operation::CORE {
            assert!(info.declares(*operation));
        }
        for optional in [
            Operation::Launch,
            Operation::SoftwareInstall,
            Operation::SoftwareUpdate,
            Operation::SoftwareRemove,
        ] {
            assert!(
                !info.declares(optional),
                "{optional} is declared but not performed"
            );
        }
    }

    #[test]
    fn the_build_digest_is_reproducible_and_binds_the_kit() {
        let once = SAMPLE.build_digest().unwrap();
        assert_eq!(once, SAMPLE.build_digest().unwrap());
        assert!(once.starts_with("sha256:"));

        let other = Harness {
            version: "0.2.0",
            ..SAMPLE
        };
        assert_ne!(other.build_digest().unwrap(), once);
    }

    #[test]
    fn two_harnesses_differing_only_in_surface_get_different_profile_digests() {
        let narrower = Harness {
            native_namespaces: &["AGENTS.md"],
            ..SAMPLE
        };
        assert_ne!(
            narrower.projection_profile().unwrap().digest,
            SAMPLE.projection_profile().unwrap().digest
        );
    }
}
