# Before Handing Off

## Which repository you are in decides what you can run

This setup ships in two places and the commands below exist in only one of them.
**They belong to the source workspace that publishes this public tree.** A
checkout of this public repository carries
`crates/`, `setups/`, `references/` and `scripts/evidence.py` -- and neither
`scripts/gate.sh` nor `tools/`.

That is not a gap to fill. A rendered tree is generated: the fix for anything
here is a change in the authoring repository and a re-render, never an edit to
this checkout. What a reader of *this* tree can run is `cargo fmt --all
--check`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`, which is what its own CI runs.

Naming a command a reader cannot run used to be the whole of this page, and the
reader is a model, which will try it and then work around the failure rather
than say so.

## The gate, in the authoring repository

One entry point, from its root:

```bash
scripts/gate.sh
```

It exists rather than four bare `cargo` commands because the workspace pins a
toolchain that a local `cargo` earlier on `PATH` will shadow, and a green run
under the wrong compiler is worse than a red one.

## And the render, whenever the output could have moved

```bash
scripts/gate.sh --render                # the question this ref can answer
scripts/check_render.sh --deterministic # does the renderer agree with itself?
```

Run it for any change to `crates/`, `setups/`, `references/`, `provider-kit/`
or the renderer. **The plain gate does not render**, and a change to the
renderer that only passes it has not been checked at all -- that has reached CI
more than once.

The strict form clones the seven from their remotes, so it answers a question no
local checkout can: are the published trees actually current? It runs on `main`
and hourly, never on a branch, because a branch has published nothing yet.

## The rule that does not move

Never weaken an invariant, raise a threshold, silence a check or delete a test
to buy green.

Every new guard is observed **failing on the defect it describes** before it is
kept -- and once per branch, not once per guard. A guard whose test has never
been red proves nothing, and this estate has twice found a new guard's first
test passing under a mutation because every case it named exercised the same
branch.

## Classifying a finding is not silencing it

A false positive dismissed with its reasoning recorded is correct. Rewriting
code until a checker goes quiet is not. The difference is whether the change
stands on its own merits: if the code was worse for a reason that has nothing to
do with the checker, fix it; if it was not, dismiss the finding and say why.
