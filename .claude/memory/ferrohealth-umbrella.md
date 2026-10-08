---
name: ferrohealth-umbrella
description: "FerroHEALTH is the mother name of the product family; family-level documentation belongs in the FerroHEALTH book, not in one product's book"
metadata:
  node_type: memory
  type: project
  originSessionId: 44d54adc-7226-4d71-bc1a-2ecc4709cb40
  modified: 2026-10-08T09:10:23.250Z
---

**FerroHEALTH** is the family (mother) name; FerroEHR is one product in it, beside FerroBRIDGE, FerroTERM, FerroCHART, FerroFED and the planned FerroPIX, FerroSMART, FerroSYS, FerroTASK; every product links one family book instead of copying it. Cadasto B.V. is the manufacturer of every product. The family site is <https://ferrohealth.eu> (repo FerroHEALTH/FerroHEALTH, local `/Users/ruben.talstra/RustroverProjects/FerroHEALTH`); each product documents itself on its own domain.

**Why:** owner 2026-10-08: "most of our documentation needs to go to FerroHEALTH because not everything is only for FerroEHR… it's about the bundled products". Tracked as FerroHEALTH#37 (the family mdBook) with one issue per product: FerroEHR#3701 (v4.3.6), FerroBRIDGE#402, FerroTERM#709, FerroFED#848, FerroCHART#233. The owner: "otherwise we are duplicating so much of the same things"; a new product (FerroPIX first) starts from the family book.

**How to apply:** before writing a page in FerroEHR's book, ask whether it is about FerroEHR alone. The first move (#3701, before v4.3.6) put the manufacturer, the shared security policy, the post-market procedure, the CRA's manufacturer side, the EHDS intended purpose and shared responsibility, licensing and the family architecture at `https://ferrohealth.eu/docs/` (`manufacturer.html`, `security.html`, `post-market.html`, `cra.html`, `ehds/intended-purpose.html`, `ehds/shared-responsibility.html`, `licensing.html`, `architecture.html`); FerroEHR keeps short pages at the old URLs with only its own facts. The information sheet, the instructions for use and the generated EHDS readiness/technical-documentation pages STAY here for now (the release lane attaches or checks them per release; moving them needs the lane to read the family book at a pinned revision, a follow-up). Product-specific pages (install, config, API, its own CRA risk assessment and Annex II user information, hazard log, claims review, control matrix) stay here. The page table is `.claude/rules/docs-website.md`. See [[ehds-cra-manufacturer-posture]].
