# Golden Helm renders

These are the committed `helm template` outputs the chart is expected to
produce, used by `deploy/helm/validate.sh` (and CI) to catch unintended drift.

One per case in `deploy/helm/validate.sh`, and the list is complete: a case
without a golden is a render nothing compares.

- `default.yaml` — `ci/default-values.yaml` (chart defaults + an external-Secret
  DB DSN; all optional integrations OFF, the hardened default security posture on).
- `all-features.yaml` — `ci/all-features-values.yaml` (every optional
  integration enabled, separate management port, autoscaling, egress policy,
  mounted config files, the migration hook, the three backup CronJobs, and
  FerroTERM serving the shaped seed alone).
- `basic-auth.yaml` — `ci/basic-auth-values.yaml` (a Basic user whose Argon2id
  hash arrives through `secrets:` and is mounted as a file).
- `viewer.yaml` — `ci/viewer-values.yaml` (the viewer as a second pod-bearing
  workload, so the restricted-profile gate sees it).
- `terminology.yaml` — `ci/terminology-values.yaml` (FerroTERM as a second
  pod-bearing workload, with a built index mounted from an existing claim).
- `audit-categories.yaml` — `ci/audit-categories-values.yaml` (a German SGB V
  § 309 controller: the access-log horizon as `retention_years: 3` and an EHDS
  priority-category map under `[audit.categories]`).

## Regenerating

Whenever a chart template or the `ci/*-values.yaml` overlays change and the
render is *intended* to differ, regenerate and review the diff:

```bash
deploy/helm/validate.sh --update
git diff deploy/helm/golden
```

`deploy/helm/validate.sh` (no args) fails on any drift, so a stale golden is
caught before merge. Renders are pinned to release name `ferroehr`, namespace
`ferroehr`. Do not hand-edit these files.
