---
name: preflight-merge-commit-before-tag
description: Before tagging a FerroEHR release, run scripts/release/preflight.sh on the release PR's merge commit, never only on the release branch
metadata:
  type: feedback
---

Before pushing a FerroEHR release tag, check out the release PR's merge commit (in a throwaway worktree inside the repo) and run `scripts/release/preflight.sh vX.Y.Z` there. Tag only if it passes. A tag is never moved or deleted, so a refused release burns the version.

**Why:** on 2026-11-02 both v5.6.0-rc8 and v5.6.0-rc9 were burnt. rc8 was refused by the vulnerability record on findings with no VEX judgement. rc9 was refused by the release `plan` job: #4800 and #4801 were removed from the merge queue and merged by hand before their queue runs finished, so the queue's `release-fragments` check (which had failed on #4801, as it should) was bypassed, and the fragments of #4799 and #4800 sat on the tagged commit without being in the rc9 section.

**How to apply:**
- Run the preflight on the merge commit before every FerroEHR tag, whatever the queue said: a hand merge skips the queue's checks.
- Ask the owner to let a release PR merge through the queue, not by hand.
- If the preflight fails, cut the next candidate rather than tagging.

Related: [[workers-own-every-open-pr]].
