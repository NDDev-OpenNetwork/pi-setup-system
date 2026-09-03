# Before Handing Off

Run the checks this tree's CI runs, in order, and report what each one said
rather than that it passed.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If a command here is not present, say so rather than working around it.

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
