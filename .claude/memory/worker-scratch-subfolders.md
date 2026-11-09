---
name: worker-scratch-subfolders
description: Each worker writes scratch files under its own subfolder of the session scratchpad, never the root, checks a file is its own before publishing it, and re-reads every PR body after setting it (2026-10-20)
metadata:
  type: feedback
---

Every worker in a session shares one scratchpad directory. A worker writes its scratch files (PR bodies, issue comments, notes, reports) under its own subfolder named for its task (`<scratchpad>/<task>/`), never at the scratchpad root. A git worktree never goes in the scratchpad ([[worktrees-for-parallel-branches]]).

**Why:** on 2026-10-20 a FerroAUTH worker left `pr1.md` at the scratchpad root, and a FerroBRIDGE worker published it as the body of another pull request (#998), which merged without closing its own issue (#1021). A generic file name at a shared root is a collision waiting for the next worker.

**How to apply:**
- Every worker brief names the worker's own scratch subfolder and forbids the root.
- Before any `--body-file` or `-F` argument, check the file lives under your subfolder and names your own issue.
- After setting a PR body, re-read it (`gh pr view <n> --json body,closingIssuesReferences`) and confirm the `Closes #N` lines name exactly your issues.
- A long report is written to a file in that subfolder ([[research-reports-go-to-files]]).
