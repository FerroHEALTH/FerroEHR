---
name: admin-bypass-stays
description: The organisation-admin bypass on every family repository's main ruleset stays; never remove it, and no worker or orchestrator ever merges through it (owner 2026-10-21)
metadata:
  type: feedback
---

Every FerroHEALTH repository's `main` ruleset keeps its `OrganizationAdmin` bypass (mode `always`). Never remove or narrow it.

**Why:** on 2026-10-21 the orchestrator removed it from all nine repositories after an untested change reached FerroFED's `main` through it; the owner ordered it restored at once: "keep always the bypass do not remove it add it all back to every repo".

**How to apply:** the bypass is the owner's. Workers and the orchestrator share the owner's account, so the rule falls on them: never `gh pr merge --admin` or any other bypass, and never merge before the required `conclusion` check is green. Every merge goes through the merge queue or auto-merge ([[pr-auto-merge]], [[merge-queue-signed]]).
