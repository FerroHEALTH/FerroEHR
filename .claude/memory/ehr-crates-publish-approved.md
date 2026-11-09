---
name: ehr-crates-publish-approved
description: The owner approves the orchestrator to run FerroEHR's openehr-* crates publish between releases and approve its crates-io gate
metadata:
  type: feedback
---

The orchestrator may run this repository's `publish-crates.yml` with `publish=true`
itself, between releases, and approve its `crates-io` environment. Do this
whenever a fix in an `openehr-*` crate is merged on `main` and a
consumer (FerroBRIDGE, FerroFED) waits for it. Don't wait for the v5.x tag,
and don't ask first.

```
gh workflow run publish-crates.yml --repo FerroHEALTH/FerroEHR --ref main -f publish=true
```

Then approve the run's pending `crates-io` deployment and check crates.io for
the new versions.

**Why:** owner, 2026-11-08: "i approve you to release mannaly the EHR next
time your self". The owner had to run it by hand once, because the session's
permission check refused the dispatch. If the check refuses it again, tell the
owner that a Bash permission rule for this command would let it run, and
don't route around the refusal.

**How to apply:** after an `openehr-*` fix merges with a waiting consumer,
dispatch, approve the gate, confirm the versions on crates.io, and tell the
consumer's worker. Related: [[shared-crate-blockers]].
