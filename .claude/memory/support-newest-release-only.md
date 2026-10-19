---
name: support-newest-release-only
description: "Owner 2026-10-19 (final): only the newest release is supported, no fixes/backports for older ones, upgrade is free; the CRA five years is a product-wide commitment to keep publishing fixes (Art 13(8)), shipped only in the newest release (Art 13(10)); the end month stays printed"
metadata:
  node_type: memory
  type: project
  modified: 2026-10-19T00:00:00.000Z
---

Owner decision 2026-10-19, final, the same for every FerroHEALTH product (#3853):
- Only the newest release is supported. An older release never receives a fix, backport or patch; the remedy is to upgrade, which is free.
- The CRA five-year minimum is a commitment for the product: Cadasto B.V. keeps publishing security fixes for at least five years after each release is placed on the market (Art 13(8)), delivered only in the version last placed on the market (Art 13(10)).
- The printed end month (Art 13(19), Annex II point 7) stays on the banner, `ferroehr --version`, `/management/info` and `/management/status`: it is the date until which fixed releases keep coming, and a release is supported only while it is the newest.

**Why:** "each release is supported for five years" read as a per-release support promise the project does not keep.
**How to apply:** never write per-release support periods ("supported for five years from the month of its tag"); use the shared wording in `SECURITY.md` §Supported versions. Related: [[ehds-cra-manufacturer-posture]].
