---
name: max-five-concurrent-workers
description: Owner cap for the whole family since 2026-10-19 - at most 5 implementation subagents at once; the owner may raise it on request
metadata:
  type: feedback
---

Owner ruling (2026-10-19, family-wide): **at most 5 implementation subagents
run at the same time.** The owner may raise the cap on request; ask before
going wider. It replaces the cap of 2 set on 2026-07-12 during W-3f.

**Why:** parallel Opus workers burn tokens fast; the first cap of 2 came
after 12-wide fan-outs proved too expensive, and the owner later set 5 for
every product.

**How to apply:** launch at most 5 implementation workers at once and queue
the rest. The cap is an upper bound for disjoint work only: same-phase
refactors still run one worker at a time with hard file fences
([[one-worker-per-phase-hard-fences]]), and parallel writers each get their
own worktree. Related: [[owner-work-style]], [[concurrent-sessions-shared-tree]].
