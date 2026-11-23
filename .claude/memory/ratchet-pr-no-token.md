---
name: ratchet-pr-no-token
description: FerroEHR's ratchet-tighten lane runs without a PAT, so its PR starts no CI; close and reopen it and arm auto-merge (owner, 2026-11-02)
metadata:
  type: feedback
---

`ratchet-tighten.yml` opens its `chore/ratchet-tighten` pull request with
GITHUB_TOKEN, so the PR gets no checks. The owner decided to add no
`RATCHET_TIGHTEN_TOKEN` or `TOOLCHAIN_BUMP_TOKEN` secret.

**Why:** a close and reopen with the owner's `gh` login is a user event, so
CI starts; no new credential is needed.

**How to apply:** never ask the owner for the token. When the ratchet PR is
open with no checks, close it, reopen it and arm auto-merge. When the branch
is ahead of main with no PR (the repository may forbid Actions to open one),
open one on it with the `no-changelog` label and arm auto-merge. The squash
keeps the github-actions co-author trailer, which the owner allows.
