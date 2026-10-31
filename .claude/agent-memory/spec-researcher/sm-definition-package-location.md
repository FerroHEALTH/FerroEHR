---
name: sm-definition-package-location
description: Where SM Platform ch.4 (Definition package) reference text lives — the 38 interface calls across 3 files, the diagram-only I_STATUS inheritance, the master02/03 inherited conventions, and the confirmed defect set (AOM2 mistyping, 3 incompatible qualified-name grammars, missing get_query)
metadata:
  type: reference
---

Reference text only: the Service Model is not an authority (owner ruling
2026-10-27); it grounds no verdict and no feature.

# SM Platform ch.4 — Definition package: file map

Companion to [[definition-artefact-delete-ownership]] (ownership of DELETE),
[[adl14-aom14-validity-location]] (the ADL14 op list), [[query-api-ops-location]]
(the ITS-REST/query-execution side), [[adl2-rest-wire-contract-location]].

## The chapter is 62 lines of prose + 5 `include::` pulls
`SM/docs/openehr_platform/master04-definition_package.adoc`:
- L3 `== Overview` (2 sentences; L11 = the artefact-kind scope sentence:
  "archetypes, templates, queries, and query sets", "other than terminology")
- L13 `== Archetypes and Templates` (L15 upload/updating/**removal**; L17 ADL2
  kind-collapse + "ARCHETYPE_HRID and a UUID"; L19 ADL1.4 OPTs "are XML
  artefacts", ARCHETYPE_ID vs UUID)
- L21 `== Registered Queries` (L25-26 the 2 qualified-name schemes; L34 default
  namespace `"misc"`) + L36 `=== Query Formalism` (`_a_type_` case-insensitive,
  `::` version, default major "1", "AQL"="aql"="AQL::1")
- L51 `== Class Definitions` = includes of i_definition_adl2, i_definition_adl14,
  i_definition_query, definition_call_status_type, query_descriptor.
**38 calls total**: ADL2 14, ADL14 16, QUERY 8. Only these 5 class files.

## Structure that exists ONLY in the diagram (tables have no `Inherit` row)
`SM/docs/UML/diagrams/SM-platform.interface.definitions.svg` is **path-text
(0 `<text>` elements)** — rasterize to read it (`rsvg-convert -z 1.6`). It shows
(a) all three I_DEFINITION_* inheriting **I_STATUS**, (b)
DEFINITION_CALL_STATUS_TYPE specialising CALL_STATUS_TYPE, (c) list returns as
`[*]`. NO `i_*.adoc` file in the whole SM tree carries an `Inherit` row
(`grep -n Inherit i_*.adoc` = empty) — so interface inheritance is
diagram-only.

## Inherited conventions live in master02/master03, not ch.4
- `master02-overview.adoc` §List Handling (L147-152) = the ONLY definition of
  `item_offset` (0 = from first) / `items_to_fetch` (0 = all). **Negative values
  undefined here**; `i_query_service.adoc` defines them for its own
  `row_offset`/`rows_to_fetch` ("zero or negative"). §Anatomy (L62-79) defines a
  Call/Arguments/Pre/Post/**Exceptions** table format the class files do NOT use
  (they use a UML export with an undocumented leading multiplicity column and
  `.Errors` blocks). §Interface Calls L60 = the transactional-equivalence rule;
  §Functional Style L109 = authn/authz out of band, L111 = last_call_failed/
  last_call_status.
- `master03-common_package.adoc` §Representing Call Status L17 = services extend
  CALL_STATUS_TYPE by inheritance (the licence for DEFINITION_CALL_STATUS_TYPE).

## Confirmed released-text defects (all first-hand verified)
1. ADL14 interface types its params as **AOM2** classes (`AOM2.html#_archetype_class`)
   for ADL 1.4 artefacts; `valid_opt(an_opt: ARCHETYPE)` for an artefact the
   chapter calls XML. CNF master04 §Normative Reference names AOM 1.4.
2. `an_arch.identifier` in both upload postconditions — **no `identifier`
   attribute exists in AOM2**; it is `ARCHETYPE.archetype_id: ARCHETYPE_HRID`
   (`AM/docs/UML/classes/org.openehr.am.aom2.archetype.adoc` L21). For ADL14 it
   is also type-incoherent (HRID vs the ARCHETYPE_ID the ops key on).
3. `delete_archetype` Pre = `has_artefact(an_id)` — a function of the OTHER
   interface (ADL14 has `has_archetype`).
4. `list_matching_opts` returns `List<ARCHETYPE_ID>` while `list_opts` returns
   `List<UUID>` and §4.2 says OPTs are UUID-identified.
5. `store_query` Pre = `is_valid_query(a_query_text)` — wrong name AND arity
   (declared `valid_query(a_query_text, a_type)`).
6. `upload_artefact` error `invalid artefact` (space) vs enum `invalid_artefact`.
7. Delete errors are `invalid_archetype`/`invalid_template`/`invalid_query`, not
   the `*_does_not_exist` codes; `template_does_not_exist` is used by NO call.
8. `list_templates`/`list_opts` Meanings both start "List all archetypes".
9. QUERY_DESCRIPTOR has NO unique-identifier attribute though its own
   Description and `store_query`'s Meaning both promise one.
10. **No `get_query`** exists (text only reachable via QUERY_DESCRIPTOR.source
    on the list ops); no version parameter on any I_DEFINITION_QUERY call.
11. `store_query_set` carries a released-text "TODO: determine details." and has
    no has/list/delete/count siblings.

## Three incompatible qualified-query-name grammars in ONE spec
master04 L25-26 `<ns>::<name>` | `<ns>::<formalism>::<name>` (ns examples
"ehr"/"misc", NOT reverse-domain) vs `master08-query_service.adoc` L20
`reverse-domain-name '::' semantic-id [ '/' version ]` vs
`stored_query_execute_spec.adoc` L17 "`reverse_domain::name`" + separate
`version` attribute. Also master08 L15 names the paging params
`item_offset`/`items_to_fetch` while `i_query_service.adoc` declares
`row_offset`/`rows_to_fetch`.

## CNF coverage of this chapter (guide only, per owner ruling)
`CNF/docs/platform_test_schedule/master04-func_tc_definition_adl.adoc` — 5
sections, **all ADL14 OPT** (validate_opt/upload_opt/get_opt/get_opts/
delete_opt); names `validate_opt`/`get_opts` which the SM does NOT declare;
keys OPTs by **`template_id` + a version parameter** (SM keys by UUID, models no
version) and asserts conflict-on-duplicate where SM ADL2 says "replace".
L325 = the only privilege sentence (admin for PHYSICAL OPT delete).
`master05-func_tc_definition_query.adoc` = a 100% skeleton: 7 case ids for
has_query/valid_query/list_queries, every Description/Pre/Post/Flow is "xx",
data sets TBD. No ADL2 case anywhere.
