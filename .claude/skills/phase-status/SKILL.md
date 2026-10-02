---
name: phase-status
description: >
  Prints the tracker's open issues (pinned first), any linked plan file's
  unchecked tasks, and a short git status. Use when the user asks "where are
  we", "what's in flight", or at the start of a work session to orient.
allowed-tools: [Read, Bash]
argument-hint: (none)
---

# /phase-status

A fast orientation dump — step 1 of the issue workflow (`CLAUDE.md`).
Read-only; makes no changes. The live state below is injected at invocation
time — ground the answer in it, not in stale conversation memory.

## Live state (injected)

### The tracker — open GitHub issues (with native relationships)

Each line carries `<Type/Priority>` (the native issue type and the
`Priority` issue field, `.claude/rules/issue-workflow.md`), `{k/n}` sub-issue
progress, `child-of #parent`, and open `BLOCKED-by`/`blocks` edges
(`.claude/rules/issue-relationships.md`). `?` marks a type or priority not
yet set.

```!
gh api graphql -f query='query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { issues(first: 100, states: OPEN, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number title issueType { name } issueFieldValues(first: 10) { nodes { ... on IssueFieldSingleSelectValue { field { ... on IssueFieldSingleSelect { name } } value } } } labels(first: 20) { nodes { name } } milestone { title } parent { number } subIssuesSummary { total completed } blockedBy(first: 30) { nodes { number state } } blocking(first: 30) { nodes { number state } } } } } }' -f owner="$(gh repo view --json owner --jq .owner.login)" -f name="$(gh repo view --json name --jq .name)" --jq '.data.repository.issues.nodes[] | ((.issueType.name // "?") + "/" + ([.issueFieldValues.nodes[] | select(.field.name? == "Priority") | .value] | first // "?")) as $tp | ([.labels.nodes[].name] | join(", ")) as $labels | ([.blockedBy.nodes[] | select(.state == "OPEN") | "#\(.number)"]) as $b | ([.blocking.nodes[] | select(.state == "OPEN") | "#\(.number)"]) as $k | "#\(.number)  \(.title)  <\($tp)>  [\($labels)]" + (if .milestone then "  (\(.milestone.title))" else "" end) + (if .parent then "  child-of #\(.parent.number)" else "" end) + (if .subIssuesSummary.total > 0 then "  {\(.subIssuesSummary.completed)/\(.subIssuesSummary.total)}" else "" end) + (if ($b | length) > 0 then "  BLOCKED-by \($b | join(","))" else "" end) + (if ($k | length) > 0 then "  blocks \($k | join(","))" else "" end)'
```

### Git

```!
cd "${CLAUDE_PROJECT_DIR}" && git status --short | head -40 && echo "---" && git log --oneline -5
```

## Steps

1. Summarize the tracker state: which issue is the current focus (pinned /
   milestone-assigned / the one the branch implements) and what its stated
   next action is. For the in-flight issue, run
   `gh issue view <n> --json title,body,comments` and report the latest
   status comment and
   the unchecked `## Acceptance criteria` boxes, plus `scripts/gh/rel.sh tree <n>`
   for its parent/children/blockers. **Flag any open issue shown `BLOCKED-by`
   an open blocker** — it is stuck until the blocker closes and is not a
   pickup candidate (`.claude/rules/issue-relationships.md`). Flag any open
   issue shown `<?/…>` or `<…/?>`: it is missing its type or priority, which
   whoever picks it up sets first (`scripts/gh/fields.sh`).
2. If that issue links a plan file (`docs/plans/*.md`), **Read it** and list
   every unchecked `- [ ]` line under its Tasks and Acceptance criteria, in
   order, so it is clear how close the work is to done.
3. Summarize the git state from the injected output: current branch work,
   uncommitted files, last commits — flag uncommitted work that looks
   finished (the owner rule: never leave finished work sitting unmerged).
4. If the user asks how things look publicly, `scripts/gh/project.sh board`
   prints the public roadmap board grouped by Status — a presentation VIEW
   over the same issues, never a second source of truth
   (`.claude/rules/project-board.md`); the issue list above stays the
   working ground truth. Flag any `In Progress` item nobody is actually
   working on (it should be parked back to `todo`).
5. **Do not** modify any issue, plan file, or make any commit — this is a
   read-only status check. If the user wants the next task turned into a
   work plan, point them at `/next-task` instead.
