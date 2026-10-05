---
name: usage-report-default-on-accepted-risk
description: Owner decision 2026-10-06 — FerroEHR's usage report stays ON by default despite the #3580 adjudication reading ePrivacy Art. 5(3)/Tw 11.7a as requiring prior consent; do not re-propose opt-in
metadata:
  type: project
---

The #3580 adjudication (`docs/plans/usage-report/adjudication-3580.md`) concluded that Art. 5(3) applies on its wording, no exemption fits, and a default-on report with a boot line is not consent; it recommended opt-in. Offered three options (opt-in + licence flag, opt-in only, keep on), the owner chose "Keep on by default" and accepts the residual risk.

**Why:** the owner wants fleet visibility from every install; prior art (Grafana, GitLab) ships default-on.

**How to apply:** keep `[usage_report] enabled = true` as the shipped default and the boot line plus single off switch; do not reopen the default unless the owner asks or a regulator/court reading changes the facts; the book page states the default plainly and never claims consent or "compliant". See [[ferropulse-usage-report]].
