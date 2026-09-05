# Setups

A setup is the complete Pi Coding Agent harness state: the system-prompt
components and the whole configuration, as a verbatim tree.

Three sources produce one immutable setup definition, and they converge before
any plan is made:

```text
a directory here             ─┐
an ai-stp setup              ─┼─▶ SetupDefinition ─▶ HarnessBundle ─▶ plan ─▶ apply
a set of ai-stp components   ─┘
```

Channel and marketplace are acquisition or projection metadata. They are never
setup identity — the same components acquired two ways are the same setup.

Component kinds and projection kinds are owned by
`provider-kit/v3/manifest.json`. They are not listed here, because a vocabulary
written in prose in a second place diverges from the executable source, and the
divergence is found only after someone has implemented the prose.

## What ships here

```text
setups/
  <setup-id>/
    setup.json    posture id, description, sources, and corpus identity
    home/         copied verbatim into the target
```

A setup that writes a configuration file names its `sources`: the vendor pages
that decided the keys inside it. Owning the right path and then writing a key
the product does not read produces a target that looks configured and is not,
so a setup writing anything other than documents is refused without them.

A setup's identity is its content: two setups with the same bytes have the same
definition digest whatever they are called. A setup that would write outside the
entries this provider owns is refused before anything is written — otherwise it
would leave files `remove` does not withdraw and `status` does not account for.
