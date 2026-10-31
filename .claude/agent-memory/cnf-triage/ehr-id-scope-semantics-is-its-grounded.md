---
name: ehr-id-scope-semantics-is-its-grounded
description: The query ehr_id/openehr-ehr-id scope's EFFECT is defined by the ITS-REST query docs (not spec-silent); EHRbase 2.34.0 discards both carriers
metadata:
  type: project
---

Adjudicated 2026-08-13 for `execute_ad_hoc_query-empty_db_bare_ehr` on the
committed EHRbase record (issue #2362).

**The scoping semantics IS released-spec-defined**, contrary to AMB-101's
"never stated" claim. (The Service Model text on `_ehr_ids_` says the same, but
it is reference text, never an authority: owner ruling 2026-10-27.)

- ITS-REST `specifications/docs/query/Query_types.md` §Single EHR queries:
  single-EHR execution "is achieved by supplying an `ehr_id` query parameter or
  setting an `openehr-ehr-id` request header".
- ITS-REST `.../docs/query/Request.md` §Common Headers and Query Parameters:
  "`ehr_id` - used to execute the query within a single EHR context".
- Released OAS `query-codegen.openapi.yaml` `components/parameters/ehr_id_Query`:
  "An optional parameter to execute the query within an EHR context" — optional
  to SUPPLY, not optional to honour.

The only weakening sentence is Request.md §About the ehr_id parameter ("MAY be
used by the underlying backend to perform routing, optimizations or similar") —
about ADDITIONAL backend uses; reading it as permission to ignore a supplied
scope contradicts §Single EHR queries in the same chapter, and the release
declares NO branch for "scope unsupported". Caveat to state honestly: parameter
SUPPORT is a SHOULD ("All query execution requests SHOULD support at least…").

AQL alone cannot supply the constraint: QUERY `master06-writing_AQL.adoc` — an
unscoped `FROM` (no `ehr_id/value`) IS a population query over all EHRs.

**EHRbase 2.34.0 (reproduced 2026-08-13, docker/sut-ehrbase.yml):** both
carriers accepted with 200 and DISCARDED. `POST /query/aql?ehr_id=<A>` with
`SELECT e/ehr_id/value FROM EHR e` returned all 3 EHRs and disclosed
`meta._executed_aql = "SELECT e/ehr_id/value FROM EHR e LIMIT 100"` (no EHR
predicate injected); the `openehr-ehr-id` header form rewrote
`FROM EHR e CONTAINS COMPOSITION c` to `FROM COMPOSITION c`. The AQL-level
predicate `EHR e[ehr_id/value='A']` DOES scope (1 row) — which is why sibling
`empty_db` passes and `empty_db_bare_ehr` fails.

The runner never resets between cases (fresh volumes per RUN only), so a bare
`FROM EHR e` sees every EHR the run minted (548 schedule files carry an
`ehr:` requirement) — the catalogue realizes the "empty ground" through the
scope on purpose. Scope-ignoring SUTs therefore also make every
scoped-count-0 query row pass VACUOUSLY.
