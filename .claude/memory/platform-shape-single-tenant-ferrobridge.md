---
name: platform-shape-single-tenant-ferrobridge
description: Owner's FerroHEALTH drawing (2026-09-14) binds FerroEHR: one instance per tenant (no in-database tenancy in the rewrite), secondary use via FerroBRIDGE over batch AQL (no research domain), platform seams in v4.3.2
metadata:
  type: project
---

The owner's FerroHEALTH architecture drawing and note (2026-09-14, `~/Downloads/FerroHEALTH/A-future-architecture.{svg,md}`) place FerroEHR inside a single-tenant instance beside FerroTERM, FerroBRIDGE (live, OMOP loaded in batches over AQL, consumes no outbox), FerroCHART, and the proposed FerroPIX (MPI), FerroFED (federation), FerroSYS (control plane, notification subscriptions). Rulings absorbed into the storage rewrite: no `tenant_id`, RLS, GUC or tenant registry in the new schema (#3378); no `research` domain, the CDR keeps the marks and serves AQL over VERSION by `time_committed` as the batch surface (#3379); RLS is NOT kept as defence in depth. Open owner decisions: the FHIR surface overlap with FerroBRIDGE (#3386), erasure propagation channel (#3379/#3382). The drawing's central authorisation server is withdrawn (owner, 2026-10-19; #3381 closed): there is no central authorisation service, each product is its own SMART resource server, because one central service would hold the signing key every product's access depends on. Federation prior art: the Syntaric "Federation Tier with AQL" 0.9 proposal (CC0), input only (#3383).

**Why:** removing structure later would be a second rewrite; the owner wants the rewrite to match the platform shape now.

**How to apply:** treat the drawing's material as input, never as best practice; every seam to a sibling is specified by the open standard it realises with a local default; do not re-add tenancy or an in-CDR research store. See [[storage-rewrite-is-greenfield]], [[sibling-products]].
