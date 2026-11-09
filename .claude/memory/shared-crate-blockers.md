---
name: shared-crate-blockers
description: Every filed issue gets a milestone at filing; a defect in a shared family crate that a product hits goes in the owning repository's CURRENT milestone with a worker at once; no consumer workaround, ever (owner 2026-11-07)
metadata:
  type: feedback
---

Every issue filed in any family repository gets its milestone at filing time. If the right milestone does not exist yet (for example the next `ferrohealth-smart-vX.Y.Z` in FerroAUTH), create it with a due date and a description first. A defect in a shared crate that another product hits or works around goes in the owning repository's CURRENT release milestone, never a later one, even when the consumer has a workaround. Two FerroEHR issues placed in v5.9.0 had to be moved to v5.7.0 by the owner (#5147, blocking FerroBRIDGE #1689, and #5148, which FerroFED patched around).

The shared crates are FerroAUTH's `ferrohealth-smart` and `ferrohealth-authz`, FerroAUDIT's `ihe-atna` and `ehds-logging`, FerroSYS's `ferrohealth-*`, FerroUI's `ferrohealth-ui`, FerroEHR's `openehr-*`, FerroBRIDGE's `fhir-types` and `ferrobridge-term`, and FerroPIX's `ihe-iti`. When a worker reports or files an issue in one of them, find which product issues wait on it, give it the matching milestone, set the blocked-by edge, and start a worker on it at once. Tag and publish its release as soon as the release PR merges, so the consumer can move.

**No consumer workaround for a shared-crate defect, ever.** A worker that hits a defect in a family crate files it in the owning repository's current milestone, fixes it there, ships the release, and moves the consumer to it. If the consumer must wait for that release, the consumer issue waits. A workaround already in a tree gets a removal issue in the current milestone.

**Why:** owner, 2026-11-07. FerroAUTH #291, which FerroEHR #5144 needed for v5.7.0, was filed with no milestone and no worker, and the owner had to point it out: "we need to make sure that things keep moving".

**How to apply:** after every worker report, scan the issues it filed or named in another repository and set milestone, edge and worker. Related: [[next-milestone-always]], [[no-half-solutions]], [[family-architecture]].
