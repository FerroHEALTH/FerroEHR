---
name: no-hand-written-copies
description: Owner 2026-10-25, the family rule. Nothing is hand-written in FerroEHR when a family server or shared crate covers it; a missing piece is added to the owning repository, never copied here
metadata:
  type: feedback
---

FerroEHR writes no code of its own for what a FerroHEALTH server or shared crate covers. It uses the crate at its newest release: `ferrohealth-smart` for authentication and the outbound client credentials grant, `ferrohealth-authz` for the policy decision once it is published, `ihe-atna` and `ehds-logging` for the whole audit trail (the local audit store's retrieval included), `ferrobridge-term` for the terminology client, `ihe-iti` as the client library of a lookup at FerroPIX, and `ferrohealth-ui` for the viewer. When the shared crate lacks what FerroEHR needs, the missing piece is filed and built in the owning repository (FerroAUTH, FerroAUDIT, FerroBRIDGE, FerroPIX or FerroUI), released, and then taken here. A local copy, an adapter that a shared crate could carry, or a "temporary" workaround is never written or grown.

**Why:** the owner, 2026-10-25: "we should use as much of those shared things we have, that's why we have these shared items", and the family ruling the same day that this gives a proper, stable ecosystem. A copy in one product drifts from the shared crate and every other product that uses it.

**How to apply:** before writing auth, audit, terminology-client, IHE-client or UI code, check the shared crates for it. If one covers it, use it. If one almost covers it, file the gap in the owning repository, build it there, and wait for the release; the FerroEHR side then deletes its own copy. What stays here is what is CDR-specific: the openEHR services, the route table, the store queries, and operational surfaces no family crate covers. FHIR and OMOP mapping belong to FerroBRIDGE, which reaches FerroEHR over ITS-REST, and are never taken in. FerroEHR is the openEHR CDR, never an IHE actor. See [[sibling-products]], [[ferroui]].
