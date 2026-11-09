---
name: worktrees-for-parallel-branches
description: "Worktrees are allowed (owner 2026-10-19) and live only at .claude/worktrees/<name> (owner 2026-10-21); each worktree builds into its own target directory, never a shared one (owner delegation 2026-11-09); a target over 30 GB or a deps/ over 200k entries gets cargo clean (#5248); remove the worktree with its target after the merge"
metadata:
  type: feedback
---

Git worktrees are allowed. Use one whenever branch work runs beside other work in the same clone: a release cut in flight, another worker or session on its own branch, or a fix you push while a PR of yours waits on CI. Every worktree lives at `.claude/worktrees/<name>` (git-ignored), created with `git worktree add .claude/worktrees/<name> -b <type>/<slug> origin/main` from the repository root; never beside the clone and never in the session scratchpad (family rule, owner 2026-10-21, repeated 2026-11-01). Never commit from the main checkout while other work runs there.

**Why:** on 2026-08-06 the owner banned worktrees so the work could be followed in one directory. On 2026-10-19 the owner said worktrees ARE allowed: with release cuts, FerroUI rollouts and fixes running side by side, a shared checkout made workers switch branches under each other.

**Each worktree builds into its own target directory (owner delegation, 2026-11-09).** A shared `CARGO_TARGET_DIR` is unsafe for parallel worktrees: workspace-member outputs carry the same name in every worktree, so one worktree's build overwrites another's, and worker 89 saw one worktree link another worktree's `openehr-its`. Build with the default `<worktree>/target` (or `CARGO_TARGET_DIR=<worktree>/target`), never the main checkout's `./target` or another worktree's. The disk had 1.1 TiB free when this was decided. The one-`./target` rule of `CLAUDE.md` holds for the main checkout and the IDE.

**The clean rule is enforced (FerroEHR #5248).** On macOS the `split-debuginfo = "unpacked"` default leaves 256 stale `.rcgu.o` files per crate per rebuild; 847k entries in `deps/` made the first TLS handshake scan it for 5 s. No profile change: `packed` adds dsymutil time and moves Linux CI to split DWARF, and `off` loses backtrace lines. Instead, a target over 30 GB, or a `deps/` over 200k entries (`find target/debug/deps | wc -l`), gets `cargo clean` before the next build.

**How to apply:**

- Never touch, reset or remove a worktree another worker created; `git worktree list` shows them.
- When the PR merges and the worker has handed back, remove the worktree with its target and its local branch (`cargo clean`, `git worktree remove`, `git branch -D`, `git fetch --prune`), so neither disk nor the IDE keeps stale state. A dirty worktree is reported, never deleted.
- `docs-dist` stays remote-only (2026-09-04): it is the branch the docs pipeline writes; never check it out locally, in a worktree or otherwise.

See also [[concurrent-sessions-shared-tree]].
