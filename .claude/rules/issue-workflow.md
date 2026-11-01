---
paths: ["scripts/gh/**", ".github/ISSUE_TEMPLATE/**"]
---

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
  `no-chart-bump`, `no-ui-visual-change`.

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

## Upstream reports, spec-version triage, CI labels and milestones

the **type** is the native issue type (`Bug`↔fix, `Feature`↔feat, `Task`,
which carries exactly ONE work-kind label —
`documentation`/`chore`/`refactor`/`perf`/`test`/`ci` — naming its commit
type), the **priority** is the organisation's `Priority` issue field
(`Urgent` drop everything / `High` current focus / `Medium` normal / `Low`
backlog) and the **effort** its `Effort` field (`Low` one sitting / `Medium`
one PR across crates or with a fixture / `High` more than one PR or a held
design; it never reorders the worklist). All three are set with
`scripts/gh/fields.sh`; the `bug`/`enhancement`/`P0`–`P3` labels are retired
and `scripts/gh/labels.sh` deletes them. **Domain/area** labels:
`spec:RM…CNF`, `spec-update`, `spec-impact:*` (triage adds exactly one),
`viewer` (the FerroEHR Viewer, its own OCI image), and `upstream-report`
(dark red — an OUTBOUND report of a defect/contradiction/silence in the
released openEHR specs; owner ruling 2026-08-01, replacing the deleted
`docs/conformance/upstream-reports.md` ledger). An `upstream-report` issue IS
the report: it opens with a plain summary, then `## What the released spec
says` (citations), `## What this implementation does`, `## Resolution sought
upstream` — never ticket-draft framing. Unverified reports sit in the
verification milestone with an acceptance checklist; **verification is
TERMINAL (owner ruling 2026-08-21, the full lifecycle in
`.claude/rules/cnf-triage.md`)**: a report re-verified first-hand as genuine
gains `upstream-confirmed` (amber; NOT `blocked-upstream`, which keeps its
narrower meaning: resolved in Jira, normative text not yet published) and —
once its divergence is fully adjudicated in-repo — CLOSES as the standing
outbound record (the closed issue stays the target of every citation of it; a
confirmed report stays open only while something in-repo is blocked on it via
a native edge); a refuted one is closed and every citation of it removed or
re-grounded (the workflow made gating). The machine layer is the conformance
instrument's catalogue: an option in `catalogue/capabilities.yaml` or a
`report:` on a citation points at the issue, never the other way only. **Spec-version
triage** (on `spec-update` issues): `spec-version:current` = fix inside a
pinned line, act immediately; `spec-version:next` = lands in a different
upstream release, collected under an on-demand `upstream:<comp>-<ver>` label
(adoption per the `docs/VERSIONS.md` §Spec version policy). **PR-flow
labels** (CI escape hatches, on PRs not issues): `no-changelog`
(changelog-guard; genuinely invisible changes only), `no-ui-visual-change`
(ui-screenshot-guard; viewer source change with zero visual effect — see
`.claude/rules/console-ui.md` §5), `no-crate-bump` (crate-version-guard; a
PR that sets the `openehr-*` lockstep version by hand, which the publish step
otherwise owns — see `.claude/rules/crates-publishing.md`). These guards read labels from the PR event
payload; `ci.yml` listens for the `labeled`/`unlabeled` types (#2777), so
applying a label raises a fresh run with the current label set by itself — a
RE-RUN of the failed job still re-uses its stale payload, so let the new run
report instead. A label referenced by CI must exist in the repo (`gh label
create`) — a missing label fails silently at apply time, not in the workflow.
**Milestones = releases** (vX.Y.Z): a milestone is a delivery promise, and
the tracker works one milestone at a time (owner, 2026-10-19). A
`blocked-upstream` issue keeps its label and carries the milestone where its
work lands once unblocked, by default the next milestone after the current
one, with a comment naming what it waits on; when the wait ends it moves to
the current milestone (the watcher's auto-unblock note says so). An
`upstream-report` follows `.claude/rules/cnf-triage.md`. A release is cut
when its milestone reaches zero open issues (procedure in
`.claude/rules/changelog.md` — changelog rename, version bumps + goldens,
release PR, tag on the merge commit, ensure the next one exists). The release
workflow enforces both ends: its `plan` job refuses a tag when no milestone is
titled exactly as the tag or that milestone still holds an open issue or pull
request, naming each one, and its `milestone` job closes it once every
leg, crates.io included, has succeeded (`scripts/release/milestone.sh`).
After a manual or recovery release, check that the milestone is closed.
Issues + git survive `/clear` and `/compact`; the built-in todo tool
is session-scoped, so the tracker is the durable layer.
