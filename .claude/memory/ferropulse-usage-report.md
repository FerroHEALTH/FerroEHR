---
name: ferropulse-usage-report
description: FerroPULSE (private repo FerroHEALTH/FerroPULSE, created 2026-10-05) collects a daily installation report from every FerroHEALTH product; on by default; FerroEHR side is #3577
metadata:
  type: project
---

Owner decision 2026-10-05: every FerroHEALTH product (FerroEHR, FerroTERM, FerroBRIDGE, FerroFED) sends one daily installation report to FerroPULSE, a Cadasto-run collector + Grafana on a Hetzner CX23 (EU), endpoint `https://report.ferropulse.eu/v1/report` (domain owned since 2026-10-05). On by default, one INFO boot line, one switch off. FerroPULSE owns the envelope schema (`product`, `instance_id`, version, licence grant TYPE only, liveness) and each product's `metrics` schema. Local checkout `../FerroPULSE` (pushed over HTTPS; SSH key not set up). v4.3.5 was created that day with no due date at the owner's request; v4.3.4 was cut down to quick wins + the report, due 2026-10-09.

**Why:** the owner wants to see which versions and licence types run in the field and how they perform; big vendors (ChipSoft, Epic) do this contractually, the owner chose anonymous-style default-on.

**How to apply:** the FerroEHR payload never carries the licence id, patient values, AQL text or request paths; the collector stores each report's source IP and country (owner decision 2026-10-05), so never call the report "anonymous" in published prose; the field contract is FerroPULSE `docs/report-v1.md`; the legal research (`docs/plans/usage-report/research-law.md`, deleted at close, read it at `944803867`) and the ePrivacy Art. 5(3) adjudication are on #3577 and #3580; the owner kept the report on by default ([[usage-report-default-on-accepted-risk]]). See [[sibling-products]], [[compliance-corpus-direction]].
