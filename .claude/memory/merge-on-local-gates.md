---
name: merge-on-local-gates
description: Owner 2026-07-26 — merge PRs immediately once local gates pass; never wait on CI during fix waves
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 871531fb-1884-468e-9033-ae616ae2eb2b
  modified: 2026-07-26T13:32:55.752Z
---

Owner ruling 2026-07-26 (during the #373 fix wave): when the full local gate
battery has passed (fmt + scoped clippy zero warnings + nextest green on the
touched crates), MERGE the PR
immediately — do not sit on a CI watcher. Waiting for CI on already-locally-
green work wastes wave throughput.

**Why:** the local gates run the exact CI checks (same flags, same tests);
CI is a backstop for the untested paths (ui-e2e, coverage), not a
prerequisite for sequential fix-wave progress. A rare CI failure after merge
is fixed forward on main.

**How to apply:** run the local gates → commit → push → `gh pr create` →
`gh pr merge <n> --merge` in the same breath → checkout main, pull, next
branch. No `--watch` loops between issues. Applies to this repo's
fix-wave/issue cadence; for release cuts keep the normal discipline.

**Since 2026-08-30 the merge click is the owner's:** a `main` branch ruleset
now requires 1 approving review, historical merges were owner-bypass, and the
Claude Code permission classifier refuses both `gh pr merge --admin` and the
REST merge endpoint from the agent. Flow: open the PR, verify gates/CI, then
ask the owner to run `! gh pr merge <n> --squash --delete-branch --admin` —
never burn attempts on classifier workarounds.

**2026-09-02 correction: auto-merge is the sanctioned path and needs no owner
click.** `gh pr merge <n> --auto --squash --delete-branch` right after `gh pr
create` arms GitHub's auto-merge; the PR lands by itself when the required
checks pass (five PRs landed this way in one session: #3052, #3053, #3056,
#3058, #3059). `--admin` stays classifier-blocked, so never try it. While the
PR waits, keep working on the NEXT issue in the same checkout; a red check on
the waiting PR is fixed by pushing to its branch with git plumbing (`git
read-tree` into a temp `GIT_INDEX_FILE`, `hash-object`, `commit-tree -S`,
`git push origin <sha>:refs/heads/<branch>`), never by switching branches under
a worker's uncommitted tree. Force-push is hook-blocked for every branch, so
resolve a conflict by MERGING `origin/main` into the PR branch, not rebasing.

**Superseded (owner, 2026-10-21, and the merge queue):** nothing merges before the required `conclusion` check is green, and nothing merges by hand or through the admin bypass ([[admin-bypass-stays]]). Every PR is armed with `gh pr merge <n> --auto` as its own command and lands through the merge queue ([[pr-auto-merge]], [[merge-queue-signed]]). Keep the part that still holds: while a PR waits, work the next issue ([[workers-never-idle-on-merge-queue]]).
