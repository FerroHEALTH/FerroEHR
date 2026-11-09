---
name: concurrent-sessions-shared-tree
description: "The user runs multiple Claude sessions in the same working tree — commit explicit paths only, scope build/test gates, expect branch switches under you"
metadata: 
  node_type: memory
  type: project
  originSessionId: 3d6bd35c-d7ca-4629-bfa9-12bad4751500
  modified: 2026-08-06T21:53:41.348Z
---

Observed 2026-07-08: while one session worked, another session in the same
clone committed (`conformance:` commits), switched the branch
(claude/s2-access-control → claude/cnf-hardening), and its `git add` swept the
first session's mid-edit files into its own commit.

**Why:** the user deliberately runs parallel Claude sessions on one checkout;
`git add -A`/branch state is shared and races.

**How to apply:**
- Never `git add -A`/`git commit -a` — stage explicit paths only, and re-check
  `git branch --show-current` immediately before committing.
- **Explicit `git add <paths>` is NOT enough** (bitten 3× on 2026-07-14, again
  2026-08-06): another session/agent may have pre-staged its own files, and a
  plain `git commit` sweeps the whole index. The 2026-08-06 case was a
  **deletion**: an agent ran `git rm` in its own fence, which stages, and two
  `deploy/` files it was mid-way through removing rode into an unrelated
  `scripts/` commit. Commit with an explicit pathspec —
  `git commit -F msg -- <paths>` — which commits ONLY those paths regardless
  of what else sits staged; or run `git diff --cached --name-only` first (it
  shows staged deletions too) and unstage strangers. Recovery is
  `git reset --soft HEAD~1` + `git restore --staged <stranger>` + recommit,
  which leaves the agent's work untouched in the tree.
- Scope cargo gates to the crates you touched (`-p`), not `--workspace` — the
  other session may have a broken crate in flight.
- For multi-file subagent work, use worktree isolation and merge branches back.
- **Parallel implementation agents MUST get `isolation: "worktree"` on the
  Agent call — never two agents in the main tree** (bitten 2026-07-20: two
  agents told to "create a new branch" in one checkout fought over
  HEAD/working files; one stashed the other's WIP incl. an untracked module
  that survived only in `stash@{N}^3`, and the tree ended half-mixed).
  Worktree agents build into their own worktree target (owner delegation
  2026-11-09, [[worktrees-for-parallel-branches]]); a shared target lets one
  worktree overwrite another's workspace-member outputs. Recovery pattern if it ever happens
  again: check `git stash list` FIRST (agents stash each other's work),
  including each stash's `^3` untracked-files parent.
- Don't "fix" broken files you didn't touch (e.g. a half-edited test file) —
  they're the other session's work in progress.

**ONE `./target` for the main checkout (owner ruling 2026-07-16):** the CLI,
subagents working in the main checkout, AND the IDE share the single default
`./target`; a worktree builds into its own target instead (2026-11-09,
[[worktrees-for-parallel-branches]]). Every extra target dir is a full
duplicate build tree — per-agent lanes and an IDE-specific dir are banned
(they once filled 394 GB between them). Lock waiting is expected and never
answered with a second target dir; subagents never run cargo in parallel
(the orchestrator builds once at convergence); check `du -sh target` at
session start and after any rewrite-scale change, `cargo clean` above
~30 GB or above 200k entries in `deps/` (#5248). Never pkill -9 rustc to "fix" slowness — it corrupts incremental
caches. Full discipline in CLAUDE.md §"Target-dir & warm-build discipline".

**Never `git stash` the shared tree while a worker is live (2026-08-24 incident).**
A stash scoops EVERY tracked modification, including a running subagent's
uncommitted edits — the worker then continues onto a silently cleaned tree and
its Edit-tool old_strings stop matching. If main is needed mid-worker:
commit the orchestrator's own files first, or branch-switch with the changes
carried, or restore later by EXPLICIT PATH (`git checkout stash@{0} -- <paths>`)
— never `stash pop` onto a tree the worker kept writing to. Back up the stash
as a patch before dropping it.
