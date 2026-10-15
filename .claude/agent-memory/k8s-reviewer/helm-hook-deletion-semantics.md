---
name: helm-hook-deletion-semantics
description: When Helm 4 actually deletes hook-succeeded resources (after ALL hooks of the event, and on failure it deletes the EARLIER successful hooks) — verified from helm/helm v4.2.3 pkg/action/hooks.go
metadata:
  type: reference
---

Verified first-hand 2026-10-14 from `helm/helm` tag v4.2.3,
`pkg/action/hooks.go` `execHookWithDelayedShutdown`:

- Hooks of one event run sorted by weight, then name. `before-hook-creation`
  deletes run per hook just before its create.
- On SUCCESS of every hook, `hook-succeeded` deletions run afterwards, in
  REVERSE order. So a hook ServiceAccount at a lower weight than its Job is
  NOT deleted while the Job runs.
- On FAILURE of hook i (e.g. Helm `--timeout` expiring before the Job's
  `activeDeadlineSeconds`), Helm deletes hooks `[0:i]` that carry
  `hook-succeeded`. A still-active Job then loses its ServiceAccount, and its
  retry pods are refused at admission (serviceaccount not found).
- WatchUntilReady only blocks on Jobs/Pods; an SA hook returns at once.

**How to apply:** for a hook-SA + hook-Job pair, check the SA's delete policy
against the failure path, and compare the Job's `activeDeadlineSeconds` with
Helm's default 5m timeout. Re-fetch the file if the pinned Helm version moves.

Related: [[chart-review-read-only-procedure]]
