---
name: query-api-ops-location
description: Where the 6 ITS-REST Query API execution operations are specified (dedicated released prose + decomposed OAS + SM I_QUERY_SERVICE) and the confirmed gaps/defects in that text
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# Query API (6 execution ops) — location map

Complements [[aql-result-set-equivalence-location]] (AQL semantics + RESULT_SET
shape), [[its-rest-wire-contract-location]] (cross-cutting wire rules) and
[[sm-query-service-chapter8-location]] (the SM ch.8 side: class map, diagram-only
multiplicities, the 5 rival name grammars, the unfinished SPECPR-292 rename).

**The Query API is one of the few API groups with DEDICATED released prose** —
unlike the EHR API whose docs dir is an overview-only stub (see
[[ehr-status-ops-location]]). Prose is the oracle; OAS is shapes only.

`docs/specs/openehr/ITS-REST/specifications/`:
- `docs/query/Request.md` — THE parameter contract. §GET vs POST (POST
  recommended, URI-length), §Common Headers and Query Parameters (L22 "All query
  execution requests SHOULD support at least the following parameters" —
  `ehr_id`/`offset`/`fetch`/`query_parameters`, **no GET/POST distinction** —
  plus "Related request headers: `openehr-ehr-id`" + "Related response headers:
  `ETag`"; L25 the `fetch` bullet whose ONLY exclusion clause links AQL
  `#_top`), §About the ehr_id parameter L35-39 (the MUST NOT for population
  queries; "MAY be used by the underlying backend to perform routing,
  optimizations or similar" = routing-hint wording ONLY, no filter/predicate/
  conflict rule), §Query parameters (the named-binding law: "SHOULD NOT be
  prefixed with `$`", two worked GET examples).
- `docs/query/Qualified_query_name.md` — the FULL name grammar
  (`[{namespace}::]{query-name}`, reverse-domain namespace, `[a-zA-Z0-9_.-]`,
  the `aql` reserved-name NOTE from SPECITS-46) + the SEMVER partial-prefix
  latest-match rule (one sentence, covers both omitted and partial version).
  **The ONLY "reserved" statement anywhere in ITS-REST** — nothing reserves the
  protocol names `q`/`ehr_id`/`offset`/`fetch` from `$parameter` binding.
- `docs/query/Query_types.md` — single-EHR / population / stored / ad-hoc taxonomy
  (§Single EHR queries says a query is executed "within a specific EHR" — a
  purpose statement, NOT a mechanic).
- `docs/query/Response.md` — 14 lines, NO normative content: two
  `<SchemaDefinition>` refs (ResultSet, ResultSetMetadata) + "metadata comprise
  a set of optional (implementation dependent) attributes, useful for debugging".
  All RESULT_SET requiredness therefore comes from the SCHEMAS, not prose.
- `docs/query/Description.md` — STABLE status marker only.
- `query.openapi.yaml` — the 3 paths → 6 operations (GET+POST each).
- `operations/query_execute_{adhoc_query,adhoc_query_body,stored_query,
  stored_query_body,stored_query_version,stored_query_version_body}.yaml`.
  The 3 POST (`*_body`) ops declare **ZERO query parameters** (headers + path
  only); the 3 GETs declare all of q/ehr_id/offset/fetch/query_parameters.
- `responses/{200,400,404,404_Query_version,408}_Query.yaml`;
  `headers/ETag_RESULT_SET.yaml` (example `W/"<uuid>"`).
- `parameters/query/{q,ehr_id_Query,offset,fetch,query_parameters}.yaml`,
  `parameters/path/{qualified_query_name,version}.yaml`,
  `parameters/header/{Accept_JSON,ContentType_JSON}.yaml`.
- `schemas/query/{AQL,AdhocQueryExecute,Query,QueryName,QueryVersion,QueryType,
  QueryParameters,Offset,Fetch,StoredQuery,ResultSet*}.yaml`.
- Stored-query DEFINITION side (store/list/get, needed for resolution +
  round-trip): `operations/definition_query_*.yaml`,
  `responses/200_StoredQuery_{get,stored}.yaml`, `200_QueryList.yaml`,
  `409_StoredQuery_version.yaml`, `headers/Location_Query.yaml`.
  **`definition_query_version_get.yaml` (the stored-query READ) declares ONLY
  `200` + `404_Query_version` — no 400 branch at all**, so a malformed
  `{version}`/out-of-class name has no declared outcome on the read.

**SM**: `SM/docs/openehr_platform/master08-query_service.adoc` (§Overview holds
the SM stored-query id form `reverse-domain-name '::' semantic-id [ '/' version ]`)
+ `SM/docs/UML/classes/{i_query_service,stored_query_execute_spec,
adhoc_query_execute_spec,result_set,result_set_column,result_set_row,
result_query_descriptor,query_descriptor}.adoc`.

**AQL binding**: `QUERY/docs/AQL/master03-syntax.adoc` §Reserved words (L15),
§Parameters + §Syntax (L97/L104 — `$name`, letters/digits/underscores, not a
reserved word), **L113 the parameter QUOTING NOTE** ("strings, dates, times and
datetimes should be quoted, numbers and booleans are not quoted" → parameters
are TYPED, against SM's `Hash<String,String>`), §Parameter Resolution (L132 —
"no specific guidelines"), §TOP L1070-1075 (deprecated as of Release 1.1.0, not
combinable with LIMIT), §LIMIT L1115-1142 (L1124 NOTE + L1125 the TOP
exclusion; L1137 `row_count` min 1 / `offset` min 0),
§ORDER BY (L1094, L1098 the undefined-default-ordering NOTE).
`master04-result_structure.adoc` (raw = Array<Array<Any>>; annotated result
sets delegated to SM/REST). LIMIT/OFFSET arrived WITH the TOP deprecation:
`master00-amendment_record.adoc` L80 SPECQUERY-16 "Add support for `LIMIT` and
`OFFSET`; deprecate `TOP`".

**The AQL ANTLR grammar IS vendored** (re-verified 2026-08-21 — any
"grammar files not vendored" claim is STALE): `QUERY/docs/AQL/grammar/
AqlParser.g4` (198 L) + `AqlLexer.g4` (325 L), `include::`d by
`QUERY/docs/AQL/master07-grammar.adoc` and linked from `QUERY/docs/index.adoc`
L56/L59. Pulled in by repo commit `c6885c6b0`. (The AM/ADL2 grammar gap in
[[adl2-parser-spec-location]] is a DIFFERENT component — do not carry it over.)

**CONFIRMED GAPS / DEFECTS (verified first-hand 2026-07-27, re-verified 2026-08-21):**
- `openehr-ehr-id` has NO parameter file in `parameters/header/` and is declared
  on NO operation — it exists ONLY in Request.md prose (and the overview
  deprecated-header table `openEHR-EHR-id` → `openehr-ehr-id`).
- `schemas/query/Query.yaml` (stored-query POST body) declares
  `required: [offset, fetch, query_parameters]` — all three REQUIRED, and it has
  no `q`. Contradicts the prose defaults (offset default 0, fetch
  implementation-dependent). `AdhocQueryExecute.yaml` requires only `q`.
- POST ops declare NO `ehr_id` anywhere (not a param, not in either body schema),
  and no released sentence assigns URL-vs-body precedence for offset/fetch/
  query_parameters (grep for precedence/overrid* in `docs/` = zero hits).
- `responses/400_Query.yaml`/`404_Query*`/`408_Query.yaml` declare NO body
  (unlike the generic `responses/400.yaml` which refs `schemas/others/Error.yaml`).
- `Accept_JSON`/`ContentType_JSON` enums = `application/json` only → the Query
  API OAS binds JSON only, while overview `Resources.md` L67 says services "MUST
  support at least one of the openEHR XML or JSON canonical formats" (an
  XML-only server therefore cannot serve this API).
  The QUERY RESULT_SET XSD **is** vendored — but only in the openehr-its bundle
  (`crates/openehr-its/schemas/xml/{components,its-xml-2.0.0-nsv2}/QUERY/latest/
  Query.xsd`, 180 L, header `<!-- Status: Draft -->`, EhrScape-derived
  RESPONSE_METADATA); `docs/specs/openehr/ITS-XML/components/QUERY/latest/`
  holds ONLY `README.adoc` ("RESULT_SET XML schema - draft").
- Request.md's headline example AQL ends `ORDER BY temperature DESC FETCH 3` —
  `FETCH` is NOT an AQL construct (grep "FETCH" across `QUERY/` = zero hits,
  still zero with the grammar vendored); the same example's node predicate
  `items[at0.63 and name/value='Symptoms']` is malformed. Released-example defect.
- The release's own examples COLLIDE on `ehr_id`: `schemas/query/
  QueryParameters.yaml` example binds `ehr_id: '7d44b88c-…'` as an ordinary
  query parameter while `StoredQuery.yaml`'s example AQL declares `$ehr_id` —
  and Request.md defines `ehr_id` as the protocol scope parameter.
- `operations/definition_query_store.yaml` + `definition_query_version_store.yaml`
  have `operationId:` values ending in `.yaml` (exactly TWO such ops).
- Glossary_and_conventions.md (19 L) defines NO query term (`q`, `offset`,
  `fetch`, `qualified_query_name`, query `version`, RESULT_SET all absent).
- CNF: `master11-func_tc_querying.adoc` is a stub (all "xx"/TBD); L46 + L81 use
  `{i_demographic_service_link}` — defined ONLY in `master10-func_tc_
  demographic.adoc` L4 — so both `I_QUERY_SERVICE` references render a
  demographic-service link. The Robot suite
  `CNF/tests/platform/robot/I_QUERY_SERVICE/` implements only
  `POST /query/aql`; all eight GET stored-query keywords in
  `_resources/keywords/aql_query_keywords.robot` (L268-296) are `No Operation`.
- SM↔ITS-REST conflicts: SM `result_set.adoc` requires `columns 1..1`/`id`/
  `creation_time` with `rows 0..1` vs `schemas/query/ResultSet.yaml`
  `required: [rows]` — **requiredness inverted in both directions**; SM
  `RESULT_SET_ROW` wraps cells in a `values: List<Any>` attribute vs the wire's
  bare `type: array` `ResultSetRow.yaml`; SM `RESULT_SET.query:
  RESULT_QUERY_DESCRIPTOR` (inherits QUERY_DESCRIPTOR: version, formalism,
  registration_time) is FLATTENED on the wire to `name` + `q` only — and
  `ResultSetMetadata.yaml` adds none of them back, so **the executed stored
  version is not observable**; SM namespace mandatory (`reverse_domain::name`)
  + `/version` separator vs REST optional namespace + path segment; SM version
  "3-part semver" (exact-only) vs REST partial prefix; SM
  `query_parameters: Hash<String,String>` **1..1 mandatory** vs REST typed JSON;
  SM `ehr_ids: List<UUID>` vs REST single scalar `ehr_id` (nothing realizes the
  list); SM `rows_to_fetch` "zero or negative means all" / `row_offset` "zero or
  negative means offset of zero" — no REST equivalent, and the SM's own
  amendment record (0.9.5/SPECPR-292, 28 Feb 2019) says those two names were
  renamed to `item_offset`/`items_to_fetch`, which master02+master08 use but the
  class file never adopted; SM error `ehr_id_does_not_exist` (declared on BOTH
  `execute_stored_query` and `execute_ad_hoc_query`) has NO REST realization —
  the ad-hoc ops declare no 404 at all and the stored 404s trigger only on the
  query name/version.
- Editorial residue in the SM query material: `result_set.adoc` `rows` Meaning is
  "Rox data."; `RESULT_SET.query` + `ADHOC_QUERY_EXECUTE_SPEC.formalism`
  Meanings are EMPTY; `result_set_column.archetype_id` Meaning is a review note
  ("NOTE: check on whether needed or inside the path"); `i_definition_query.adoc`
  `store_query_set` Meaning carries a left-in "TODO: determine details."
