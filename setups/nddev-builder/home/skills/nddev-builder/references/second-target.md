# The second target this harness owns

## `target_scope: user_root`, rooted at `~/.agents`

**`~/.agents` is not this product's configuration home.** It is a
different target, reached by a consumer naming the scope on the
request, and every path below is relative to that root rather than
to the home -- writing the root into the path again would nest it
twice, which is a mistake this estate has made and shipped.

| path | routes | decided by | exercised by |
|---|---|---|---|
| `skills` | skill | measured from the pinned bundle | read its bytes |
### `skills`, as measured

`package-manager.js:1976` builds `userAgentsSkillsDir = join(getHomeDir(), ".agents", "skills")` and line 2017 loads from it: `addResources("skills", collectAutoSkillEntries(userAgentsSkillsDir, "agents"), ...)`. No Pi page says so. A neighbouring use in `trust-manager.js:160` *excludes* this directory while walking up for a project-scoped one, which is what a first reading of the variable name would have mistaken for the read -- the line that matters is 2017, not 1976.

**`bytes` and not `ran`, and what was tried, so the next reader does not repeat it.** The pinned `0.84.4` bundle was installed into a temporary prefix and every credential-free entry point exercised: `pi list` reports installed *packages* and an auto-loaded skill is a resource rather than a package, `pi config` is a TUI, and `--verbose --offline --print` reaches provider selection and exits before any skill is resolved. There is no command that reports the resolved skill set without a credential. Codex, opencode and grok each have one; this product does not, and the absence is a property of the product rather than of the effort.

The one root in this estate that belongs to a convention rather than to a product. `$HOME/.agents/skills` is a *sibling* of this product's configuration home, not a child, so nothing declared against this provider's own target can reach it -- that is what `user_root` exists for.

**Owning a shared root, and the reason this record used to decline it.** Five of the seven products read this root, and the decline said: *a namespace is removed whole, so a second declaration would make either provider's remove take the other's skills.* That sentence was true when it was written and stopped being true when `written_paths` shipped -- `remove` under this scope takes the files this provider recorded writing and refuses rather than widening when it cannot read the record, and each harness carries its own state file, so they coexist under one root. The reason was not re-read when the thing it described changed.

Relative to this scope's own root the path is `skills`, not `.agents/skills`: the root is what the scope names, and writing it into the path again would put the skills at `~/.agents/.agents/skills`.


**A setup cannot carry one of these.** A setup is installed into one
target and its payload is relative to that target, so a component
for this scope is installed by the consumer against that root -- not
by a setup aimed at the configuration home. If you are looking for
where to put one by hand, it is the path above joined to the root
above, and nowhere under the home.

**The root is shared, and that changes what removal means.** Several
products read it. Under this scope `remove`, the backup and a
restore act on the files this provider recorded writing rather than
on the directory whole, so a neighbour's files are never captured
into a slot here and never reverted out of one.

