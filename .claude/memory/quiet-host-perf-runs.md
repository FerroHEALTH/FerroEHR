---
name: quiet-host-perf-runs
description: a FerroEHR load or capacity run happens only on a quiet host, with no build, test or other container running beside it
metadata:
  type: feedback
---

A load or capacity measurement of FerroEHR runs only on a quiet host. The
orchestrator opens a quiet window; inside it nothing runs but the measured
load: no cargo build, test or clippy, no other container. Load runs and large
seeds take the podman lock so they never overlap. Deterministic budgets
(statement counts, buffers, WAL, allocations, callgrind) need no quiet host.

**Why:** numbers measured beside builds and tests are not trustworthy.

**How to apply:** record host load per step, tear the containers down with
`podman rm -f -v` afterwards, and stop the Podman machine when done. Related:
[[capacity-targets]].
