---
name: preflight-merge-commit-before-tag
description: before tagging a FerroEHR release, run scripts/release/preflight.sh on the release PR's merge commit, never only on the release branch
metadata:
  type: feedback
---

Before pushing a FerroEHR release tag, check out the release PR's merge
commit in a throwaway worktree and run `scripts/release/preflight.sh vX.Y.Z`
there. Tag only if it passes.

**Why:** a tag is never moved or deleted, so a release the workflow refuses
burns the version; a hand merge skips the merge queue's checks.

**How to apply:** run the preflight on the merge commit before every tag,
whatever the queue said. A release PR merges through the queue, never by
hand. If the preflight fails, cut the next candidate.
