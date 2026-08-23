## What changed, and why it matters

<!-- The subject of the change, not the list of files. A reader should learn
     what is different about the program now. -->

## How it was verified

<!-- Name what you ran. If a new guard was added, say that it was observed
     failing on the defect it describes before it was kept. -->

- [ ] `cargo test --locked --all-targets`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo fmt --all --check`

## Contract

<!-- Delete the lines that do not apply. -->

- [ ] No fact the provider kit or the baseline owns is restated in prose here.
- [ ] Nothing reaches a target except through `setup-core`.
- [ ] Any new refusal carries a stable reason code, not only a message.
