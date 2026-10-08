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

**How to apply:** before writing a page in FerroEHR's book, ask whether it is about FerroEHR alone. Material about the manufacturer (post-market, vulnerability handling, advisories, support period, licensing), about the EHR system FerroEHR and FerroBRIDGE form together (EHDS intended purpose, readiness, shared responsibility, information sheet, instructions for use), or about the CRA's manufacturer side belongs in the FerroHEALTH book; FerroEHR's book links there. Product-specific pages (install, config, API, its own CRA risk assessment and Annex II user information) stay here. See [[ehds-cra-manufacturer-posture]].
