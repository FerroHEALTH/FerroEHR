---
name: memory-lives-in-repo
description: Every learning is a tracked file in the repository's .claude/memory/ with an index line, never a per-user note (owner 2026-09-04)
metadata:
  type: feedback
---

Every memory (decision, owner preference, reference, correction) is a file in `.claude/memory/` with an entry in `.claude/memory/MEMORY.md`, committed and merged like any other change. The per-user auto-memory directory outside the repository is not used for project learnings.

**Why:** the owner, 2026-09-04: "the memory needs to be in the project you know that right!!! so everyone uses the same learning". A private note helps one machine; a tracked file teaches every session and survives a crash with the repository.

**How to apply:** write the file with the frontmatter and header the other files here carry, add its index line, and land it with the change that taught it. Check for an existing file first and update it rather than adding a duplicate. A lesson that holds for every FerroHEALTH repository is carried in each repository's own memory, adapted to its paths and scripts.
