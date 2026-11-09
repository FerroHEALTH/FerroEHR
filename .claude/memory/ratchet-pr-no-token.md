---
name: ratchet-pr-no-token
description: FerroEHR's daily ratchet-tighten PR runs without a PAT; the orchestrator closes and reopens it so CI starts
metadata:
  type: feedback
---

FerroEHR's `ratchet-tighten.yml` opens its daily `chore/ratchet-tighten` pull request with GITHUB_TOKEN, so the PR gets no CI. The owner decided on 2026-11-02 to add no `RATCHET_TIGHTEN_TOKEN` or `TOOLCHAIN_BUMP_TOKEN` secret.

**Why:** the owner did not see the need for a new credential. A close and reopen with the owner's `gh` login is a user event, so CI starts and auto-merge takes the PR in.

**How to apply:**
- Do not ask the owner for the token again.
- The repository also forbids GitHub Actions from creating pull requests (found 2026-11-02), so the lane only pushes `chore/ratchet-tighten` and ends with a notice (#4797). When that branch is ahead of main with no PR for its head, open the PR on it with the `no-changelog` label and arm auto-merge; the healthwatch watcher does this. The squash then carries a github-actions bot co-author trailer, which the owner allows (2026-11-02, like Dependabot's); FerroEHR #4815 taught the attribution guard to keep it.
- When the ratchet PR is open with no checks, close it, reopen it and arm auto-merge.

Related: [[workers-own-every-open-pr]].
