---
name: sibling-products
description: "The sibling products and shared-crate repositories sit beside this checkout under ~/RustroverProjects; never edit one from here; all are BUSL-1.1; FerroEHR reaches the others only over their published protocols (see family-architecture)"
metadata:
  type: project
---

The family is the five products in [[family-architecture]]: FerroEHR (this
repository, the openEHR CDR), FerroBRIDGE (`../FerroBRIDGE`, openEHR to FHIR
through FHIRconnect and to the OMOP CDM through OMOCL), FerroFED
(`../FerroFED`, the federation gateway), FerroPIX (`../FerroPIX`, the master
patient index and the IHE profiles) and FerroTERM (`../FerroTERM`, the FHIR
terminology server; the local checkout was `../notio` until 2026-09-03). All
five are repositories of the `FerroHEALTH` organization. The shared crates
live in their own repositories beside them: FerroAUTH (`FerroHEALTH/FerroAUTH`,
renamed from FerroSMART on 2026-10-25; local checkout `../FerroSMART`),
FerroAUDIT (`../FerroAUDIT`) and FerroUI (`../FerroUI`).

FerroEHR issues #2646 and #2652 were transferred to FerroBRIDGE on 2026-09-03
(FerroBRIDGE #1 and #2). The in-tree FHIR connector was retired in favour of
FerroBRIDGE (#3080, removed in v4.3.6), and the owner settled the FHIR surface
question (#3386, closed): FerroEHR serves no FHIR face.

Licences: every product is Business Source License 1.1. FerroTERM and
FerroBRIDGE were Apache-2.0 at creation and the owner moved both to BUSL-1.1
on 2026-09-04 (stated in the FerroEHR session; check a sibling's `LICENSE`
first-hand before quoting it). Only the five generated `openehr-*` model
crates are Apache-2.0, see [[license-busl]].

There is no central authorisation service (owner, 2026-10-19): each product
is its own SMART resource server over the shared `ferrohealth-smart` crate,
because one central service would hold the signing key every product's
access depends on. The openEHR Federation Working Group's "Federation Tier
with AQL" proposal (syntaric, v0.9.0 RC, CC0) and its Apache-2.0 reference
gateway are input for FerroFED's node contract (#3383), never a
specification.

**Why:** the owner treats each product as separate and wants the whole
family under the same resale and hosting protection.

**How to apply:** never edit a sibling or shared-crate repository from a
FerroEHR session unless the owner asks. FerroEHR consumes no sibling as a
crate dependency; the siblings reach FerroEHR over ITS-REST and AQL, and
FerroEHR reaches FerroTERM as a FHIR terminology client through
`ferrobridge-term`. When a FerroEHR text describes a sibling's licence, say
BUSL-1.1. See [[family-architecture]], [[platform-shape-single-tenant-ferrobridge]].
