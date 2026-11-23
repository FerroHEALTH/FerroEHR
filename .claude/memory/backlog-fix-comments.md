---
name: backlog-fix-comments
description: when a fix for a Cadasto/FerroHEALTH-backlog issue merges, comment on the backlog issue and set it to In review (owner, 2026-11-17)
metadata:
  type: feedback
---

When a fix for an issue reported in Cadasto/FerroHEALTH-backlog merges in a
product repository, post a comment on the backlog issue: what was wrong, what
changed (PR and merge SHA), the release it ships in, and how to verify it,
in a plain, short owner voice. Then set its Status on the Cadasto
"FerroHealth" project (#1) to In review, never Done; the owner and the
reporter close it.

**Why:** the owner wants the testers kept informed on their own tracker.

**How to apply:** put the rule in every worker brief that touches a
backlog-derived issue, and check it at each merge. Related:
[[verify-before-implementing-old-issues]].
