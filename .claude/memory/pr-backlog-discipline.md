---
name: pr-backlog-discipline
description: Several open PRs are fine when each merges cleanly; parallel PRs touch disjoint files, nothing is stacked on an unmerged branch, and every open PR is tracked to its merge (owner 2026-10-17)
metadata:
  type: feedback
---

On 2026-10-17 four workers in FerroBRIDGE, each told to open "one PR per issue", produced 21 open pull requests in a few hours. Several were stacked on unmerged branches; when a base merged squashed, its stack conflicted everywhere, one squash carried three other PRs' changes, and two PRs that rewrote the same startup code collided. The owner: "you went to fast and didn't checked what you already had done and kept track of all the PR's ... now we have an whole migration hell", and "you can of course have more than one open PR ... but you need to make sure that they merge correctly".

**Why:** throughput is the merged count, not the opened count. Every overlap costs a rebuild.

**How to apply:**
- Parallel PRs touch disjoint files. Two issues in the same module go to one worker, in sequence.
- No stacked PRs: never branch a PR from another open PR's branch; wait for the base to merge, then branch from `origin/main`.
- Verify each PR as it opens: `gh pr view <n> --json closingIssuesReferences` names exactly its issues, auto-merge or the queue holds it, and it is not a draft.
- When PRs sit conflicting or red, fix them before starting new work ([[workers-own-every-open-pr]]). The merge queue removes the up-to-date churn but not the overlap.
