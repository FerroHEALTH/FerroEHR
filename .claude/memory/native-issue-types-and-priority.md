---
name: native-issue-types-and-priority
description: Since 2026-10-02 type/priority/effort are GitHub's native issue type + the FerroHEALTH Priority and Effort issue fields (scripts/gh/fields.sh), not labels; bug/enhancement/P0-P3 deleted; only open issues were migrated
metadata:
  type: project
---

Owner decision 2026-10-02 (#3524, mirroring FerroHEALTH/FerroFED#154): an issue's type is the native issue type (Bug/Feature/Task), its priority the org `Priority` field (Urgent/High/Medium/Low), its effort the org `Effort` field (High/Medium/Low). A Task carries exactly one work-kind label (documentation/chore/refactor/perf/test/ci). Policy: `.claude/rules/issue-workflow.md`.

Migration 2026-10-02: `scripts/gh/migrate-fields.sh` moved the 42 OPEN issues only (owner: "migrate only the open one"); closed issues were left without type or priority. Then `scripts/gh/labels.sh` deleted `bug`, `enhancement`, `P0`–`P3` (owner: "we delete the labels").

**Why:** native fields sort and filter on the platform, and FerroFED/VernumBOEK already run this model.

**How to apply:**
- File every issue with `scripts/gh/fields.sh new <type> <priority> <effort> --title … --body-file … --milestone …`, never bare `gh issue create`.
- Read issues with `gh issue view <n> --json title,body,comments`; `--comments` prints NOTHING on gh 2.101.0 for an issue without comments.
- Watcher lanes set the type only (REST update after create); priority and effort are set at pickup. `<?/?>` in the SessionStart dump marks the gap.
- The board's "Current focus" view filters `priority:Urgent,High`, which only works once the Priority field is added to the project in the UI (no mutation can add it).

Related: [[new-issues-go-to-next-patch-milestone]], [[pr-closes-one-keyword-per-issue]].
