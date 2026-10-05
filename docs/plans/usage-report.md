# Usage report to FerroPULSE (tracker issue #3577)

- Status: in-progress
- Started: 2026-10-05
- Consumes: `docs/law/` (GDPR, CRA, EHDS, NIS2, UAVG, BW 7, BDSG, StGB, DSG, DSV);
  the legal research in `usage-report/research-law.md`; no openEHR spec governs
  this, it is our own design.

## Objectives

Every FerroEHR instance sends one small installation report a day to FerroPULSE,
the Cadasto-operated collector and Grafana dashboard on Hetzner (EU): liveness,
version, licence grant type and coarse performance aggregates. On by default,
announced by one INFO line at every boot, switched off by one setting. The
collector and dashboard live in their own repository (FerroPULSE).

## Tasks

- [ ] #3580: vendor ePrivacy Art. 5(3) and Tw art. 11.7a; adjudicate the default.
- [ ] #3578: the client in `app/ferroehr` (payload v1, one send per day across
      replicas, failure isolation, `ferroehr usage-report --print`).
- [ ] #3579: the book page (GDPR Art. 13/14 notice, CRA Annex II information),
      the four pages that say FerroEHR sends nothing, the changelog.
- [ ] #3581, #3582: law-corpus provenance findings from the research.

## What FerroEHR expects of FerroPULSE

The client relies on these collector properties, and the book page states them:

- `POST /v1/report`, HTTPS only, body at most 64 KiB, answers `204`.
- The source IP (IPv4 or IPv6) and the country derived from it are stored
  with each report (owner decision 2026-10-05), under the retention period
  and erasure by instance id.
- A fixed retention period, published on the book page.
- Erasure of every stored report for an instance id on request.
- The dashboard sits behind access control, never public.

## Exit criteria

- [ ] The acceptance criteria of #3577 and its sub-issues are ticked.
- [ ] This file and `usage-report/` are deleted in the PR that closes #3577.
