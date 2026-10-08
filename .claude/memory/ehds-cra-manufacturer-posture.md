---
name: ehds-cra-manufacturer-posture
description: "Owner 2026-10-06 — Cadasto B.V. is manufacturer of each tagged release (FerroFED A73 adopted); FerroEHR+FerroBRIDGE is one EHR system, FerroBRIDGE ships the interoperability component; v4.3.6 = EHDS+CRA readiness (v4.3.5 became a security-only patch, 2026-10-08)"
metadata:
  node_type: memory
  type: project
  originSessionId: 7514b6fd-a08b-4154-8327-9b3929e46f2e
  modified: 2026-10-06T08:22:12.303Z
---

Owner decisions 2026-10-06, recorded in #3606 until `docs/architecture.md` carries them:
- FerroEHR adopts FerroFED's A73: each tagged release is a product Cadasto B.V. places on the market; BUSL is not FOSS (CRA Art 3(48)); CRA Art 14 reporting applies now; one tech doc/DoC/CE for CRA+EHDS from 11 Dec 2027.
- The EHR system is FerroEHR + FerroBRIDGE: FerroBRIDGE ships the European interoperability component, FerroEHR the logging component.
- v4.3.5 (due 2026-12-11) became the EHDS+CRA readiness milestone (#3606–#3626); the former v4.3.5 work moved to v4.3.6.
- 2026-10-08: that readiness milestone was renamed v4.3.6 so v4.3.5 could be a security-only patch cut from the v4.3.4 tag (#3664, #3656, #3629, #3700); the old v4.3.6 became v4.3.7. The in-tree FHIR connector is removed in v4.3.6 in favour of FerroBRIDGE (#3080).

**Why:** the old readiness page said the deployer is the manufacturer; that reading is withdrawn.
**How to apply:** never re-propose the deployer-as-manufacturer reading; compliance pages and issues follow these two decisions. Related: [[licence-busl]], [[platform-shape-single-tenant-ferrobridge]], [[compliance-corpus-direction]].
