---
name: push-pr-fixes-without-asking
description: "Owner 2026-10-07 — when fixing a red PR, commit and push the fix to its branch without asking first"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 44d54adc-7226-4d71-bc1a-2ecc4709cb40
  modified: 2026-10-07T19:08:37.774Z
---

When asked to look into why a PR fails, fix the failures, verify locally and push to the PR branch directly, without asking.

**Why:** the owner said "yes yes push why ask me?" after being asked for permission to push CI fixes to their own PR (#3672).

**How to apply:** this covers fix-forward pushes to an existing PR branch. Merging stays the owner's click (see [[merge-on-local-gates]]). Force-pushes and pushes to `main` are not covered.
