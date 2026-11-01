---
name: aql-result-set-equivalence-location
description: Where AQL 1.1 result-set semantics (row order, DISTINCT/duplicates, ORDER BY, TOP/LIMIT, cell values) and the ITS-REST RESULT_SET wire shape live — for conformance result-equivalence rules
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# AQL result-set equivalence — location map

**AQL 1.1 semantics** (QUERY component, `docs/specs/openehr/QUERY/docs/AQL/`):
- `master04-result_structure.adoc` — whole chapter, ~8 lines: raw result =
  `Array<Array<Any>>`; NULL = missing/unknown cell; annotated result sets are
  "not formally defined by this specification" (delegated to SM Query + REST).
- `master03-syntax.adoc` — the SELECT chapter owns everything:
  - `§DISTINCT` (~L1055) — bag-by-default; DISTINCT removes rows equal on every column.
  - `§TOP` (~L1070) — deprecated 1.1.0; not combinable with LIMIT.
  - `§Name alias` (~L1089) — `AS` alias = AQL variable syntax.
  - `§ORDER BY` (~L1094) — the load-bearing NOTE at L1098: no ORDER BY ⇒
    "default ordering in results is undefined" (impl-defined or random).
    ASC default (L1111); multi-key tie-break left-to-right (L1113); sort needs
    Ordered/primitive comparability (L1109). NO null-ordering rule stated.
  - `§LIMIT` (~L1115) — OFFSET/row_count 0-based; determinism "requires ORDER BY";
    DISTINCT applied before LIMIT/OFFSET.
- `master06-writing_AQL.adoc §Ordering and pagination` (~L98) — worked example only.
- Aggregate fns (MIN/MAX/SUM/AVG/COUNT) ignore NULL inputs (master03 ~L507-519).

**ITS-REST RESULT_SET wire shape** (`docs/specs/openehr/ITS-REST/specifications/`):
- `schemas/query/ResultSet.yaml` — required=`rows` ONLY; meta/name/q/columns optional;
  rows described "An ordered set of RESULT_SET rows" (server order authoritative on wire).
- `schemas/query/ResultSetColumn.yaml` — `name` REQUIRED, `path` optional; the
  `#0`/`#1` 0-based hash-index convention for unaliased columns is defined HERE
  (NOT in AQL — AQL only has AS).
- `schemas/query/ResultSetRow.yaml` — row = untyped array (`items: {}`); cell = ANY;
  example mixes primitives + full RM objects with `_type` (DV_TEXT).
- `schemas/query/ResultSetMetadata.yaml` — all meta fields optional, additionalProperties:true.
- `docs/query/Request.md` — offset/fetch params; fetch "cannot be combined with AQL-top".
- `docs/overview/Resources.md` — canonical JSON SHOULD (not MUST) validate vs ITS-JSON;
  NO integer-vs-real cell number-typing rule stated.

**SPEC SILENCES (a conformance schedule must legislate):** no-ORDER-BY row order,
NULL sort position, cross-type cell number typing (int vs real rendering),
missing-cell repr (JSON null vs absent), meta-field equivalence. CNF query
schedule `CNF/docs/platform_test_schedule/master11-func_tc_querying.adoc` is a
STUB (all "xx"/"TBD", 108 lines) — defines NO result-comparison rules. Every
test case row (smoke_test §L48, execute_stored/adhoc_query-empty_db/loaded_db,
placeholder `bbbb`) has Description/Pre/Post/Flow = "xx"; only concrete field is
the `Test runners` .robot link.

**Extra exact-heading facts (verified 2026-07-21):**
- AQL headings in master03-syntax.adoc: `=== WHERE` (L989), `=== ORDER BY` (L1094,
  NOTE L1098), `=== LIMIT` (L1115), `==== DISTINCT` (L1055), `==== TOP` (L1070,
  deprecated 1.1.0), `== Functions` (L490) → Aggregate/String/Numeric/Date-time/
  Other subsections, `==== TERMINOLOGY` (L699–769, 3 WHERE usage positions only,
  ops expand/validate/lookup/map/subsumes non-closed), `==== Containment` (L958,
  CONTAINS; no literal "containsExpr" heading in prose — grammar token in
  master07-grammar.adoc), `== Identified Paths` (L251), `== openEHR path syntax` (L36).
- **TIMEWINDOW removed in Release 1.0.1** (NOT 1.1.0): master00-amendment_record.adoc
  L88 SPECQUERY-20, completed 15 Jun 2020. Absent from master03 entirely.
- LIMIT/OFFSET added 1.1.0: amendment L80 SPECQUERY-16.
- RESULT_SET `meta._type` = "RESULTSET" is EXAMPLE-ONLY (ResultSetMetadata.yaml),
  schema declares `_type: string` with no enum.
- SM signatures: `SM/docs/UML/classes/i_query_service.adoc` §`=== I_QUERY_SERVICE
  Interface` — execute_stored_query(exec_spec: STORED_QUERY_EXECUTE_SPEC, row_offset,
  rows_to_fetch, ehr_ids: List<UUID>): RESULT_SET (L16); execute_ad_hoc_query(exec_spec:
  ADHOC_QUERY_EXECUTE_SPEC, ...): RESULT_SET (L39); both 0..1, error ehr_id_does_not_exist.
- ITS-REST operationIds: `query_execute_adhoc_query` / `query_execute_stored_query`
  (OAS spells "adhoc"; SM spells "ad_hoc"). 200_Query.yaml (able-to-execute) +
  400_Query.yaml (invalid input / invalid syntax) + 408 + (stored) 404.
