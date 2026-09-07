# Before Handing Off

For setup authoring, run the component validators, composition/evaluation and
disposable product scenarios described in the ai-stp lifecycle guidance. A
setup containing Python tools does not require a Rust provider checkout.

When changing provider implementation, run that checkout's CI checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Report each result and any unavailable check. The cargo commands apply only
to the provider implementation workspace.

## A lifecycle smoke test against a disposable target

Never against a live configuration home. A temporary directory outside the
repository:

```bash
target="$(mktemp -d)/pi-target"
mkdir -p "$target"
pi-setup-system install baseline    --target "$target"
pi-setup-system status              --target "$target"
pi-setup-system select full-auto    --target "$target"
pi-setup-system diff                --target "$target"
pi-setup-system backups             --target "$target"
pi-setup-system restore             --target "$target"
pi-setup-system remove              --target "$target"
```

The same sequence runs as a test in every published tree, on ubuntu, macos and
windows, against the binary that tree builds -- so a change that breaks it fails
before anyone types it.

## Conformance against the consumer

Use this additional check when changing or qualifying the provider itself.
The wire surface is checked by the consumer's own runner, not by anything here.
Ask `pi-setup-system provider-info` for `harness_id`; that is the value
`--harness` takes, and it is not always the directory name.

```bash
ai-stp provider conformance --harness <harness_id> \
  --executable <verified-provider-path> \
  --target <empty-dir> --protocol-version 3 --json
```

Report the verdict with the consumer version that gave it. An empty target and a
populated one are different questions. A defect that only appears against a real
home is the kind this project has already shipped.

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

## Task scope and disposable verification

Authoring includes creating files and running the required checks in disposable
homes, targets and prefixes. Installing verified prerequisites and launching a
product there are valid validation steps. Keep credentials and live state out
of those copies. Publishing or applying to a user's live target happens only
when the task includes that effect, through the exact reviewed lifecycle.
Never change the running agent's active configuration in place.
