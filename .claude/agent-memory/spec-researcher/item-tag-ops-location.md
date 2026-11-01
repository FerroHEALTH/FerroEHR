---
name: item-tag-ops-location
description: Where the ITEM_TAG spec text lives — RM common master07-tags + item_tag class, the 7 EHR-side + 16 demographic-side ITS-REST tag ops/schemas/params, the header wrappers; and the total SM/CNF/ITS-XML silence
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# ITEM_TAG — where the spec text lives

## RM (the class authority)
- `RM/docs/common/master07-tags.adoc` — the `common.tags` package chapter
  (Overview + Semantics + include of the class). Package is a SIBLING of
  `change_control` (master06), NOT inside it. **Filename has NO `_package`
  suffix** (unlike every other Common chapter). The chapter is TINY: 19
  lines, 3 sections, ~2 paragraphs of own prose.
- **THE REAL TAG SEMANTICS ARE NOT IN CH.7.** They live in
  `RM/docs/ehr/master04-ehr_package.adoc` §Tags (L133-153): the four
  consequence bullets (added any time / not part of content / **no
  re-versioning** / distinct instance per use), the 13-instance example,
  "one logical list, no grouping", the AQL claim. Ch.7 §7.2 defers nothing
  and master04 defers TO ch.7 — a mutual-deferral loop where master04 is
  the only carrier.
- `RM/docs/ehr/diagrams/tags_example.svg` — draw.io export; labels are in
  `<foreignObject>` HTML, NOT `<text>` (the `<text>` nodes are truncated
  "COMPOSITION..." stubs + "Viewer does not support full SVG 1.1"). Extract
  foreignObject to read the 5 ITEM_TAG instances (all show `value = ""`,
  `owner_id` = the bare ehr_id).
- Ch.7's own UML diagram `RM-common.tags.svg` is `{uml_diagrams_uri}` =
  REMOTE — not vendored, unreadable.
- `RM/docs/common/master02-overview.adoc` enumerates archetyped/generic/
  directory/change_control/resource — **`tags` is missing** (SPECRM-87
  never updated it).
- `RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc` — the ONLY class
  table: key/value/target/target_path/owner_id + `Inv_key_valid`,
  `Inv_value_valid`.
- Added by SPECRM-87 (`RM/docs/common/master00-amendment_record.adoc`,
  issue row 2.1.5, 17 Nov 2022). Also in
  `RM/computable/BMM/openehr_rm_1.2.0.bmm.json` (~L1982).
- `is_justified` (used in Inv_key_valid) is **defined nowhere** — the BASE
  String class (`BASE/docs/UML/classes/org.openehr.base.foundation_types.string.adoc`)
  lists only is_empty/is_integer/as_integer/append/less_than/contains.
  The intent lives only in the attribute Meaning column.

## ITS-REST (the wire)
- Routes in `ITS-REST/specifications/ehr.openapi.yaml` L95–113 (5 EHR-side
  paths / 7 ops); tag group `ITEM_TAG` L148.
- Ops: `operations/{ehr,composition,ehr_status}_tags_{get,update,delete}.yaml`
  (+ 15 demographic ones: person/role/group/organisation/agent).
- Schemas: `schemas/common/ItemTag.yaml` (base, `additionalProperties: false`),
  `schemas/common/UpdateItemTag.yaml` (the PUT item — key/value/target_path
  ONLY), `schemas/ehr/ItemTagOf{Composition,EhrStatus}.yaml`
  (allOf ItemTag + example only).
- Responses: `responses/200_{COMPOSITION,EHR_STATUS}_ItemTagList_{retrieved,
  updated}.yaml`, `204_updated.yaml`, `204_deleted.yaml`,
  `404_unknown_ehr_id[_or_uid_based_id[_or_key]].yaml`.
- Params: `parameters/path/key.yaml`, `parameters/query/tag_{key,value,
  target_path}.yaml`, `parameters/path/uid_based_id.yaml`.
- Header wrappers: prose in
  `ITS-REST/specifications/docs/overview/Requests_and_responses.md`
  §"openehr-item-tag and openehr-version-item-tag" (L98–126);
  request param files `parameters/header/openehr-*item-tag.yaml`,
  response header files `headers/openehr-*item-tag.yaml`.
- There is NO tag prose in `docs/ehr/Description.md` (it is a stub — see
  [[ehr-status-ops-location]]).

## The DEMOGRAPHIC tag half (group 13, `demographic.openapi.yaml` L93–135)
- **16** path-method pairs (NOT 13): `GET /demographic/tags` (1) + per party
  subtype `person|agent|group|organisation|role` × {GET,PUT `/…/{uid_based_id}/
  tags`, DELETE `/…/tags/{key}`} = 15.
- Ops `{person,agent,group,organisation,role}_tags_{get,update,delete}.yaml`
  are **byte-identical mod type-name across all 5 subtypes** (verified by
  sed-normalized diff); so are the 10 `200_<T>_ItemTagList_{retrieved,updated}
  .yaml` responses and the 5 `schemas/demographic/ItemTagOf<T>.yaml`.
- Structural mirror of the EHR twin **minus `ehr_id`**: `person_tags_get` ≡
  `composition_tags_get`, `_update` ≡ `composition_tags_update`, `_delete` ≡
  `composition_tags_delete`, `demographic_tags_get` ≡ `ehr_tags_get`.
  404 files swap to `404_unknown_uid_based_id[_or_key].yaml`.
- Deltas worth remembering: `demographic_tags_get` has **no scope param at
  all** (server-wide list) yet its description still says "within given EHR",
  and it declares the PERSON-specific list schema; `ItemTagOf<Party>` examples
  use `owner_id.type: SYSTEM` (EHR side uses `EHR`) and `target_path: ""`;
  `person_tags_update` says "VERSIONED_OBJECT.uid.value" where get/delete say
  "VERSIONED_PARTY.uid.value"; all 5 `<t>_update.yaml` declare ONLY
  `openehr-version-item-tag` (missing `openehr-item-tag`) though their prose
  names both — the composition/ehr_status updates declare both.
- **RM has NO demographic-side tag anchor**: `EHR.tags` (`RM/docs/UML/classes/
  org.openehr.rm.ehr.ehr.adoc` L53-55, prose-only "Tag `_target_` values can
  only be within the same EHR", NO `Tags_valid` invariant) is the only
  containment; grep of `RM/docs/demographic/` + all `org.openehr.rm.
  demographic.*.adoc` = zero tag hits. RM `ITEM_TAG.target` is a plain
  `UID_BASED_ID` (OAS wraps it in `UObjectRefOfUidBasedId` — RM wins),
  target types unrestricted, `owner_id` "such as EHR" (open list).

## Ch.7-audit facts worth not re-deriving
- **`EHR.tags` is `List<OBJECT_REF>`, NOT `List<ITEM_TAG>`** (class table
  L53-55 + BMM `P_BMM_CONTAINER_PROPERTY` type `OBJECT_REF`), yet the
  master04 diagram draws ITEM_TAG *instances* inline, and EHR has a
  `<X>_valid` type invariant for EVERY other ref-typed attribute except
  `tags`. Containment is genuinely unresolved in the released model.
- `is_justified` (Inv_key_valid) is defined in NO vendored file — grep over
  BASE + RM returns only the two use sites (class table L36 + BMM L1982).
- **Uniqueness is a WIRE-ONLY rule**: `(key, target_path)` per target comes
  from ITS-REST `Requests_and_responses.md` L114 + 10 op descriptions. RM
  has zero uniqueness invariant.
- **QUERY/AQL is 100% silent on tags** (grep of the whole QUERY component =
  zero hits) although RM ch.7 §7.2 and ehr master04 L153 both claim direct
  AQL support.
- `ehr_tags_get` (EHR-wide list) declares the **COMPOSITION-specific**
  response schema; `demographic_tags_get` declares the PERSON one and says
  "within given EHR". Both are released-text defects.
- FOLDER: docs text L100 names FOLDER as a taggable resource, but there is
  NO folder/directory tags endpoint and NO directory op carries a tag
  header.
- Header asymmetry: the 5 demographic `*_update` ops declare ONLY
  `openehr-version-item-tag` while their `*_create` twins and
  composition/ehr_status update declare BOTH — though every prose block
  names both.

## Total silences (verified by grep across the whole vendored tree)
- **SM**: zero ITEM_TAG / tag-operation anchor anywhere in `SM/docs/`
  (only unrelated "tagged String values" / "language tag" hits). No
  I_* interface owns these routes.
- **CNF**: zero coverage — no schedule row, no Robot suite (the one grep hit
  is an unrelated archetype element named `tags` in a FLAT fixture).
- **ITS-XML**: no ITEM_TAG type at all, although the tag ops declare
  `Accept: application/xml`.
- The `ehr.openapi.yaml` ITEM_TAG tag description links RM
  **/development/** (every other class links /latest/) — ITEM_TAG is in no
  released RM.

Related: [[its-rest-wire-contract-location]],
[[composition-crud-ops-location]], [[ehr-status-ops-location]],
[[directory-api-location]].

## Wire-branch census (re-verified first-hand 2026-08-21)
- **7 TYPED tag families** (composition, ehr_status + the 5 party subtypes) x
  {GET, PUT, DELETE} + the 2 space-wide GETs (`ehr_tags_get`,
  `demographic_tags_get`) = 23 tag ops. The typed families are byte-identical
  modulo the type name (sed-normalized diffs clean).
- **All 7 tag PUTs declare exactly `200`/`204`/`400`/`404` — no `422`**, while
  the 5 sibling PARTY updates (`person_update.yaml` etc.) on the same resources
  DO declare `422`. The overview's 422 row is
  `docs/overview/Requests_and_responses.md` **L233** ("The request was
  well-formed but was unable to be followed due to semantic errors"); the 422
  body text is `responses/422.yaml`; `responses/400.yaml` covers only
  parse/syntax failure.
- 404 text is the ONLY not-found trigger and says nothing about resource KIND:
  `404_unknown_ehr_id_or_uid_based_id{,_or_key}.yaml` (EHR side) /
  `404_unknown_uid_based_id{,_or_key}.yaml` (demographic side) = "when the
  `uid_based_id` does not exist". `parameters/path/uid_based_id.yaml` describes
  only the two FORMS (OBJECT_VERSION_ID / HIER_OBJECT_ID), never the kind.
  Grep for wrong-type/mismatch wording across `ITS-REST/specifications/` = zero.
- The `Prefer` identifier contract lives at
  `docs/overview/Requests_and_responses.md` **L299-311** (§Prefer only
  identifier): "the status will be `201 Created` or `200 OK`, never `204 No
  Content`" + "the response body will be a single JSON object with a single
  `uid` attribute". `parameters/header/Prefer.yaml` (enum representation/
  minimal/identifier, default minimal) IS declared on every tag PUT — and
  ITEM_TAG has no `uid` attribute, so the branch is unsatisfiable.
- The `(key, target_path)` identity sentence is
  `Requests_and_responses.md` **L114** (§openehr-item-tag…), repeated in 10 op
  descriptions. **No `uniqueItems`** anywhere on the tag surface (grep of the
  tag ops + `ItemTag.yaml` + `UpdateItemTag.yaml` = zero), and no merge rule.
- Filter grammar: both space-wide GETs `$ref` the SAME three
  `parameters/query/tag_{key,value,target_path}.yaml` — each is a bare
  `type: string` with an `example` and **no `description`** — and both repeat
  verbatim "This list can be filtered by the given one or more `tag_key`,
  `tag_value`, `tag_target_path` query parameters." (`explode: true`,
  `style: form`). No combination/match-mode/absent-path rule.
- `target_path` example census: **6 of 7** `ItemTagOf<T>` schemas carry
  `target_path: ""` (EHR_STATUS + Person/Agent/Group/Organisation/Role); only
  `ItemTagOfComposition.yaml` uses a real path
  (`/context/start_time/value`); NONE omits the attribute. All 7 targets are
  `_type: HIER_OBJECT_ID` — there is **no VERSION-targeted example** — and the
  COMPOSITION one sets `type: COMPOSITION` on a HIER_OBJECT_ID (should be the
  VERSIONED_COMPOSITION container).
- Editorial residue confirmed: `responses/204_deleted.yaml` says "(logically)
  deleted" (the only "logically" in ITS-REST); all 7
  `200_<T>_ItemTagList_updated.yaml` are BYTE-IDENTICAL to their `_retrieved`
  siblings and say "successfully retrieved" on a PUT; the DELETE descriptions
  say "identified by `tag_key`" while the parameter file is
  `parameters/path/key.yaml` (name `key`); party `_tags_get`/`_tags_delete` say
  "VERSIONED_PARTY.uid.value" while `_tags_update` says
  "VERSIONED_OBJECT.uid.value" for the resource its own next line calls "the
  target VERSIONED_PARTY container"; `person_create.yaml` declares BOTH
  item-tag header params while `person_update.yaml` declares only
  `openehr-version-item-tag` though its prose (L12) names both;
  `ehr.openapi.yaml` L150 links ITEM_TAG to RM **/development/**.
- No paging/ordering parameter on ANY of the 9 tag GETs (only ehr_id/
  uid_based_id/tag_* filters + `Accept_canonical`); `Accept_canonical.yaml`
  offers `application/xml` while **zero of the 157 vendored XSDs mention
  ITEM_TAG**; `Requests_and_responses.md` L116 ("If the server does not support
  ITEM_TAGs, these headers will also be unsupported") assigns no status; RM
  `master07-tags.adoc` is 19 lines with no lifecycle rule and
  `operations/composition_delete.yaml` never mentions tags.

## The `-codegen` OAS bundle carries defects the decomposed tree does NOT (2026-08-21)
- `docs/specs/openehr/ITS-REST/specifications/schemas/common/ItemTag.yaml` is
  `additionalProperties: false` with NO `_type` property **and NO discriminator**
  (the only 17 discriminator-bearing schemas in the decomposed tree are the `U*`
  union wrappers). The `discriminator: propertyName: _type / mapping: ITEM_TAG`
  that contradicts `additionalProperties: false` lives in the published BUNDLES:
  `crates/openehr-its/vendor/rest-oas/ehr-codegen.openapi.yaml` L3188-3191 and
  `demographic-codegen.openapi.yaml` (~L3155). The `-html` variant has neither;
  the `-validation` variant has no `ItemTag` schema at all. **Always name the
  bundle, not the decomposed file, when reporting a discriminator defect.**
- Header declaration split: `headers/openehr-item-tag.yaml` (RESPONSE) = bare
  `schema: type: string`, no example, no pattern; `parameters/header/openehr-item-tag.yaml`
  (REQUEST) = `type: array` of `UpdateItemTag` with `style: simple, explode: true`
  and the `key="…",value="…"; …` example. Neither carries a `pattern`.
- **The 13 ops carrying the request header** are composition_create/update,
  ehr_status_update and {person,agent,group,organisation,role}_{create,update}.
  `directory_create`/`directory_update` do NOT (zero tag mention), despite
  `Requests_and_responses.md` L100 naming FOLDER as a taggable resource.
