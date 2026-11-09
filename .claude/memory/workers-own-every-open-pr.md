---
name: workers-own-every-open-pr
description: Every worker drives every PR it opened to merge; the orchestrator checks every open PR in every repository in play for conflicts and red checks after each merge and hand-back, red main first (owner 2026-11-01, 2026-11-03)
metadata:
  type: feedback
---

A worker owns every open PR it opened, not only its latest one. Before it starts new work, and while it waits, it lists its open PRs and drives each to merge: arm auto-merge if it is off, fix a failing check, merge `origin/main` in on a conflict, re-run a job once when it failed only on a GitHub outage. It never leaves a PR red or unmerged.

The orchestrator does not wait for a report or for the owner to paste a red PR. After each merge and each hand-back it lists every open PR in the repositories in play (`gh pr list` with `mergeStateStatus`, `gh pr checks`) and fixes what it finds. A red `main` comes first, because every open PR inherits it. A background watcher arms auto-merge on our own ready PRs that lack it, and closes and reopens once any Actions-bot PR with no CI (a PR opened with `GITHUB_TOKEN` starts no workflows).

**Why:** on 2026-11-01 `main` had been red for days on one stale test and three PRs carried their own failures; nobody noticed until the owner pasted them ("you need to do it autonomic okay, going through the issues"). On 2026-11-03 the owner saw many open PRs across the repositories that no one was moving.

**How to apply:** put this rule in every worker brief. When the owner asks about open PRs, list every repository's open PRs with merge state, failing checks and queue entry before saying anything is fine. Related: [[workers-never-idle-on-merge-queue]], [[pr-backlog-discipline]].
