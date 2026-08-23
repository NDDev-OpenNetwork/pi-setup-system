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

The software lifecycle — installing, updating or removing the product itself —
and `launch` are optional in the provider contract and are not declared here,
because this build does not perform them. A provider that advertised them would
let a caller ask for something that cannot be honoured, which is worse than not
offering them.

All five core operations do work: `backup`, `restore`, `remove`, `install` and
`replace`, both from the local setup catalog and from an `ai-stp-bundle/1`
arriving over the wire.

## Response

One maintainer. Defects are triaged as time allows; security reports are
acknowledged first.
