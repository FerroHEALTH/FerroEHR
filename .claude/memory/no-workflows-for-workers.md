---
name: no-workflows-for-workers
description: Start workers with the Agent tool only, never the Workflow tool, because a workflow agent cannot be messaged (owner 2026-11-01)
metadata:
  type: feedback
---

Start every worker with the `Agent` tool (`model: 'opus'`, in the background). Never fan workers out through the `Workflow` tool.

**Why:** the owner, 2026-11-01. The five product workers ran in one workflow, and FerroTERM #956 then hit a merge conflict. A workflow agent does not appear in the agent list and takes no `SendMessage`, so the orchestrator could not tell the worker and had to resolve the conflict itself.

**How to apply:** for parallel work, send several `Agent` calls in one message, within [[max-five-concurrent-workers]]. Each one can then be messaged with a correction, a follow-up issue or a conflict to fix. Related: [[workers-on-opus-only]], [[worker-wait-loops]].
