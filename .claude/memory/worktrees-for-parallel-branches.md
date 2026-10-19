---
name: worktrees-for-parallel-branches
description: "Owner 2026-10-19: git worktrees are allowed on FerroEHR; use one for any branch work beside a release or another worker, and leave other worktrees alone; supersedes the 2026-08-06 no-worktrees rule"
metadata:
  type: feedback
---

Git worktrees are allowed. Use one (`git worktree add ../ferroehr-wt-<slug> -b <type>/<slug> origin/main`) whenever branch work runs beside other work in the same clone: a release cut in flight, another worker or session on its own branch, or a fix you push while a PR of yours waits on CI. Work on the main checkout's own branch can stay in the main checkout.

**Why:** on 2026-08-06 the owner banned worktrees so the work could be followed in one directory. On 2026-10-19 the owner said worktrees ARE allowed: with release cuts, FerroUI rollouts and fixes running side by side, a shared checkout made workers switch branches under each other.

**How to apply:**

- Put each worktree beside the clone (`../ferroehr-wt-<slug>`) or under `.claude/worktrees/` (git-ignored), never in a tracked path. Never touch, reset or remove a worktree another worker created; `git worktree list` shows them.
- Remove your worktree and its local branch when the PR merges (`git worktree remove`, `git branch -D`), so the IDE does not show stale branches.
- The ONE `./target` rule in `CLAUDE.md` still holds: a worktree gets no second target directory and no `CARGO_TARGET_DIR` override. Run cargo in the main checkout, or let CI build the branch.
- `docs-dist` stays remote-only (2026-09-04): it is the branch the docs pipeline writes; never check it out locally, in a worktree or otherwise.

See also [[concurrent-sessions-shared-tree]].
