# Security Policy

## Supported surface

Security reporting covers the setup catalog, the provider wire surface, the
human command surface, the lock, journal, backup and restore kernel, the
vendored provider kit, and the GitHub workflows in this repository. Only the
latest release is supported.

## Reporting a vulnerability

Report privately through
[GitHub Security Advisories](https://github.com/NDDev-OpenNetwork/pi-setup-system/security/advisories/new).

Do not publish exploit details, credentials, tokens, private configuration or
backup contents in an issue or pull request. A backup slot may contain whatever
the target held at capture time, so treat its contents as sensitive even when
the finding is not.

Include the affected command or path, reproduction steps, impact, and a
non-sensitive description of the environment.

## What this program is trusted with

It writes into a directory the caller names, captures backups of what was there
before, and reports identity digests. It does not read credentials, does not
send anything over the network in its configuration lifecycle, and records no
secret values in its state.

A finding that breaks any of those four statements is in scope regardless of
severity.
