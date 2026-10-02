# Issue relationships (GitHub native sub-issues + dependencies)

The tracker is GitHub Issues (`CLAUDE.md` §Issue workflow). GitHub exposes four
native issue relationships; we use them as first-class tracker structure rather
than describing structure in prose. This file is the **policy** (when to use
each) and the **canonical commands** (how, with zero guessing). It is
machine-adjacent: the one sanctioned write path is `scripts/gh/rel.sh`.

## Two facts that dictate everything below

1. **`gh` has no native subcommand** for sub-issues or dependencies (verified,
   gh 2.88.1). Every relationship goes through `gh api` — or, preferably,
   `scripts/gh/rel.sh`, which wraps the correct endpoints.
2. **Write endpoints take the issue's database `id`, not its `#number`.** The
   sub-issue/dependency POST/DELETE bodies want `sub_issue_id` / `issue_id` =
   the numeric database id (e.g. `4946992792`), which is NOT the `#231` you see
   in the UI. `scripts/gh/rel.sh` resolves `#number → id` for you and fails loud
   on a bad number, so **always prefer it over raw `gh api` for writes.** Reads
   are `#number`-keyed and need no resolution.

## The one sanctioned command surface — `scripts/gh/rel.sh`

| Intent | Command |
|---|---|
| Make #child a sub-issue of #parent | `scripts/gh/rel.sh parent <child> <parent>` |
| Move #child to a new parent (it already has one) | `scripts/gh/rel.sh parent <child> <parent> --replace` |
| Detach #child from its parent | `scripts/gh/rel.sh unparent <child>` |
| #n is blocked by #blocker | `scripts/gh/rel.sh blocked-by <n> <blocker>` |
| Remove "#n blocked-by #blocker" | `scripts/gh/rel.sh unblock <n> <blocker>` |
| #n blocks #blocked | `scripts/gh/rel.sh blocking <n> <blocked>` |
| Remove "#n blocking #blocked" | `scripts/gh/rel.sh unblocking <n> <blocked>` |
| Show every relationship of #n | `scripts/gh/rel.sh tree <n>` |
| Print the database id of #n | `scripts/gh/rel.sh id <n>` |

Each write prints a one-line `ok: …` confirmation. Run `scripts/gh/rel.sh` with
no args for the full usage banner.

## The four relationships and when to use each

### 1. Parent / sub-issue — "Add parent" (decomposition)

A parent issue breaks into sub-issues; children roll up into a parent progress
bar (visible in the SessionStart dump, `/phase-status`, and Projects). Limits:
**≤100 sub-issues per parent, ≤8 nesting levels, one parent per issue** (reparent
with `--replace`).

**Use it** to decompose a genuinely multi-part issue into individually
trackable, individually closeable work items — e.g. a "ranked backlog of N
items" issue → one child per item; a "audit every chapter" issue → one child per
chapter. Each child is a real issue with its own contract + acceptance criteria.

**Do NOT** use sub-issues to duplicate release grouping. **Milestones remain the
release spine** (`changelog.md`: a release cuts when its milestone hits zero open
issues) — do not create per-release "epic" parent issues (owner ruling
2026-07-24: that double-books what the milestone already groups and muddies the
cut trigger). Sub-issues express *decomposition*, milestones express *release*.

When new work is discovered en route, its new issue (`scripts/gh/fields.sh
new`, `issue-workflow.md`) is **linked** — as a sub-issue of the issue it
decomposes, or a dependency of the issue it sequences — not left as a prose
"see also".

### 2. Blocked-by — "Mark as blocked by" (sequencing)

#n cannot start/finish until its blockers close. GitHub marks blocked issues
with a "Blocked" badge on the Issues page and Projects. Limit: **≤50 issues per
direction.**

**Use it** for real in-repo sequencing: #A must merge before #B is workable.
`scripts/gh/rel.sh blocked-by B A`.

**Upstream waits** (owner rulings 2026-08-01): `blocked-upstream` keeps its
narrow meaning — resolved in upstream Jira, normative text not yet published
(a **label**, no milestone). A wait on a defect WE reported instead points at
the in-repo `upstream-report` issue with a native `blocked-by` edge (the
report itself carries `upstream-confirmed` once verified). A wait with no
in-repo counterpart stays label-only — you cannot be `blocked_by` a Jira
ticket.

### 3. Blocking — "Mark as blocking" (the mirror direction)

The inverse of blocked-by: #n blocks #other. GitHub stores this as the *other*
issue's `blocked_by` (the only writable direction), so `scripts/gh/rel.sh
blocking n other` posts to #other under the hood — read it back on either side
with `tree`. Use whichever direction reads more naturally at the moment; they
describe the same edge.

### 4. Security alerts — "Add security alert" (UI-only)

Links a **code-scanning alert** to an issue so a security fix shows up in
planning. This is **public preview and UI-only — there is no REST/GraphQL/`gh`
API**, so it cannot be scripted and `scripts/gh/rel.sh` does not cover it.
Prerequisite: code scanning must be enabled (this repo runs CodeQL via
`.github/workflows/codeql.yml`).

**Flow (manual, in the browser):** Security tab → Code scanning → the alert →
**Tracking** → *Create issue* (new) or *Add existing GitHub issue* (link). Or
from the issue's **Relationships** panel → **Security alerts**. Requires write
access. When an alert relates to tracked work, link it so remediation is visible
alongside normal issues; otherwise this relationship stays idle by design.

## No duplication — a relationship lives in exactly ONE place

A relationship is GitHub metadata with its own panel. **Never also write it into
an issue body.** A body copy has no backlink, is not updated when the edge
changes, and rots into a contradiction the first time a child is added, closed,
reparented, or a dependency shifts. This decays fast and silently — it is the
single most likely way this system goes stale. Concretely:

- **A parent's body never lists its sub-issues.** The native **Sub-issues**
  panel and its `{k/n}` progress bar are the single source of truth. Do not
  enumerate child `#numbers` in the body, and do not write a body checklist that
  mirrors the children (`- [ ] #231 …`) — that double-books the progress bar and
  is exactly the duplication that shows the same children twice.
- **An issue's body never lists its blockers or what it blocks.** The
  **Dependencies** panel is canonical.
- **A parent's acceptance criteria are OUTCOMES, not a roll-call of children.** "Every
  sub-issue closed" is already tracked by the progress bar — state the outcome
  the program must reach, and (if useful) point at the panel without naming
  individual children.
- **Prose may name an issue only when it is NOT a native edge** — e.g.
  "supersedes #X", "context in #Y", "adjudicated in #Z (closed)". If the
  reference *is* a parent/child or blocking edge, set the real relationship with
  `scripts/gh/rel.sh` and leave it out of the body entirely.

Review-enforced (there is no CI parser for body prose). The structural safeguard
is that `scripts/gh/rel.sh` only ever touches metadata, never issue bodies — so
the only way an edge lands in a body is someone typing it, which this rule
forbids. When you create a parent issue, its body describes the *program /
contract*; the children are the panel.

## Reading relationships

- `scripts/gh/rel.sh tree <n>` — parent, sub-issues, blocked-by, blocking (all
  with state + title).
- The **SessionStart** dump and **`/phase-status`** surface, per open issue, its
  sub-issue progress (`k/n`), whether it is **blocked** (and by which open
  issues), and what it blocks — computed in one batched GraphQL call
  (`Issue.parent` / `subIssuesSummary` / `blockedBy` / `blocking`). A blocked
  issue is not a candidate for `/next-task` until its blockers close.

## Interaction with the rest of the workflow

- **`/next-task`** skips issues with open blockers (unless the user names one)
  and, for a parent issue, points at the next open child.
- **`/phase-done`** checks the closing issue's sub-issues: do not close a parent
  with open children (finish or re-parent them first); closing a blocker
  unblocks its dependents automatically.
- Relationships live in their native panels, never in issue-body prose (§No
  duplication above) — set the real edge, never restate it in the body.

## Official documentation (durable citations)

- Sub-issues REST API — https://docs.github.com/en/rest/issues/sub-issues
- Issue dependencies REST API — https://docs.github.com/en/rest/issues/issue-dependencies
- Adding sub-issues — https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/adding-sub-issues
- Creating issue dependencies — https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-issue-dependencies
- Linking code-scanning alerts to issues — https://docs.github.com/en/enterprise-cloud@latest/code-security/how-tos/manage-security-alerts/manage-code-scanning-alerts/linking-code-scanning-alerts-to-github-issues
