# Support

## Before opening anything

`--help` states what this build does and does not do. `status --target <dir>
--json` reports what it found in a target without changing it, and its output is
safe to share: it carries identities and digests, never secret values.

## Where to go

| You have | Go to |
| --- | --- |
| A defect | [Issues](../../issues) — use the defect template |
| A question about behaviour | [Issues](../../issues) — a blank issue is fine |
| A vulnerability | [Security advisories](../../security/advisories/new), privately |

Never open a public issue for a vulnerability, and never paste credentials,
tokens, or the contents of a backup slot anywhere in this repository. A backup
slot holds whatever the target held when it was captured.

## What is not supported

This build applies `backup`, `restore` and `remove`. `install` and `replace` are
planned and then refuse, because no reader for `ai-stp-bundle/1` ships yet — that
is a known limitation, not a defect. The software lifecycle and `launch` are not
declared at all.

## Response

One maintainer. Defects are triaged as time allows; security reports are
acknowledged first.
