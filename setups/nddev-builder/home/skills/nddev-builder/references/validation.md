# Before Handing Off

Run the checks this tree's CI runs, in order, and report what each one said
rather than that it passed.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If a command here is not present, say so rather than working around it.

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

The wire surface is checked by the consumer's own runner, not by anything here.
Ask `pi-setup-system provider-info` for `harness_id`; that is the value
`--harness` takes, and it is not always the directory name.

```bash
ai-stp provider conformance --harness <harness_id> \
  --executable target/release/pi-setup-system \
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

## What this toolkit does not do

- It does not push, tag, or release.
- It does not write a live configuration home.
- It does not install software or start a product.
