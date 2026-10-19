---
name: sibling-products
description: "The Ferro family is four repos side by side under ~/RustroverProjects (ferroehr, FerroCKM, FerroTERM, FerroBRIDGE), all BUSL-1.1 since 2026-09-04; FerroTERM's checkout was renamed from notio on 2026-09-03; FerroBRIDGE owns the FHIRconnect bridge and the in-tree FHIR extension is expected to move there"
metadata: 
  node_type: memory
  type: project
  originSessionId: bf54647c-c2da-4ef6-b22d-44b6fb729b01
  modified: 2026-09-04T14:22:26.039Z
---

Four products, four repositories: FerroEHR (this repo, the CDR), FerroCKM (`../FerroCKM`, the Clinical Knowledge Manager, created 2026-09-03, BUSL-1.1 with FerroEHR's grant so the two share the application layer; research on its #1), FerroTERM (`../FerroTERM`, the FHIR terminology server; the local checkout was `../notio` until 2026-09-03 and the harness memory link was moved with it) and FerroBRIDGE (`../FerroBRIDGE`, the bridge from openEHR to HL7 FHIR (FHIRconnect spec, openFHIR as prior art) and to the OMOP CDM (OMOCL spec, Eos as prior art), created 2026-09-03). FerroEHR issues #2646 and #2652 were transferred there (FerroBRIDGE #1 and #2).

Licences: all four are Business Source License 1.1. FerroTERM and FerroBRIDGE were Apache-2.0 at creation (checked 2026-09-03) and the owner moved both to BUSL-1.1 on 2026-09-04 (stated in the FerroEHR session; not re-verified in their trees from here). Since the same day only the five generated `openehr-*` model crates are Apache-2.0; the three hand-written engines are BUSL, see [[license-busl]].

**Why:** the owner treats each as a separate product but wants the whole family under the same resale and hosting protection; the earlier plan to keep the bridge open source was reversed on 2026-09-04.

**How to apply:** never edit a sibling repo from a FerroEHR session unless the owner asks; when a FerroEHR text describes a sibling's licence, say BUSL-1.1 and, if it matters, check the sibling's `LICENSE` first-hand; FerroEHR's `ferroehr-ext` FHIR feature is expected to be retired in favour of FerroBRIDGE (#3080); FerroBRIDGE consumes FerroEHR strictly over ITS-REST, never as a crate dependency. See [[license-busl]].

Update 2026-09-14: the FerroHEALTH platform services (FerroPIX, FerroFED, FerroSYS) are each their own product in their own repository; FerroFED's repository is confirmed. FerroEHR builds only the seams (#3377). Update 2026-10-19: the planned authorisation-server product is dropped; there is no central authorisation service, and each product is its own SMART resource server, because one central service would hold the signing key every product's access depends on. The openEHR Federation Working Group's "Federation Tier with AQL" proposal (syntaric, v0.9.0 RC, CC0) and its Apache-2.0 reference gateway are input for #3383, never a specification. See [[platform-shape-single-tenant-ferrobridge]].
