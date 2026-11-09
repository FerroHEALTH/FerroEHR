---
name: max-five-concurrent-workers
description: Family worker cap - up to 10 implementation subagents at once since 2026-11-03 (was 5 from 2026-10-19, 2 before); every worker on Opus, started with the Agent tool
metadata:
  type: feedback
---

Owner ruling (2026-11-03, family-wide): **up to 10 implementation subagents run at the same time** ("you are allowed to have 10 workers okay!!"). It replaces the cap of 5 set on 2026-10-19, which replaced the cap of 2 set on 2026-07-12 during W-3f.

**Why:** the owner asked why so few workers ran while every product had open work. The first cap of 2 came after 12-wide fan-outs proved too expensive.

**How to apply:** launch at most 10 implementation workers at once, each on `model: 'opus'` ([[workers-on-opus-only]]) through the `Agent` tool ([[no-workflows-for-workers]]). The cap is an upper bound for disjoint work only: same-phase refactors still run one worker at a time with hard file fences ([[one-worker-per-phase-hard-fences]]), and parallel writers each get their own worktree ([[worktrees-for-parallel-branches]]). Related: [[owner-work-style]], [[concurrent-sessions-shared-tree]].
