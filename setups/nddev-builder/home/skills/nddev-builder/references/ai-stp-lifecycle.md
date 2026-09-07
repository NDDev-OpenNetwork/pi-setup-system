# Build a complete setup with ai-stp

A setup is a complete configuration of one chosen harness: a working collection
of tools for a user outcome. It is more than one plugin or a set of unrelated
files. Use this workflow to create a new setup, improve an existing collection,
or recast it for another harness. Provider development is a separate task.

Start with `ai-stp doctor --json` and resolve command arguments from
`ai-stp help --agent --json` in the installed consumer. Do not invent options or
assume a newer development command is available in a released CLI.

## 1. Define the outcome and inspect existing tools

Record the intended tasks, chosen harness, operating systems, installation
scopes and the user's existing authority. Name concrete acceptance scenarios,
such as building a tested application, reviewing a change, or maintaining an
MCP integration. Inspect the explicitly named authoring directories with
`ai-stp component inventory`; use `ai-stp component discover` for native
configuration. Discovery does not adopt files or establish ownership.

Build a capability inventory: the outcome each component enables, its source,
exact version, native entry point, scope, dependencies, external accounts,
activation needs and evidence. Reuse a suitable existing component before
creating another. Explain overlap and omitted capabilities. Choose only the
tools the intended tasks need; a large file count is not completeness.

## 2. Author native components

Use `ai-stp setup scaffold plan` / `ai-stp setup scaffold apply` for a complete
authoring tree, or `ai-stp component scaffold plan` / `ai-stp component scaffold apply` for a missing member. Replace every draft marker with useful content.
Keep authored sources and generated harness projections distinct.

Read this harness's surfaces and per-kind references before choosing paths or
keys. Put durable context in instructions, repeatable procedures in skills,
external tool connections in MCP, lifecycle callbacks in hooks, and narrowly
scoped delegation in native agents where supported. A plugin packages the
capabilities its own harness supports; it is not itself the whole setup.
Shared executables use the consumer's `cli` component lifecycle and are not
slash commands. Do not create a new component kind for a descriptive category.

Use `ai-stp component passport validate` for metadata and
`ai-stp component skill validate` for a skill package. Validate the native file
format and demonstrate discovery in the actual product separately. Passing a
parser does not prove the harness discovers, trusts or executes the component.
Keep credential values out of the artifact; document only required variable
names or the product's account connection procedure.

## 3. Compose one exact graph

Freeze authored components with `ai-stp component version release`. Compose
exact sources through `ai-stp setup compose plan` / `ai-stp setup compose apply`,
or select registered components through `ai-stp select propose` /
`ai-stp select confirm`. Apply the exact returned plan, after revalidating its
preconditions. Inspect dependency closure, path and key conflicts, scope
compatibility, executable prerequisites and conversion losses with the
consumer's graph and report commands. Resolve conflicts before installation.

A setup stays bound to one harness. To derive another, use `ai-stp setup recast plan` / `ai-stp setup recast apply`; inspect the destination's native files,
semantic losses and provenance. Do not relabel the original or copy one
harness's config into another. A shared instruction or skill format does not
make permissions, hooks, agents or plugin manifests interchangeable.

## 4. Prove the collection works

Use `ai-stp eval plan` / `ai-stp eval run` for the setup's own adaptations, and
`ai-stp eval component plan` / `ai-stp eval component run` when evaluating all
adaptations of a component. Local static evaluation is not a security scan or
an authenticated product run; retain those evidence distinctions.

Build and review the exact bundle. For a single scope use `ai-stp install plan`,
`ai-stp install approve` with the returned digest, then `ai-stp install apply`.
For a setup spanning roots use `ai-stp install transaction plan`,
`ai-stp install transaction approve`, and its matching apply/recovery commands.
These approval commands record the exact effect already authorized by the
task; they do not require another user question for that same effect.
Exercise this first
in disposable homes, targets and prefixes. Read `ai-stp target status`, diff
and backups; an exit code alone is not a verified effect. Preserve any pending
authorization, refusal or unknown outcome as such and follow its recovery
path. Demonstrate restore and verify that pre-existing files survive.

Run each acceptance scenario through the real harness, including one implicit
and one explicit invocation where supported. Check missing dependencies and
conflicting components as well as the happy path. Record the exact harness,
provider and consumer versions, OS/architecture, artifact digests and results.
An unavailable credential or platform is unmeasured, never a passing cell.

## 5. Deliver a usable setup

Write a concise setup guide with its purpose, supported tasks, component and
capability inventory, native activation/invocation examples, required accounts,
scope, compatibility, evidence limits, update path and backup/restore path.
Separate built-in harness features from features supplied by this setup.
Use current vendor documentation and the measured product version; cite the
source for a feature claim instead of promising parity across harnesses.

Use `ai-stp setup export` for a reviewable tree. For a requested publication,
use `ai-stp component publish` or `ai-stp setup publish plan` followed by
`ai-stp setup publish confirm` on the reviewed exact set. Preserve immutable
versions. Task authority is separate from verification; changing an existing
object's visibility or access rights needs the user's decision. Report what
was created, where it is, how to invoke it, what passed and what remains
unmeasured. Deliver authoring artifacts without modifying the running agent's
own active configuration.
