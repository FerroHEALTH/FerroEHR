---
name: workers-never-idle-on-merge-queue
description: A worker whose PR sits in a merge queue starts its next independent issue in a fresh worktree instead of waiting (owner 2026-11-03)
metadata:
  type: feedback
---

A worker never idles while its PR waits in a merge queue. It leaves a background wait running, starts its next issue that touches none of the queued PR's files in a fresh worktree from `origin/main`, and comes back to the queued PR when the wait fires.

**Why:** on 2026-11-03 the owner saw workers running for 5 to 6 hours and asked if they had stalled. Each queue run repeats the full CI (30 to 60 minutes per PR in the larger repositories), and workers waited on each merge before starting the next issue.

**How to apply:** put this in every worker brief. A queued PR shows `autoMergeRequest: null` with a `mergeQueueEntry`; read the entry through GraphQL before taking the null to mean auto-merge is off. When a worker has been quiet for a while, check whether its PR is queued before deciding it is stuck. Related: [[pr-auto-merge]], [[workers-own-every-open-pr]].
