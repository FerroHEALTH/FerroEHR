---
name: remove-worktrees-after-merge
description: remove every agent worktree, its branches and its build folder as soon as its PR merges, in every FerroHEALTH repository
metadata:
  type: feedback
---

Remove a worktree and its local branches as soon as the PR carrying its work
merges, and run `git fetch --prune`. A worktree whose work was discarded or
superseded goes too. Check `git worktree list` at every merge notice. Owner:
worktrees "never hang there"; "always remove old worktrees, never forget".

**Why:** stale worktrees and their `target/` folders fill the shared disk.

**How to apply:** on each merge, `git worktree remove <path>`,
`git branch -D <branch>`, `git fetch --prune`. Before removing, check the
worktree has no uncommitted or unpushed work; a dirty one is reported, never
deleted. Run `cargo clean` in a worktree that cannot be removed yet, and in a
main checkout's `target/` once no build uses it. Every worker brief ends with
"remove your worktree once your PR merges". Worktrees live under
`.claude/worktrees/<name>` ([[worktrees-for-parallel-branches]]).
