---
name: demographic-api-location
description: Where the ITS-REST 1.1.0 Demographic API (party CRUD x5 subtypes, versioned_party family, demographic contribution) requirements live — the DEVELOPMENT lifecycle statement, the exact-mirror-of-composition structure, RM demographic package + SM I_DEMOGRAPHIC_SERVICE/I_PARTY reference text, and the total CNF/Robot vacuum
metadata:
  type: reference
---

# Demographic API (ITS-REST 1.1.0) — file map

Companion to [[composition-crud-ops-location]], [[versioned-object-read-ops-location]],
[[contribution-ops-location]], [[its-rest-wire-contract-location]].

## Route inventory (`ITS-REST/specifications/demographic.openapi.yaml`)
`info.x-status: DEVELOPMENT`, `version: development` (the only API besides admin
with a non-STABLE status). `servers: https://{baseUrl}/v1`, `security: []`.
26 non-tag operations + 13 tag operations (tags = group 13):
- 5 identical CRUD quintets `person|agent|group|organisation|role`:
  POST `/demographic/<t>`, GET/PUT/DELETE `/demographic/<t>/{uid_based_id}`.
  **Verified byte-identical mod type-name** across all 5 (ops AND responses).
- `/demographic/versioned_party/{versioned_object_uid}` + `/revision_history`
  + `/version` + `/version/{version_uid}`.
- `/demographic/contribution` POST + `/{contribution_uid}` GET.

## Docs prose = STUB
`docs/demographic/Description.md` (24 lines: purpose / related docs / the
`DEVELOPMENT` status sentence). NO per-operation prose anywhere. Same stub
pattern as the EHR + Definition chapters. All cross-cutting rules come from
`docs/overview/{Requests_and_responses,Resources,Glossary_and_conventions}.md`.
The glossary types `uid_based_id`/`versioned_object_uid`/`version_uid`/
`version_at_time` API-generically — that IS the docs-text grounding for party.
**Nothing in the vendored ITS-REST text defines what `DEVELOPMENT` MEANS for
conformance** (the maturity ladder lives in the external openEHR release
strategy, not vendored).

## The structural law: party family == composition family
Party CRUD is an exact structural mirror of composition CRUD minus `ehr_id`:
same `uid_based_id` / `uid_based_id_as_versioned_object_uid` /
`uid_based_id_as_version_uid` path params, same Prefer/If-Match/Accept_LOCATABLE/
ContentType_LOCATABLE/openehr-item-tag headers, same 201/200/204/400/404/412/409/422
branch set, same `409_<T>_with_uid_based_id` DELETE gate (no If-Match on DELETE),
same generic `204_deleted_at_time` / `204_version_updated` / `204_version_deleted`.
versioned_party mirrors versioned_composition exactly, incl. the ETag asymmetry
(ETag_VERSION only on the at_time read, none on versioned_X or version-by-id) and
the ContentType_canonical-on-a-GET oddity. => Every group 3-8 adjudication
transfers verbatim; only the deltas below are demographic-specific.

## Demographic-specific deltas
- Generic `404.yaml`/`404_not_found_or_no_version_at_time.yaml` replace the
  ehr_id-scoped 404 files. **`<t>_create` declares a 404 with no path parameter
  to trigger it** (mirror slot of `404_unknown_ehr_id`) — trigger undefined.
- `demographic_contribution_create/get` use **Accept/ContentType_canonical**
  (json|xml only) — NO Simplified-Formats section in the description, unlike the
  EHR contribution ops (SPECITS-84 landed only on the EHR side). But they reuse
  the SHARED `200_CONTRIBUTION.yaml`, whose text DOES mention FLAT/STRUCTURED
  and whose Content-Type header is `ContentType_LOCATABLE` — internal OAS tension.
- Both demographic contribution ops carry `operationId: contribution_create` /
  `contribution_get` — **duplicated with the EHR ops** (OAS-shape defect).
- Own `schemas/demographic/{NewContribution,UpdateVersion,Version,
  OriginalVersion,ImportedVersion,UVersionable}.yaml` — literal copies of the
  `ehr/` ones with `data` retyped to `UVersionable` (= UParty union); each file
  carries the comment "copy of same schema from `ehr` … reminder to keep them in sync!".
- `PARTY_RELATIONSHIP` has NO route (only the inline `PARTY.relationships`
  attribute) — the `PARTY_RELATIONSHIP_schema` tag calls it a "resource" anyway.
  `VersionOfParty.yaml` + `SeePartyRelationship.yaml` are wired ONLY into
  `components.schemas` for doc display, never into an operation.

## RM grounding
- `RM/docs/demographic/master02-demographic_package.adoc` — §Party Identification
  ("via the `_uid_` attribute (type `OBJECT_VERSION_ID`) of the containing
  `VERSION`"), §Party Relationships (stored by value on the SOURCE party;
  source/target are OBJECT_REFs containing **HIER_OBJECT_IDs**, not
  OBJECT_VERSION_IDs), §Versioning Semantics ("Every Party is stored in its own
  Version container").
- `RM/docs/UML/classes/org.openehr.rm.demographic.*.adoc` — 13 classes.
  PARTY invariants incl. **`Uid_mandatory: uid /= Void`** (party uid is
  mandatory, unlike COMPOSITION) + `Is_archetype_root` + `Type_valid: type = name`
  + `Identities_valid`. PARTY_RELATIONSHIP `Target_valid` reads
  `not target.reverse_relationships.has(self)` — likely released-text defect
  (asymmetric with Source_valid; both also dereference a PARTY_REF).
- `RM/docs/common/master06-change_control_package.adoc` §Overview L7 — versioning
  applies to "'top-level structures', such as EHR Compositions and Party objects
  in a demographic system" => the same VERSION/VERSIONED_OBJECT law.
- BASE `org.openehr.base.base_types.party_ref.adoc` `Type_validity` enumerates
  PERSON|ORGANISATION|GROUP|AGENT|ROLE|PARTY|ACTOR.

## SM reference text (not an authority)
`SM/docs/openehr_platform/master06-demographic_service.adoc` includes
`UML/classes/{i_demographic_service,i_party,i_party_relationship,uv_party,
uv_party_relationship}.adoc`. `create_party`/`update_party`/
`create_party_relationship`/`update_party_relationship` all say "Causes
server-side creation of a new VERSIONED_OBJECT, ORIGINAL_VERSION and new
CONTRIBUTION" — the SM-side proof that a party write emits CONTRIBUTION+AUDIT.
**SM defects:** `i_party`/`i_party_relationship` factory params are UNTYPED;
`get_party_at_version` precondition calls `has_party_version(...)` but the
declared function is `has_party_version_id`; `i_demographic_service` preconditions (L21+L37 ONLY) call `valid_content`
while `i_validity_checker.adoc:22` declares `content_valid` (`i_party` /
`i_party_relationship` have NO such precondition at all, yet still declare the
`content_invalid` error); `create_party_relationship`
lists error `definition_unknown` with no `definitions_valid` precondition;
all ids typed `UUID` incl. the version ids; `delete_party` postcondition
`not has_party(...)` contradicts RM logical deletion. **SM has NO versioned-party
read ops and NO demographic-contribution interface** — the 4 versioned_party
routes + both contribution routes have zero SM counterpart.

## CNF = total vacuum
`CNF/docs/platform_test_schedule/master10-func_tc_demographic.adoc` is a pure
SKELETON: Test Environment TBD, Test Data Sets TBD, 12 SM-operation sections ×
2 cases each, EVERY case body literally "TBD", case names "aaaa"/"bbbb". It also
mis-attributes all 12 ops to `I_DEMOGRAPHIC_SERVICE` (10 belong to I_PARTY /
I_PARTY_RELATIONSHIP). **NO `I_DEMOGRAPHIC_SERVICE` Robot suite exists** under
`CNF/tests/platform/robot/` (the `demographic` grep hits are only
`external_ref.namespace: "demographic"` inside contribution fixtures).

## Register
Only demographic entry = **AMB-32** (PARTY_RELATIONSHIP has no wire,
`fixed_handling`, #1495). AMB-33 notes `physical_party_delete` /
`archive_parties` are also unwired.
