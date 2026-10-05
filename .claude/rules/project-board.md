---
paths: ["scripts/gh/**"]
---

# The public roadmap board (GitHub Project v2)

The tracker is GitHub Issues (`CLAUDE.md` §Issue workflow); milestones are the
release spine; the native issue type and the organisation's `Priority` and
`Effort` issue fields carry type, priority and effort (`issue-workflow.md`);
labels carry work kind and domain; native edges carry decomposition and
sequencing (`issue-relationships.md`). The **"FerroEHR Roadmap"
Project** (a GitHub Project v2 under the repo owner, public) exists for one
reason: **outward transparency** — anyone can see what is planned, in
progress, and shipped, without reading the raw issue list. It is a **VIEW over
the tracker, never a second tracker.** This file is the policy (what the board
may and may not carry) and the canonical commands (the one sanctioned write
path is `scripts/gh/project.sh`).

## The one-datum rule

**Status (`Todo` / `In Progress` / `Done`) is the ONLY board-managed datum.**
Everything else the board displays is read straight from the issue and already
has a canonical home:

| Fact | Canonical home | NEVER duplicated as |
|---|---|---|
| Priority | the organisation's `Priority` issue field (`scripts/gh/fields.sh`) | a board Priority field |
| Type | the native issue type (`Bug`/`Feature`/`Task`) | a board Type field |
| Effort | the organisation's `Effort` issue field (`scripts/gh/fields.sh`) | a board Effort/estimate field |
| Release | the `vX.Y.Z` milestone | a board Release/Iteration field |
| Decomposition | native sub-issue edges | a board hierarchy field |
| Sequencing | native blocked-by edges | a board Blocked column/field |

Do not add custom fields, iteration fields, estimate fields, or extra Status
options. A board-only fact has no backlink, is invisible to `gh issue`
consumers (the SessionStart dump, `/phase-status`, `/next-task`), and rots the
first time it disagrees with the field, label or milestone it shadows — the
same decay class `issue-relationships.md` §No duplication bans for issue
bodies. If the board ever needs to show a new fact, give the fact a canonical
home on the ISSUE (issue field, label, milestone, native edge) and let the
board filter/group on it.

**The ONE sanctioned derived field: `Target date`.** The roadmap layout
places items only by date/iteration fields — milestone due dates draw
timeline markers, never item bars (roadmap-layout docs, read 2026-08-04) — so
`Target date` exists as a machine-derived mirror of the item's milestone due
date. It is written ONLY by `scripts/gh/project.sh sync-dates` (re-run after
changing a milestone due date or re-milestoning issues; the release
procedure's cut steps re-run it), never by hand — a hand-set date is the
duplication this file bans.

**No fourth Status, no manual "Blocked/Stalled" column (adjudicated
2026-08-04).** Blocked-ness already has an automatic canonical home: native
`blocked-by` edges (GitHub renders the red "Blocked" badge on those cards in
Projects by itself, and clears it the moment the blocker closes) and the
`blocked-upstream`/`upstream-confirmed` labels. A hand-moved status would
double-book that and keep claiming "stalled" after the blocker closes.
"Needs extra attention" is served by the **Needs attention** view (filter
`is:open label:blocked-upstream,upstream-confirmed`) — label-driven, so it
empties itself; `Urgent` work shows in **Current focus**. (Projects filters expose no `is:blocked` qualifier —
filtering-projects docs, read 2026-08-04 — which is exactly why the
label/edge layer stays the source.)

## Status semantics + who moves it

- **`Todo`** — every open issue starts here (the auto-add workflow sets it).
- **`In Progress`** — set at pickup, when work on the issue actually starts
  in a session: `scripts/gh/project.sh status <n> in-progress`. `/next-task`
  does this as its final step. This is the ONE manual move in the lifecycle —
  GitHub has no built-in "branch/PR opened → In Progress" workflow.
- **`Done`** — never set by hand. The issue closes via the PR's `Closes #N`
  and the built-in "item closed → Done" workflow moves it. A reopened issue
  goes back to `Todo` automatically.

An issue abandoned mid-flight (session ended, work parked) goes back to
`todo` explicitly — a stale `In Progress` column is a false public claim.

## The one sanctioned command surface — `scripts/gh/project.sh`

Projects v2 writes (`gh project item-edit`) take four opaque GraphQL node ids
(project, field, option, item) — never the issue `#number`. The helper
resolves them all from the `#number` and fails loud (the `gh-rel.sh` pattern).
Requires the `project` token scope (`gh auth refresh -s project`).

| Intent | Command |
|---|---|
| Start work on #n | `scripts/gh/project.sh status <n> in-progress` |
| Park #n (work stopped, not done) | `scripts/gh/project.sh status <n> todo` |
| Put #n on the board (auto-add missed it) | `scripts/gh/project.sh add <n>` |
| Move #n to another repository (drops its card in the same step) | `scripts/gh/project.sh transfer <n> <owner/repo>` |
| Read #n's board status | `scripts/gh/project.sh show <n>` |
| Print the whole board by column | `scripts/gh/project.sh board` |
| Print the project URL | `scripts/gh/project.sh url` |
| Post a status update | `scripts/gh/project.sh update <on-track\|at-risk\|off-track\|complete\|inactive> "<markdown>" [--start YYYY-MM-DD] [--target YYYY-MM-DD]` |
| Read recent status updates | `scripts/gh/project.sh updates` |

Never move `Done` by hand, never `gh project item-edit` raw, and never
`item-archive`/`item-delete` — closed items stay visible as the shipped
record (the built-in auto-archive workflow stays OFF). The ONE deletion the
helper performs belongs to a transfer: GitHub moves a project item along with
an issue transferred to another repository, so the board would otherwise show
a foreign issue as ours (found 2026-09-03 after #2646 and #2652 moved to
FerroBRIDGE). An issue therefore leaves this repository ONLY through
`scripts/gh/project.sh transfer <n> <owner/repo>`, which moves it and drops its
card in one step; `project.sh transferred <owner/repo>#<n>` repairs a transfer
someone ran raw. Never run `gh issue transfer` by hand.

## Status updates (the board's progress narrative)

GitHub project **status updates** (shown in the board header + side panel;
`createProjectV2StatusUpdate` — verified live in the GraphQL schema
2026-08-04, the concept docs describe the UI only) are the outward progress
narrative. Post one via `scripts/gh/project.sh update …`:

- **At every release cut** (part of the release procedure,
  `.claude/rules/changelog.md`): status `on-track` (or the honest
  alternative), a short markdown summary of what the release shipped and
  what the next milestone targets, `--target` = the next milestone's due
  date **only if that milestone actually has one** — never invent a date.
- **When direction genuinely shifts** (a milestone re-scoped, a program
  re-prioritized): post the change with the reason.
- Write for the public reader: no internal codenames, phase markers, or
  repo-internal file paths; numbers only when they come from committed
  artifacts. The same honesty rules as everything published: `at-risk`/
  `off-track` are used when true — the board never claims what the tracker
  does not show.

## Board configuration (the intent, for anyone recreating it)

Fields: the built-in `Status` with exactly `Todo` / `In Progress` / `Done`.
Views — name/layout/filter ARE scriptable
(`createProjectV2View`/`updateProjectV2View`, `layout` ∈ `BOARD_LAYOUT`/
`TABLE_LAYOUT`/`ROADMAP_LAYOUT` — verified live in the GraphQL schema
2026-08-04; the concept docs don't mention them. Fine view configuration —
grouping, the roadmap's date source, visible fields — is still UI-only):

1. **Board** — `BOARD_LAYOUT`, grouped by Status, **slice by Milestone**
   (one sidebar click = a single release's kanban, without hardcoding a
   milestone into the filter where it would go stale at every cut); the
   "what is going on right now" surface.
2. **Roadmap** — `ROADMAP_LAYOUT`, filter `is:open`; items placed by the
   derived **`Target date`** field (Date fields → Target date in the UI;
   kept true by `sync-dates`); **group by Milestone**, **slice by Labels**,
   milestone markers ON, zoom Month, sort by Target date. Every open
   `vX.Y.Z` milestone carries a due date — set one when creating a
   milestone; the markers come from it.
3. **Current focus** — `TABLE_LAYOUT`, filter `is:open
   priority:Urgent,High`; columns Title/Status/Type/Priority/Labels/
   Milestone/Sub-issues progress; no slice. The filter reads the
   organisation's `Priority` issue field, which the project shows only once
   the field is added to it in the UI (`ProjectV2FieldType` has `ISSUE_TYPE`
   but no issue-field type, so no mutation adds it; read 2026-10-02). Until
   then the view is empty, while `type:` filters work already.
4. **Needs attention** — `TABLE_LAYOUT`, filter
   `is:open label:blocked-upstream,upstream-confirmed`; same columns;
   no slice.

Built-in workflows (verified 2026-08-04: only `deleteProjectV2Workflow`
exists in the schema — **enabling/configuring them is UI-only**; visibility +
repo-link ARE scriptable via `gh project edit --visibility` /
`gh project link`). The full adjudicated switchboard:

| Workflow | State | Why |
|---|---|---|
| Auto-add to project | ON, filter `is:issue is:open` | issues only — every PR declares `Closes #N`, so a PR card would show the same work twice |
| Auto-add sub-issues to project | ON | sub-issues are real issues |
| Item added to project | ON → `Todo` (issues only) | every new open issue starts in Todo |
| Item reopened | ON → `Todo` | a reopened issue re-enters the working columns |
| Item closed | ON → `Done` | the tracker drives the board |
| Pull request merged | ON → `Done` | harmless (PRs never land on the board) |
| Pull request linked to issue | ON → `In Progress` (when offered) | auto-pickup safety net: a PR referencing the issue proves work started |
| **Auto-close issue** | **OFF** | it closes the REAL issue when a card is dragged to Done — the board is a view and must never mutate the tracker; closing happens only via the PR's `Closes #N` |
| Auto-archive items | OFF | closed items stay visible as the shipped record |
| Code changes requested / Code review approved | OFF | PR-status workflows; PRs aren't on the board |

(An Actions-based alternative — `actions/add-to-project` — exists but needs a
PAT secret for a user-owned project; one-time UI toggles beat a standing
secret.) Visibility: public.

## Interaction with the rest of the workflow

- **`/next-task`** moves the picked issue to `In Progress` once the plan is
  accepted and work starts.
- **`/phase-done`** verifies the closing issue lands in `Done` (the merge +
  workflow do it; the skill only checks).
- **`/phase-status`** may cite `scripts/gh/project.sh board` for the public
  view, but the issue list stays the working ground truth.
- The **board readme** (`gh project edit --readme`) is the ONE home of the
  durable direction themes (owner ruling 2026-08-04, issue #1867): never
  create a roadmap markdown file, and
  keep the readme themes-only (no item-level state, no quotable numbers —
  those live in issues and the committed artifacts). `README.md` links the
  board for visitors.

## Official documentation (durable citations)

- gh project commands — https://cli.github.com/manual/gh_project
- Projects v2 API — https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-api-to-manage-projects
- Built-in workflows — https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-built-in-automations
- Roadmap layout — https://docs.github.com/en/issues/planning-and-tracking-with-projects/customizing-views-in-your-project/customizing-the-roadmap-layout
