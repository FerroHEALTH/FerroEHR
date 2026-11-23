---
name: verify-before-implementing-old-issues
description: before assigning or building an older issue, check it against current main and the current spec; close what is done or obsolete with evidence, build only what is still open (owner, 2026-11-16)
metadata:
  type: feedback
---

Before an older issue moves into the current milestone or goes to a worker,
check that it is still open work. It may already be done, superseded by later
changes, or resting on spec text that has moved on. Close what is done or
obsolete with the evidence posted on the issue; build only what remains.
Owner, 2026-11-16: "we should not just implement old things right?"

**Why:** building a stale issue spends a worker on work that is done or
wrong.

**How to apply:** when triaging a milestone or picking up an issue older than
a few days, check each against main and the cited spec first. A worker brief
for old issues starts with that check, and the worker reports what it closed
before it builds anything. Related: [[findings-drive-redesign]],
[[no-half-solutions]].
