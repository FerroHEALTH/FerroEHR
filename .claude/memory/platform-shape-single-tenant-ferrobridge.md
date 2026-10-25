---
name: platform-shape-single-tenant-ferrobridge
description: "FerroEHR is one instance per tenant (no in-database tenancy), secondary use leaves through FerroBRIDGE over batch AQL (no research domain in the CDR), and every seam to a sibling is a published protocol (owner rulings 2026-09-14 onward)"
metadata:
  type: project
---

The owner's FerroHEALTH drawing (2026-09-14) placed FerroEHR in a
single-tenant instance beside its siblings. The product set and their lanes
are now fixed by [[family-architecture]]: FerroTERM for terminology,
FerroBRIDGE for FHIR and for OMOP (loaded in batches over AQL, consuming no
outbox), FerroFED for federation, FerroPIX for identity and the IHE profiles.

Rulings absorbed into the storage rewrite: no `tenant_id`, RLS, GUC or tenant
registry in the schema (#3378); no `research` domain, the CDR keeps the
restriction, objection and retention marks and serves AQL over VERSION by
`time_committed` as the batch surface (#3379, now in v5.5.0); RLS is not kept
as defence in depth. The FHIR surface question (#3386) is closed: FHIR is
FerroBRIDGE's, and FerroEHR serves no FHIR face. The drawing's central
authorisation server is withdrawn (owner, 2026-10-19; #3381 closed): each
product is its own SMART resource server over `ferrohealth-smart`. Federation
prior art: the syntaric "Federation Tier with AQL" 0.9 proposal (CC0), input
only (#3383).

**Why:** removing structure later would be a second rewrite; the owner wants
the CDR to match the platform shape now.

**How to apply:** treat the drawing's material as input, never as best
practice; every seam to a sibling is the open protocol it realises (ITS-REST
and AQL inbound, FHIR terminology outbound), never a crate dependency or an
in-CDR copy of the sibling's function; do not re-add tenancy or an in-CDR
research store. See [[storage-rewrite-is-greenfield]], [[sibling-products]].
