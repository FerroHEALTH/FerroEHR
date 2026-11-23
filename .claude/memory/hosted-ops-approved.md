---
name: hosted-ops-approved
description: the orchestrator may operate the FerroEHR hosted sandbox and its database server over SSH when the work needs it; the owner approved it permanently (2026-11-22)
metadata:
  type: feedback
---

The orchestrating session may operate the FerroEHR hosted sandbox (app box
and database box, reached over SSH with the existing key) whenever a task
needs it, without asking first: inspecting, stopping and starting services,
recreating a database on a new PostgreSQL major, reseeding the sandbox.
Owner, 2026-11-22: "you have my full permission to all of this when it's
needed".

**Why:** release work should not wait on the owner for an operation the
orchestrator can do.

**How to apply:** a worker inspects read-only first and sends a plan with a
rollback; the orchestrator approves it; old data is kept (renamed volume or
data directory) until the orchestrator releases it; no secret or private key
goes into a log, issue or message. Nothing is in production
([[ferrohealth-not-in-production]]), so sandbox data may start fresh.
