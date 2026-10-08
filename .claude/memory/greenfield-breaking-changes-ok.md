---
name: greenfield-breaking-changes-ok
description: Owner 2026-10-08 — FerroEHR is still effectively greenfield; removing routes/config keys may ship in a 4.3.x patch release without a major bump
metadata:
  node_type: memory
  type: feedback
  originSessionId: 44d54adc-7226-4d71-bc1a-2ecc4709cb40
  modified: 2026-10-08T08:21:44.857Z
---

Breaking changes (removed REST routes, removed config keys) may ship in the current 4.3.x line; do not push them to a 5.0.0 or a deprecate-then-remove cycle.

**Why:** the owner said "we do not care okay because it's still sort of greenfield okay!!! so no real production apps use it" when the custom FHIR mapper was removed in favour of FerroBRIDGE (2026-10-08).

**How to apply:** remove superseded code outright in the release being cut; still record it under `### Removed` in CHANGELOG.md and update the book in the same PR. Same spirit as the migration-rewrite ruling ("the rewrite is breaking changes only"). Re-ask only if a real production deployment is mentioned.
