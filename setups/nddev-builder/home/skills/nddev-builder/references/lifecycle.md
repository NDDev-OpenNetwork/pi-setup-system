# The Commands This Program Answers

Use this reference when changing how a target is installed, observed, restored
or removed.

Ask the binary rather than this file if the two ever disagree:
`pi-setup-system` with no arguments prints every command it has.

```text
list                                          every setup this build carries
status    --target <dir>                      what a target holds, changing nothing
install   <setup> --target <dir>              write a setup into a target
select    <setup> --target <dir>              reach a different setup's complete state
reinstall --target <dir>                      write the applied setup again
diff      --target <dir>                      what drifted since it was applied
backups   --target <dir>                      the slots, newest first
restore   [--backup <ref>] --target <dir>     the last backup, or a named one
hold      --backup <ref> [--reason <why>] --target <dir>
release   --backup <ref> --target <dir>
remove    --target <dir>                      everything this program owns
software  --prefix <dir>                      which product versions a prefix holds
rollback  --to <version> --prefix <dir>       point the command at one already there
```

There is no `--json` on these. JSON is the **provider** surface --
`provider-info`, `status --target <dir> --json`, `validate-bundle`,
`plan-operation`, `apply-operation`, `recover-operation` -- and a consumer calls
those.

## Invariants worth knowing before changing anything

- **The target is named, never guessed.** Absolute, existing, a directory, and
  its final component not a symbolic link. Nothing is inferred from `$HOME`, the
  working directory, or the documented configuration home.
- **A backup is captured before every change**, so `restore` always has
  somewhere to go. The pool is bounded; a held slot is not reclaimed and is not
  counted against the bound.
- **There is one write path.** A human command builds a real plan and calls the
  same `perform` the wire surface does, so a human command cannot bypass a
  guarantee the provider owes its consumer.
- **`remove` takes owned namespaces whole**, except under `target_scope
  user_root`, where it takes only the files this provider recorded writing --
  because that root is shared with other products and taking a namespace whole
  would take a neighbour's content.

## The software half

`software_install`, `software_update` and `software_remove` are the product's
own program, not its configuration. They live under `--prefix`, never under
`--target`: one program can serve several targets, and a program inside a target
would claim a path this build promises not to touch.

Each build pins two versions -- the current one and the one before it -- so an
update has somewhere to come from and a rollback somewhere to return to. A bump
moves the current pin into the second slot rather than adding a second choice.
