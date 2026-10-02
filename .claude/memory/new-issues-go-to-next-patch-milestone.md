---
name: new-issues-go-to-next-patch-milestone
description: Owner ruling 2026-09-14: new issues filed during a cycle go to the NEXT patch milestone (v4.3.1 after v4.3.0), never to v5.x, even when breaking changes are allowed
metadata:
  type: feedback
---

New issues filed while working a milestone belong to the CURRENT milestone (fix-first) or the NEXT patch milestone (v4.3.1 when v4.3.0 is current). They never go to a v5.x milestone, even for a breaking rework.

**Why:** the owner said on 2026-09-14, after the storage-redesign sub-issues were filed into v5.0.0: "add all these new ones to milestone 4.3.1 please, not to v5". The v5.x milestones are the owner's own grouping for the application-layer refactors; the storage rework and everything blocked on it live in the patch line.

**How to apply:** `scripts/gh/fields.sh new … --milestone v4.3.1` (or the current one) for every issue filed this cycle; re-check `gh issue list --milestone v5.0.0` before ending a session and move anything I put there. Related: [[pr-closes-one-keyword-per-issue]], [[component-fixes-ride-current-patch]].
