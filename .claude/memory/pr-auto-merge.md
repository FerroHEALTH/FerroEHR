---
name: pr-auto-merge
description: Arm every pull request with gh pr merge <n> --auto the moment it opens, as its own command, which queues it in the merge queue; never --squash, --delete-branch or --admin; watch it until merged (owner 2026-09-04)
metadata:
  type: feedback
---

After opening a pull request, arm it at once, as its own command:

```sh
gh pr merge <n> --auto
```

`main` merges through GitHub's merge queue, which sets the merge method itself and refuses `--squash` and `--delete-branch`; the repository deletes a merged head branch on its own.

**Why:** the owner, 2026-09-04: "for PR's do not forget to trigger auto merge okay!! so when the CI is green it will be merged". The `main` ruleset requires the `conclusion` check, so auto-merge is the hand-off.

**How to apply:** never leave a pull request waiting for a manual merge, and watch it until it merges (`gh pr view <n> --json state,mergedAt`); arming does not finish the job ([[workers-own-every-open-pr]]). Branch from `origin/main` in a worktree ([[worktrees-for-parallel-branches]]), stage named paths, never `git add -A` from the root after a build. Never `--admin` ([[admin-bypass-stays]]).
