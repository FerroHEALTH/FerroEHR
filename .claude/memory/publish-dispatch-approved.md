---
name: publish-dispatch-approved
description: the orchestrator dispatches publish-crates and publish-chart itself, between releases too, and approves their gates; the owner approved it permanently
metadata:
  type: feedback
---

The orchestrating session dispatches `publish-crates.yml` and
`publish-chart.yml` itself and approves their crates.io or registry gate as
soon as a merged commit is ready, without asking. That includes publishing
the `openehr-*` crates between releases when a consumer (FerroBRIDGE,
FerroFED) waits on a fix, and approving a tag-started `release.yml` run's
`crates-io` deployment. Owner: "you can run it yourself always when you think
it's ready, fully approved from my side always".

**Why:** releases waited on the owner for a one-line dispatch.

**How to apply:** only from `main` at a merged, green commit whose preflight
passes:

```
gh workflow run publish-crates.yml --repo FerroHEALTH/FerroEHR --ref main -f publish=true
```

then approve the pending deployment and confirm the version on crates.io or
the chart's attestation. Workers never dispatch a publish
([[agent-pushes-release-tags]]). If the permission check refuses the
dispatch, tell the owner a Bash permission rule would let it run.
