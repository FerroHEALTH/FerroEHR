---
name: merge-queue-signed
description: main takes only signed commits through pull requests and the merge queue gated on conclusion; arm with gh pr merge --auto, fix a conflict by a local signed merge, never update-branch --rebase or --admin; no commit-message line may start with "-" or a git header word (FerroEHR #5233, #5245, 2026-11-09)
metadata:
  type: feedback
---

The `main` ruleset requires signed commits, a pull request, and the merge queue gated on the `conclusion` check, with no strict up-to-date rule.

**Why:** a server-side rebase (`gh pr update-branch --rebase`) drops the author's signature and the ruleset then refuses the branch; the queue tests pull requests on top of `main`, so a branch never has to be brought up to date by hand, and several land per CI run.

**How to apply:** arm a pull request with `gh pr merge <n> --auto` (the queue sets the method; `--squash` and `--delete-branch` are refused). Fix a conflicting branch with a local merge of `origin/main`, signed by `commit.gpgsign`, and a plain push; no force-push. Dependabot branches get `@dependabot rebase`. Never `--admin` ([[admin-bypass-stays]]).

**Commit-message lines (2026-11-09):** no line of a commit message may start with `-` or with a git header word (`tree`, `parent`, `author`, `committer`, `gpgsig`). The queue's squash commit then fails its signature check and the queue removes the PR with `branch_protection_failure` after a green run (FerroEHR #5233 and #5245). Write the body as prose sentences, not a dashed list.
