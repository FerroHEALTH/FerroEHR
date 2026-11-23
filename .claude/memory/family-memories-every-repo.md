---
name: family-memories-every-repo
description: a family-wide owner rule is written into the .claude/memory of every FerroHEALTH repository through a PR, never only into the repo the session runs in (owner 2026-11-23)
metadata:
  type: feedback
---

When the owner gives a rule that applies to the whole family, it goes into
`.claude/memory/` (and the matching `.claude/rules/` file) of every
FerroHEALTH repository: FerroAUDIT, FerroBRIDGE, FerroEHR, FerroFED,
FerroHEALTH, FerroLICENCE, FerroPIX, FerroPULSE, FerroSMART, FerroSYS,
FerroTERM and FerroUI. Each goes in through a merged PR, because workers start
from origin/main and never see an uncommitted file. Owner, 2026-11-23:
"we need to put these memories as well in the other repos so it's not only
inside bridge ... very very important ... otherwise the agents will never
understand it".

**Why:** a worker in one repository reads only that repository's memory and
rules; a rule kept in one repository alone never reaches the others.

**How to apply:** write the memory where the session runs, then open one
small PR per family repo with the same rule in that repo's format, and watch
each to merge. Related: [[lean-comments-no-history]],
[[memory-lives-in-repo]].
