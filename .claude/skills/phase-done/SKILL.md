---
name: phase-done
description: >
  Closes a tracker issue: verifies the work is genuinely done (exit
  criteria, gates, docs, changelog), writes the close narrative into the PR
  description, ensures the PR declares `Closes #N`, posts the handoff
  comment, and deletes the implemented plan file. Use when the user says a
  work item is complete or asks to close it out.
allowed-tools: [Read, Edit, Grep, Glob, Bash]
argument-hint: "[issue number] (optional)"
---

# /phase-done

The closing step of the issue workflow (`CLAUDE.md`). Only run this once
the issue's work is actually finished — this skill verifies and records, it
does not decide the work is done on your behalf.

## Steps

1. **Identify the issue being closed** (the user names it, or it is the
   issue this branch's PR declares `Closes #N` for). Read it with
   `gh issue view <n> --json title,body,comments` (never `--comments`, which
   prints nothing for an issue without comments on gh 2.101.0); if it links
   a plan file
   (`docs/plans/*.md`), that file's exit-criteria checklist also applies.
2. **Verify every `## Acceptance criteria` checkbox is ticked** — in the issue
   body and in any linked plan file. If any remain `- [ ]`, stop and list
   them — do not tick a criterion yourself just to proceed; a tick must
   reflect real, verified state (e.g. "workspace builds" means someone
   actually ran `cargo build --workspace` and it succeeded). Tick verified
   boxes in the issue body via `gh issue edit <n> --body-file`.
2a. **Relationships check** (`scripts/gh/rel.sh tree <n>`;
   `.claude/rules/issue-relationships.md`): if the issue is a **parent** with
   **open sub-issues**, do NOT close it — finish or re-parent the children
   first (a parent's job is done only when its decomposition is). Closing this
   issue auto-unblocks anything it was `blocking`, which is expected; note any
   dependents that become workable so the handoff comment can point at them.
2b. **Fields check** (`scripts/gh/fields.sh show <n>`;
   `.claude/rules/issue-workflow.md`): the issue carries a type, a priority
   and an effort, and a `Task` carries exactly one work-kind label. Set what
   is missing before closing, so the closed record says what the work was.
3. **Spec-adherence check:** for work that shipped spec-facing behaviour,
   confirm a conformance pass happened (`/spec-audit` findings addressed or
   filed as new issues). If it never happened, stop and say so — that is an
   unmet exit criterion in spirit.
3a. **CNF zero-drift gate:** confirm a full CNF 2.0 run
   (`/run-conformance` → `scripts/conformance.sh`) happened at close and
   shows zero drift vs the committed baseline
   (`docs/conformance/ferroehr/results.json` + `verdicts.json`; the
   baseline only ratchets upward), and that the ratcheted
   `docs/conformance/ferroehr/` artifacts (results.json, verdicts.json,
   CONFORMANCE_REPORT/STATEMENT/CERTIFICATE.md, badge*.json) are in-branch.
   No green CNF run → the issue is not closable.
3b. **User docs + changelog updated?** If this work changed a user-visible
   surface (REST, configuration, CLI, deployment artifacts), confirm BOTH:
   the matching `website/book/src` page was updated in-branch
   (`.claude/rules/docs-website.md`), AND a `CHANGELOG.md [Unreleased]` entry
   exists (`.claude/rules/changelog.md`; CI `changelog-guard` enforces it).
   If not, stop: both are part of the deliverable.
3c. **Living-reference-doc maintenance:** confirm the living reference docs
   (`docs/architecture.md`, `docs/VERSIONS.md`) were
   refreshed to verified reality for anything this work changed. (There is
   no blueprint/design-doc layer — internal plan/design files are deleted
   in the PR that implements them; the durable record is the closed
   issues + PR descriptions, `CHANGELOG.md`, git history, and these living
   reference docs.)
4. **Write the close narrative into the PR description**: what shipped,
   the key decisions with their spec citations, the gate results, and what
   was deliberately left out (with follow-up issue numbers). The PR
   description + the issue thread ARE the build record.
5. **Post the handoff comment on the issue** (`gh issue comment <n>`):
   where things stand at close, what was deliberately left out (with the
   follow-up issue numbers), and what a follow-up session should do first.
6. **Ensure the PR body declares `Closes #<n>`** (`gh pr view` /
   `gh pr edit`) so the merge into main auto-closes the issue — never
   close the issue by hand when a PR carries the work.
7. **Delete the implemented plan file** in the same PR (the
   delete-on-implementation lifecycle; `docs/plans/README.md`) — unless
   another still-open issue consumes the same plan file; then note that on
   the issue instead. Exceptions that are NEVER deleted:
   `docs/plans/WORKLIST.md` (the tracker pointer stub) and
   `docs/plans/README.md` (the lifecycle guide).
8. **Roadmap-board check** (`.claude/rules/project-board.md`): `Done` is set
   by the built-in workflow when the merge closes the issue — never by hand.
   After the merge, `scripts/gh/project.sh show <n>` should say `Done`; if
   the issue is missing from the board entirely, `scripts/gh/project.sh add
   <n>` and let the closed→Done workflow settle it. Do not archive or delete
   board items.
9. **Remind the user to commit** the close on the current
   conventional-type branch (`feat/…` etc., per the CLAUDE.md branch hard
   rule).

## What this skill does not do

It does not run `cargo build`, the test suite, or the CNF pipeline to "check"
the acceptance criteria for you — those must already have been run and have
genuinely passed before this skill is invoked. If in doubt, run
`/run-conformance` or the relevant `cargo` command first.
