---
name: one-worker-per-phase-hard-fences
description: 2026-08-02 incident — concurrent workers collided in tools/openehr-codegen; within a refactor phase run ONE worker at a time with hard file fences; reverts are orchestrator-only
metadata: 
  node_type: memory
  type: feedback
  originSessionId: af8ec1a8-3953-4ae1-a5d1-355a712f597b
  modified: 2026-08-03T11:25:10.305Z
---

Incident 2026-08-02 (foundation phase): round 3 (app-scoped) and the
model-query worker (codegen-scoped) ran concurrently; round 3's RESULT_SET
task led it into the emitter — the crate the other worker was editing — and
an owner-ordered revert then unwound shared files under live work. Recovery:
freeze both, tar-backup all dirt to the scratchpad, reset uncommitted state
to HEAD, redo sequentially. Nothing committed was lost.

**Rules now standing:**
- Within a refactor phase, ONE implementation worker at a time — the
  worker cap ([[max-five-concurrent-workers]]) is an upper bound for genuinely disjoint work (e.g. catalogue
  vs app), never for same-phase refactors where scope can bleed.
- Every worker brief carries a HARD FILE FENCE: "touching any path outside
  <list> = STOP and report" — a task that turns out to need an out-of-fence
  change reports the need (it may be another leg's work), never does it.
- REVERTS ARE ORCHESTRATOR-ONLY: a worker never runs git checkout/restore/
  clean on shared state; it reports what should be reverted.
- Before any recovery reset: tar-backup every dirty file to the scratchpad;
  verify committed history intact; keep .claude memory dirt out of resets.
- **2026-08-03 corollary: ONE WRITE lane at a time IN THE MAIN TREE —
  including the orchestrator.** Two write workers (or a worker + an
  in-session fix) sharing one working tree cross-contaminate each other's
  cargo gate runs even on disjoint files, and any `git checkout -b` by one
  lane silently moves the branch pointer under the other. Disjoint-scope
  concurrency is safe only for READ-ONLY agents.
- **2026-08-03 refinement (owner): WORKTREE isolation unlocks parallel
  write lanes for small/medium shots.** `Agent(isolation: "worktree")`
  gives the worker its own checkout — no file or branch interference. The
  cost is a COLD `./target` per worktree (the one-target rule's disk
  physics), so: worktree workers run SCOPED gates only (`-p` touched
  crates, never `--workspace`); the orchestrator collects the diff, runs
  the FULL battery once at convergence in the main tree, and DELETES the
  worktree + its target after merging. Big implementation rounds stay
  serial in the main tree (cold build + convergence overhead eats the
  win).
Related: [[concurrent-sessions-shared-tree]], [[max-five-concurrent-workers]].
