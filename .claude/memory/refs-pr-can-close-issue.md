---
name: refs-pr-can-close-issue
description: A PR that only "Refs #N" (with "(#N)" in its commit subject) was still recorded as closing #N and closed it on merge; check closingIssuesReferences before merging a plan-only PR
metadata:
  type: feedback
---

PR #3583 (plan files only) said "Refs #3577" in its body and "(#3577)" in the commit subject; GitHub listed #3577 in `closingIssuesReferences` and closed it on merge (2026-10-05). It was reopened by hand.

**Why:** the issue loop relies on the merge closing exactly the issues named with Closes; an early close drops live work off the worklist and the board.

**How to apply:** before `gh pr merge` on a PR that must NOT close an issue, run `gh pr view <n> --json closingIssuesReferences`; if the issue is listed, remove the "(#N)" from the commit subject and unlink it in the PR's Development panel, then re-check. See [[pr-closes-one-keyword-per-issue]].
