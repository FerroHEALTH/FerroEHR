---
name: next-milestone-always
description: Every FerroHEALTH repository keeps a next milestone open after the active one, with a due date, so work that cannot finish in time has a home (owner 2026-11-03)
metadata:
  type: feedback
---

Each repository keeps at least one open milestone after the one being worked. Work that cannot be completed in the active milestone moves there, never into a milestone without a due date and never left without a milestone.

**Why:** the owner, 2026-11-03: "for the different repos do not forget to also create a new milestone after the current milestone where things can go in when they can't be completed during the active milestone you are working on".

**How to apply:** when a repository has only its active milestone, create the next one, `vX.Y.Z+1`, with a due date. When an issue cannot finish in time, move it there with a comment saying why, so the active milestone can still be cut. Related: [[new-issues-go-to-current-milestone]], [[shared-crate-blockers]].
