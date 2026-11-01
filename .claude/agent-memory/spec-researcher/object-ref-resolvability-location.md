---
name: object-ref-resolvability-location
description: Where (and whether) the released specs say an OBJECT_REF / LINK / tag target must resolve to an existing object — the cross-component silence map, plus the CONTRIBUTION.versions parallel and the CNF fixture that commits a dangling uid
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

OBJECT_REF **resolvability** (does a committed reference have to point at something that
exists?) — the cross-component navigation map. Short answer the text supports: NO released
clause requires resolvability anywhere; the only scope constraint that exists is on
`EHR.tags`.

Where the load-bearing sentences live:

- **BASE, the general statement** —
  `BASE/docs/UML/classes/org.openehr.base.base_types.object_ref.adoc` Description:
  "a reference to another object, which may exist locally or be maintained outside the
  current namespace, e.g. in another service". Same class's `namespace` Meaning enumerates
  the legal values `"local"` / `"unknown"` / the regex — so an unresolvable/foreign
  namespace is explicitly legal. Prose companion:
  `BASE/docs/base_types/master05-identification_package.adoc §References` (L183-185, the
  primary-key/foreign-key analogy, "distributed referencing"); architecture framing in
  `BASE/docs/architecture_overview/master09-identification.adoc §Levels of Identification`
  (L51-93). **None of these state a resolution obligation.**
- **RM, per-holder attribute meanings + invariants** (no invariant anywhere constrains
  EXISTENCE):
  - `RM/docs/UML/classes/org.openehr.rm.ehr.ehr.adoc` — SEVEN invariants:
    Contributions_valid / Ehr_access_valid / Ehr_status_valid / Compositions_valid /
    Directory_valid / Folders_valid are `.type.is_equal("<TYPE>")` checks, **but
    `Directory_in_folders` is NOT a type check** (`folders.item(1) = directory`, a positional
    identity rule). So the accurate phrasing is "no EHR reference-list invariant constrains
    existence" — NOT "every one constrains `.type` only" (that over-generalization is
    falsified by Directory_in_folders; corrected 2026-08-21).
    Its `tags` Meaning is the ONE scope rule in the RM: "Tag `_target_` values can only be
    within the same EHR" (nothing equivalent for folders/compositions/directory).
  - `org.openehr.rm.common.folder.adoc` — `items` = "references to other (usually)
    versioned objects logically in this folder"; **entirely empty invariant section**.
  - **`org.openehr.rm.common.contribution.adoc` — the SECOND reference-holding class with an
    entirely empty invariant section**: `versions: List<OBJECT_REF>` at **1..1** ("Set of
    references to Versions causing changes to this EHR"), zero invariants. Cite it beside
    FOLDER: a "FOLDER is the only reference-holding class with no invariants" claim is only
    defensible under the RM's own narrow sense of *top-level* (= versioned top-level
    structures, per `RM/docs/common/master03-archetyped_package.adoc` L41 +
    `master06-change_control_package.adoc` L7, "COMPOSITION, EHR_STATUS, PARTY etc").
  - `org.openehr.rm.common.link.adoc` — LINK.type doc explicitly contemplates links
    "which must be followed and which can be broken when the extract is created" — the RM's
    clearest admission that a reference may not resolve.
  - `org.openehr.rm.common.item_tag.adoc` — only Inv_key_valid / Inv_value_valid.
  - Full census method: `grep -rln OBJECT_REF RM/docs/UML/classes/*.adoc` then count
    `^h|\*Invariants\*` per file → zero-invariant hits are contribution, folder, care_entry
    (`guideline_id`), and 4 ehr_extract classes.
- **RM, the nearest thing to a general integrity clause** —
  `RM/docs/ehr/master04-ehr_package.adoc §Change Control in the EHR` L159-163: "the record
  should always be in a consistent informational state" (vague, non-operationalized).
  §Folders L110: folders "may be _added or removed contemporaneously or subsequently_ to the
  committal of the referenced Compositions"; same-Contribution grouping is "normally", not
  a requirement.
- **RM, EHR Extract** — `RM/docs/ehr_extract/master09-semantics.adoc §Creation Semantics`
  L76-77: on export, "for each instance of `OBJECT_REF` encountered … obtain the target of
  the reference from the relevant service" and rewrite `_namespace_` = "local". This is the
  only algorithm anywhere that assumes resolvability, and it is an EXTRACT-BUILD step, not a
  commit-time rule.
- **SM** — `SM/docs/UML/classes/i_validity_checker.adoc` is the whole commit-time validation
  vocabulary: `definitions_valid` (archetype/template ids) + `content_valid` ("Return True if
  the content structure is a valid instance of the relevant RM classes"). Neither mentions
  references. `i_ehr_directory.adoc` create_directory (L47) + update_directory (L97) carry
  `Pre_content_valid`. See [[directory-api-location]] for the `valid_content` vs
  `content_valid` naming defect (**exact scope: 6 sites / 3 files** —
  i_ehr_composition:104,125, i_ehr_directory:47,97, i_demographic_service:21,37;
  i_party.adoc and i_party_relationship.adoc contain it NOWHERE).
- **ITS-REST** — grep-verified: no "referential", "resolvable", "dangling", "must exist"
  anywhere in `ITS-REST/specifications/docs/**`. `directory_create.yaml` = 201/400/404,
  `directory_update.yaml` = 200/204/400/404/412 — no 422 on any directory op; `422.yaml` is
  referenced by exactly 12 operation files (composition_create/update + the 10
  agent/group/organisation/person/role create/update). The only refs-related clause is
  `docs/overview/Requests_and_responses.md §Prefer resolving Object references` (L324-332,
  `Prefer: return=representation, resolve_refs`) — a READ-side option with no
  unresolvable-ref branch, and **`parameters/header/Prefer.yaml`'s schema `enum` does not
  admit that value at all** (only return=representation|minimal|identifier), so the
  documented header value fails its own parameter schema.
- **CNF, the fixture that pins the permissive reading** —
  `CNF/tests/platform/robot/_resources/test_data_sets/directory/{empty,subfolders}_*_items.json`
  and `.../directory/update/3_add_items.json` all carry ONE OBJECT_REF: id
  `d936409e-901f-4994-8d33-ed104d46015b`, `namespace: "my.system.id"` (FOREIGN, not `local`),
  `type: VERSIONED_COMPOSITION` — a uid no fixture ever creates. The consuming case
  `I_EHR_DIRECTORY/update_directory/…-ehr_with_directory.robot` asserts
  `validate PUT response - 200 updated`.
