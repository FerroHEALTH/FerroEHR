# The tracker is GitHub Issues

**The open GitHub issue list IS the worklist** — there is no in-repo tracker
table. This file is a delete-protected pointer.

- **Orient**: `gh issue list --state open` (the SessionStart hook injects
  it; pinned issues = current focus).
- **Work an item**: `gh issue view <n> --json title,body,comments` for the
  contract and the running discussion.
- **Record**: tick acceptance-criteria checkboxes (`gh issue edit`), post
  status comments (`gh issue comment`); PRs declare `Closes #<n>` — the
  merge into main auto-closes the issue.
- **Deep plans** live here in `docs/plans/*.md`, linked from their issue,
  deleted in the PR that implements them (`docs/plans/README.md`).

The full workflow and milestone rules: root `CLAUDE.md` §Issue workflow;
the issue type, the `Priority` and `Effort` fields and the labels:
`.claude/rules/issue-workflow.md`. The build
narrative is the closed issues + PR descriptions + `CHANGELOG.md`.
