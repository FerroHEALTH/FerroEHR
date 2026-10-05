# Issue fields: type, priority, effort and labels

The tracker is GitHub Issues (`CLAUDE.md` §Issue workflow is the loop). This
file is the policy for what an issue carries and the one sanctioned command
surface for setting it, `scripts/gh/fields.sh`. Relationships between issues
are `issue-relationships.md`; the public board is `project-board.md`.

## Type, priority and effort are GitHub's own fields, never labels

Owner decision 2026-10-02 (#3524), the model FerroFED adopted the same day.
The FerroHEALTH organisation defines all three:

| Fact | Where it lives | Values | Command |
|---|---|---|---|
| Type | the organisation's native issue type | `Bug`, `Feature`, `Task` | `scripts/gh/fields.sh type <n> bug\|feature\|task` |
| Priority | the organisation's `Priority` issue field | `Urgent`, `High`, `Medium`, `Low` | `scripts/gh/fields.sh priority <n> urgent\|high\|medium\|low` |
| Effort | the organisation's `Effort` issue field | `High`, `Medium`, `Low` | `scripts/gh/fields.sh effort <n> high\|medium\|low` |

`gh` sets none of the issue fields, so `fields.sh` goes through the
`updateIssueIssueType` and `setIssueFieldValue` GraphQL mutations, resolving
every node id from a name and failing loud on a typo.
`scripts/gh/fields.sh show <n>` prints all three with the labels and the
milestone. **Every issue is filed through `scripts/gh/fields.sh new <type>
<priority> <effort> <gh issue create args…>`**, never a bare `gh issue
create`, so it carries all three from its first second.

- **Priority:** `Urgent` is drop everything (build broken, data integrity,
  security), `High` the current focus, `Medium` the normal course of work,
  `Low` the backlog. Among candidates that are not pinned or named, the
  priority decides the order and the oldest issue goes first within it.
- **Effort** is the size of the work as filed: `Low` is one sitting (a
  comment, a guard, a one-file fix), `Medium` is one pull request across more
  than one crate or with a test fixture, `High` is more than one pull request
  or a design the orchestrator holds in context. It is set at filing and
  re-set when the work turns out bigger. It never reorders the worklist.
- **Type → commit type:** a `Bug` is a `fix`, a `Feature` is a `feat`, and a
  `Task` names its commit type with exactly ONE work-kind label. A `Bug` and a
  `Feature` carry no work-kind label. An `upstream-report` is a `Bug`; a
  `spec-update` triage issue is a `Task` with `chore`.
- The organisation's `Start date` and `Target date` issue fields stay empty:
  the milestone is the release spine and its due date the target (the board's
  `Target date` is the derived mirror `project-board.md` describes).

## Issues a scheduled lane files

A workflow's `GITHUB_TOKEN` cannot read the organisation's issue fields
(GraphQL answers `organization: null`, not an error), so no lane sets a
priority or an effort. The lanes set the type through the REST issue update
after the create (`file-watcher-issue`'s `type` input, `hosted-watch.yml`),
and a refused type is a warning, never a lost finding. **Whoever picks up such
an issue sets the missing type, priority and effort before anything else**;
the SessionStart dump and `/phase-status` show the gap as `<?/?>`.

## Reading an issue

Never `gh issue view <n> --comments`: on gh 2.101.0 it prints nothing and
exits 0 for an issue with no comments (verified on #3521), so the contract
comes back empty and nothing says so. Read with:

```sh
gh issue view <n> --json title,body,comments \
  --jq '.title, .body, (.comments[] | "--- comment ---", .body)'
```

## Labels: what the platform has no field for

`scripts/gh/labels.sh` declares them (idempotent, `--force`) and deletes the
six retired ones (`bug`, `enhancement`, `P0`–`P3`) wherever they reappear.

- **Work kind, on a `Task` only, exactly one:** `documentation`, `chore`,
  `refactor`, `perf`, `test`, `ci`.
- **Domain:** `spec:<component>` (BASE, RM, AM, LANG, QUERY, TERM, SM, CNF,
  ITS, ITS-XML, ITS-JSON, ITS-BMM, ITS-REST), `viewer`, `fhir`, `regulation`.
- **Spec-update triage:** `spec-update`, exactly one `spec-impact:*`,
  `spec-version:current`/`next`, the on-demand `upstream:<comp>-<ver>`, and
  `blocked-upstream` (resolved in Jira, normative text not yet published).
- **Outbound:** `upstream-report` and `upstream-confirmed` (lifecycle in
  `cnf-triage.md`).
- **Workflow:** `on-hold`, `dependencies` (Dependabot).
- **Pull-request escape hatches** CI reads: `no-changelog`, `no-crate-bump`,
  `no-chart-bump`, `no-ui-visual-change`, `no-conformance-run`.

A new label goes into `labels.sh` in the same change that first uses it.

## The migration (2026-10-02)

`scripts/gh/migrate-fields.sh` (`plan`, `apply`, `verify`, `--self-test`)
moved every OPEN issue off the retired labels (`bug`→Bug,
`enhancement`→Feature, else Task; `P0`→Urgent … `P3`→Low), gave each its
judged effort and every Task its work-kind label; `verify` reported zero
outstanding changes before `labels.sh` deleted the six labels. Closed issues
were left as they were (owner decision), so they carry no type or priority.

## Official documentation (durable citations)

- Issue types — https://docs.github.com/en/issues/tracking-your-work-with-issues/configuring-issues/managing-issue-types-in-an-organization
- Issue fields — https://docs.github.com/en/issues/tracking-your-work-with-issues/configuring-issues/managing-issue-fields-in-an-organization
- GraphQL mutations — https://docs.github.com/en/graphql/reference/mutations
- REST issues (`type` on create/update) — https://docs.github.com/en/rest/issues/issues
- Issue forms (`type:` key) — https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms
