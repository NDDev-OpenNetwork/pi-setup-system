# Writing this harness's instruction file

Generated from `references/pi-baseline.json`. Do not edit:
the next render overwrites it, and the baseline is where a correction
belongs.

## Where it goes

`~/.pi/agent/AGENTS.md`

Decided by: https://pi.dev/docs/latest/sdk

## What the record says about it

**Re-asked at 0.84.4 on 2026-08-31.** The pinned npm package was fetched and its `integrity` checked before a byte was read; the row still stands (AGENTS.md 57). Searched with three controls -- `nddev_invented_reference_line`, `~/.nddev-not-a-home/skills/` and `NDDEV_INVENTED_SURFACE` -- each zero in the same bytes, so the search discriminates.

## Where the other harnesses keep theirs

| harness | path | shape |
|---|---|---|
| `antigravity` | `config/rules` | directory |
| `claude` | `CLAUDE.md` | file |
| `codex` | `AGENTS.md` | file |
| `cursor` | `rules` | directory |
| `grok` | `AGENTS.md` | file |
| `opencode` | `AGENTS.md` | file |
| **this one** | `AGENTS.md` | file |

**They are not interchangeable, and the difference is not only the
name.** Two of the seven take a *directory* of rules rather than a
single document, so a file moved between the two is not a rename.

**Some products read a neighbour's.** `references/surfaces.md` records
every such cross-read this estate has measured, on the declined rows:
a file written for one product can change what a second one sees, and
removing a setup can change what a third one sees. That is a property
of the products, not of this program, and it is the reason the declined
list is worth reading before writing here.

## Before you write one

- **This file is the floor, not the ceiling.** A repository's own
  instructions sit above it; write what is true everywhere and leave
  the rest to the project.
- **Read it back where the product reads it**, not where the install
  put it. Several of these products resolve a home through an override
  chain, and the two are not always the same directory.

## The owned region inside it

A setup's `instruction` component does not replace `AGENTS.md`;
the consumer's text lives inside one marked region spliced between
`:::begin-ai-stp` and `:::end-ai-stp` -- visible markers, because at
least one product strips HTML comments. Every byte outside the pair
is preserved, and re-applying identical bytes is a no-op.

- **`patch_instruction_region`** is the operation that changes it,
  carrying the new section under `--instruction-section` -- exactly
  one ordered marker pair. Planning records the file's observed
  digest and presence; apply re-reads both and refuses a drifted
  file rather than splicing into text it did not measure.
- **`detach_instruction_region`** removes the marked section and
  nothing else. A file that held only the region is removed with
  it; a file that never had one detaches to itself, so a repeat
  is a no-op rather than an error.
- **Both refuse a `--target-scope`.** The region lives at the
  target root, a scoped request cannot address it, and `status`
  reports `instruction_region: null` under a scoped measure.
- **It is not whole-setup payload.** Install, replace, remove and
  reset preserve or detach the region through the kernel's two
  hooks instead of treating it as text they are free to empty.
- **Ambiguous markers refuse at plan time.** An unpaired or
  out-of-order marker fails before anything is written -- never
  as a mid-apply surprise.

