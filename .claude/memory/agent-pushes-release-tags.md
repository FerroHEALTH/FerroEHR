---
name: agent-pushes-release-tags
description: Owner 2026-10-08 and 2026-10-28, the agent merges the release PR through the queue, creates and pushes the signed release tags (and runs the cut steps); the owner never does
metadata:
  type: feedback
---

Create and push release tags myself: `git tag -s vX.Y.Z <commit> -m "vX.Y.Z"`, verify with `git tag -v`, `git push origin vX.Y.Z`. Local GPG signing is configured (`user.signingkey`, `tag.gpgsign = true`), so the tag is signed as the release lane expects. Then watch the release run, check that its `milestone` job closed the milestone ([[release-closes-milestone]]), and clean up the release branch.

**Why:** the owner, 2026-10-08: "you push everything stupid i do not do anything?" and "i never push the tags and everything you always do that", after being told to push the v4.3.5 tag himself.

**How to apply:** never hand the tag, the push, the milestone close or branch cleanup back to the owner. Nothing stays the owner's: the release PR is armed with `gh pr merge <n> --auto` and lands through the merge queue like any PR (owner 2026-10-28: "you merge it for me as well do not wait for me"). The agent approves the crates.io environment when the release run pauses there (`gh api …/pending_deployments`), after checking what the leg will publish (a no-op when no packaged crate content changed). Publishing the draft security advisories when a release is published is done by the agent too. See [[release-pr-chart-and-third-party-images]], [[push-pr-fixes-without-asking]].
