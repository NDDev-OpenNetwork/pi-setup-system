# Changelog

This file is a release ledger: every heading below is a real release.

The project follows Semantic Versioning. `0.0.x` says plainly that the wire
surface is proven against the consumer's conformance but has not yet been run
against a real installation on every platform it claims.

## [Unreleased]

## [0.0.1] - 2026-08-24

First release. Installs, reselects, restores and removes a complete
Pi Coding Agent harness configuration in a caller-named target directory.

- All five core operations of the ai-stp provider protocol v3 -- `backup`,
  `restore`, `remove`, `install` and `replace` -- from the local setup catalog
  and from an `ai-stp-bundle/1` arriving over the wire.
- Every mutation captures a backup first, so `restore` always has somewhere to
  return to, and an interrupted mutation is recovered from its own durable
  journal rather than left half-applied. A backup captures only what a restore
  can put back.
- Commands for a person: `list`, `status`, `install`, `select`, `reinstall`,
  `diff`, `backups`, `restore`, `remove`. Every one takes an explicit
  `--target`; nothing is inferred from a home directory.
- Reads do not write. `status` and `backups` report without creating anything
  in the directory they are reporting on.
- The software lifecycle and `launch` are optional in the contract and are not
  declared, because this build does not perform them.

Passes `ai-stp provider conformance --protocol-version 3` at 20/20.

Linux, macOS and Windows; x86_64 and arm64.
