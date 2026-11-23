---
name: definitions-change-in-place
description: until FerroEHR has a production release, a shipped definition (archetype, template) changes in place at v1.0.0; never a new version id because a release candidate published it (owner, 2026-11-14)
metadata:
  type: feedback
---

A release candidate is not production. Until FerroEHR has a production
release, a shipped definition changes in place at its current version, with
its digest updated in `deploy/definitions/published.sha256`. Never create a
new version id (v1.1.0 and so on) only because a release candidate published
the definition; fold any such version back into v1.0.0. Owner, 2026-11-14:
"off course the v1.0.0 can chnage because nothng is in production".

**Why:** nothing is in production ([[ferrohealth-not-in-production]]).

**How to apply:** tell every worker that changes a definition to change it in
place. Once a production release exists, the never-change-under-its-id guard
applies again.
