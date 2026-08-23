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

No setups are published in this tree yet.
