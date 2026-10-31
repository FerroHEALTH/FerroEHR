---
name: sm-query-service-chapter8-location
description: SM Platform ch.8 (Query service) map — the 2 calls, diagram-only return/association multiplicities, the INCOMPLETE SPECPR-292 row_offset rename (amendment-record evidence), the 5 rival qualified-name grammars, and the orphaned RESULT_QUERY_DESCRIPTOR
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.8 "Query Service" — navigation

The SM-side companion to [[query-api-ops-location]] (ITS-REST query wire) and
[[aql-result-set-equivalence-location]] (AQL row semantics); shares the
master02/master03 conventions catalogued in [[sm-ehr-service-chapter5-location]].

## File map
`SM/docs/openehr_platform/master08-query_service.adoc` = **38 lines**: §Overview
L5-23 (L11 formalism-agnostic claim, L13 the "parameters must be provided"
sentence, L15 the RESULT_SET description + **`item_offset`/`items_to_fetch`**,
L17-23 the stored-query id grammar block + worked example) + 6 `include::`.
Classes: `i_query_service` (2 calls), `stored_query_execute_spec`,
`adhoc_query_execute_spec`, `result_set`, `result_set_column`, `result_set_row`.
**`result_query_descriptor.adoc` is included by NO chapter** (only
`UML/class_index.adoc` links it) yet `result_set.adoc` L33 xrefs it → the
published body has a dangling `<<_result_query_descriptor_class,...>>`. It
`Inherit`s `query_descriptor.adoc`, which master04 (not master08) includes.
Second consumer of RESULT_SET: `openehr_sample.adoc` (ch.10 Subject Proxy).

## Diagram-only content — rasterizes legibly
`SM/docs/UML/diagrams/SM-platform.interface.query.svg` = 141 `<path>`, 0
`<text>`; `rsvg-convert -w 2600` legible. ONLY source for: `I_QUERY_SERVICE`
**inherits `I_STATUS`**; the return is **`RESULT_SET [0..1]`** (matching the
tables' leading column, contradicting L15's "the response IS a RESULT_SET");
`ehr_ids : UUID [*]` (tables say `List<UUID>[0..1]`); and the RESULT_SET
association bounds **`columns 1..*`**, `rows *`, `query 0..1` — the element-level
lower bound on columns exists NOWHERE else.

## The rename that never finished (hard evidence, not inference)
`master00-amendment_record.adoc` entry **0.9.5 / SPECPR-292, completed 28 Feb
2019**: "Rename `_row_offset_` and `_rows_to_fetch_` to `_item_offset_` and
`_items_to_fetch_`". master02 §List Handling and master08 L15 use the NEW names;
`i_query_service.adoc` still DECLARES the old ones — so the divergence is a
half-applied change request, and the old names carry semantics the new ones lack
("zero or negative" -> offset zero / 'all'; master02 defines only zero).
Neither location defines the ABSENT-parameter default.

## Five rival qualified-query-name grammars (all first-hand)
1. master08 L20 `reverse-domain-name '::' semantic-id [ '/' version ]` (ns
   mandatory, version inside the name after `/`).
2. `stored_query_execute_spec.adoc` L17 "`reverse_domain::name`" + a SEPARATE
   `version` [0..1] attr = "semver.org 3-part string", latest-if-absent.
3. master04 L25-26 `<namespace>::<query-name>` | `<ns>::<formalism>::<name>`,
   default ns `"misc"`, non-reverse-domain examples.
4. `query_descriptor.adoc` L17 `<namespace>::<query_name>` (underscore),
   example `ehr::all_over_50_women`.
5. ITS-REST `docs/query/Qualified_query_name.md`: `[{namespace}::]{query-name}`,
   ns OPTIONAL + reverse-domain is a SHOULD, charset `[a-zA-Z0-9_.-]`, `aql`
   reserved, PARTIAL semver prefix -> latest match.

## Other confirmed ch.8 defects/silences
- `ADHOC_QUERY_EXECUTE_SPEC.source` Meaning = "AQL text of query" while its own
  `formalism` (default `"aql"`) and master08 L11 claim formalism-agnosticism;
  `formalism` has an EMPTY Meaning cell.
- `query_parameters: Hash<String,String>` is **1..1 mandatory** on both exec
  specs (empty-hash-required for parameterless queries); values are String only
  (ITS-REST passes typed JSON).
- `result_set.adoc` L38 "Rox data" typo; `result_set_column.archetype_id`
  Meaning is a released-text editorial note ("NOTE: check on whether needed");
  `RESULT_SET.query` + `ADHOC...formalism` Meanings are blank.
- SM `RESULT_SET.id [1]` + `creation_time [1]` have NO ITS-REST counterpart
  (schemas/query/ResultSet.yaml requires only `rows`; `_created` lives in
  optional `meta`) despite amendment 0.9.6 claiming REST alignment.
- Negative-number conflict with AQL: `QUERY/docs/AQL/master03-syntax.adoc` L1137
  "`row_count` minimal value is 1, while minimal value for `offset` is 0" — and
  AQL never states how in-query LIMIT/OFFSET composes with service paging.
- CNF `master11-func_tc_querying.adoc` (108 L) is all-"xx": L46+L81 wrongly use
  `{i_demographic_service_link}` (defined only in master10 → renders a
  demographic link), L74 "Test Case bbbb / TBD", L58/L71 runner paths that do
  not exist in `CNF/tests/platform/robot/I_QUERY_SERVICE/`. Zero offset/fetch
  coverage in the robot suite.
