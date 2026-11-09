---
name: update-branch-is-unsigned
description: gh pr update-branch (and the GitHub "Update branch" button) creates an unsigned merge commit that the main ruleset's required-signatures rule blocks; refresh a stale PR branch with a local signed rebase
metadata:
  type: feedback
---

Never refresh a stale pull-request branch with `gh pr update-branch` or the web "Update branch" button. Both create a merge commit through the GitHub API, which is unsigned, and the `main` ruleset requires signatures, so the PR sits at `mergeStateStatus: BLOCKED` with every check green and `verification.reason = "unsigned"` on the head commit.

**Why:** seen on #3432 (2026-09-16); a local `git rebase origin/main` plus force-push of the branch cleared it at once.

**How to apply:** on the PR branch, `git fetch origin && git merge origin/main` (the merge commit signed locally by `commit.gpgsign`) and a plain `git push`. No force-push: the family worker rule brings a branch up to date by merging `origin/main` in, and the merge queue makes a stale branch harmless anyway ([[merge-queue-signed]]). Same root cause as [[api-commits-are-unsigned]].
