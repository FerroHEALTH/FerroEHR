---
name: cadasto-hosting-and-certifications
description: "Owner 2026-10-06 — Cadasto may host FerroEHR as a service (then operator + GDPR processor, EHDS Art 26(2)); Cadasto holds ISO 9001/27001/NEN 7510 (org, not product)"
metadata:
  node_type: memory
  type: project
  originSessionId: 7514b6fd-a08b-4154-8327-9b3929e46f2e
  modified: 2026-10-06T15:48:24.443Z
---

Two owner facts of 2026-10-06, recorded in `docs/architecture.md` §Regulatory position:

- Cadasto B.V. may host FerroEHR as a service for other organisations. Every page states both cases: self-hosted (Cadasto is manufacturer only) and hosted (Cadasto is also operator and the customer's GDPR Art 28 processor; the system is "put into service", EHDS Art 26(2)). Never write "Cadasto does not operate it" or "the manufacturer is not your processor" unconditionally.
- Cadasto B.V. holds ISO 9001, ISO/IEC 27001 and NEN 7510 (badges on cadasto.com). They certify the organisation's management system, never the product: never call FerroEHR certified. Whether the scope covers FerroEHR development and hosting is an open owner fact.

**Why:** the owner corrected a page that assumed self-hosting only.
**How to apply:** check compliance prose for single-case wording; tell compliance workers both facts. Related: [[ehds-cra-manufacturer-posture]], [[platform-shape-single-tenant-ferrobridge]].
