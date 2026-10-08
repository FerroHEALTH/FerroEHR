---
name: stacked-prs-after-squash
description: "Stacked PRs conflict after the lower one is squash-merged; replay only the PR's own commits with rebase --onto"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 44d54adc-7226-4d71-bc1a-2ecc4709cb40
  modified: 2026-10-08T06:55:27.207Z
---

Main merges are squash merges, so a PR stacked on another branch still carries the lower PR's original commits after that one merges, and GitHub reports `CHANGELOG.md` (and others) as conflicting.

**Why:** happened 2026-10-08 with #3695/#3697 after #3688 and #3693 were squash-merged; the owner saw "This branch has conflicts".

**How to apply:** `git fetch --prune`, then `git rebase --onto origin/main <last commit of the lower PR> <branch>` so only the branch's own commits replay, and repeat up the stack (`--onto <new lower tip> <old lower tip>`). The remote branch may have been rewritten by GitHub's stack retarget: compare its tree to the local commit before overwriting, then `git push --force-with-lease=<branch>:<exact remote sha>`. Prefer opening stacked PRs only when needed; see [[push-pr-fixes-without-asking]].
