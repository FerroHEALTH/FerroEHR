---
name: chart-review-read-only-procedure
description: What a read-only FerroEHR chart review can actually run (validate.sh, helm lint/template) and the two gates it does NOT cover — chart-version-guard and chart-boot
metadata:
  type: reference
---

Read-only chart review here runs clean without a cluster or cargo:

- `helm version --short` must be `v4.2.3` or the golden compare is not
  byte-comparable (rule §8). Verified matching 2026-09-15.
- `bash deploy/helm/validate.sh` renders every `deploy/helm/ci/*-values.yaml`
  overlay and runs the structural gates (restricted profile per container,
  selector stability/disjointness, secret-leak, schema/fixture, golden compare,
  Artifact Hub metadata, README-vs-values). It skips `kubeconform` silently when
  that binary is absent — a "skipping schema validation (optional)" line is NOT
  a pass.
- Rendered-selector comparison across branches is worth doing directly:
  `diff <(git show main:deploy/helm/golden/X.yaml | grep -A6 matchLabels) ...`.

Two things `validate.sh` does not check and a reviewer must check by hand:

- **`chart-version-guard`** (`.github/workflows/ci.yml`, job `chart-version`):
  ANY diff under `deploy/helm/ferroehr/**` requires a `Chart.yaml` `version`
  bump, or the `no-chart-bump` label. The guard is pull-request-only, so it does
  not fire in a merge queue. Value overlays, goldens and `validate.sh` itself
  live one level up and do not trigger it.
- **`chart-boot`** (`deploy/helm/ci/boot-check.sh`): whether the image ACCEPTS
  the rendered `ferroehr.toml`. The root config struct is
  `#[serde(deny_unknown_fields)]` (`app/ferroehr/src/config/mod.rs`), so a
  removed config section left in an operator's own values file renders fine and
  crash-loops the pod — `values.schema.json`'s `config` node carries no
  `additionalProperties: false`, so the schema will not catch it either.

**How to apply:** run validate.sh and quote its result, then check the
Chart.yaml bump separately; never report a chart change as clean on
validate.sh alone.

Related: [[deployment-schema-name-surfaces]]

Observed 2026-10-14: `helm` was NOT on PATH on the review machine (no
`/opt/homebrew/bin/helm`), so validate.sh could not run. Check `command -v
helm` first and say plainly in the report when the gates were not run.
