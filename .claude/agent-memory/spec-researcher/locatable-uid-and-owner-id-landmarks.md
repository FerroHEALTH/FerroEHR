---
name: locatable-uid-and-owner-id-landmarks
description: Where the top-level LOCATABLE.uid population/form rule and the VERSIONED_OBJECT.owner_id concrete shapes live — the FIVE RM class-table NOTEs, the master03 truncating example, and the three ITS-REST Versioned*.yaml owner_id examples
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# `LOCATABLE.uid` on top-level objects + `VERSIONED_OBJECT.owner_id` — where the sentences are

## The uid-population NOTE lives in FIVE RM class tables, not one
grep `"strongly recommended that the inherited"` under `RM/docs/UML/classes/`
returns exactly five files — and it is the decisive answer to any
"is the served uid assigned anywhere / only for COMPOSITION?" question:
- `org.openehr.rm.composition.composition.adoc` L11-12
- `org.openehr.rm.ehr.ehr_status.adoc` L10-11
- `org.openehr.rm.ehr.ehr_access.adoc` L11-12
- `org.openehr.rm.common.folder.adoc` L11-12 (scoped to *top-level* FOLDER)
- `org.openehr.rm.demographic.party.adoc` L10-11
Present identically in the RELEASED RM 1.1.0 BMM `documentation` and in the
1.2.0 pin (checked via `tools/openehr-codegen/vendor/bmm/components/RM/json/`).
So a claim that "no released text states this outside COMPOSITION" is FALSE.

## The internal split on the FORM (object_id vs all three parts)
All five class NOTEs use the same two-sentence shape: the RULE says
"copied from the `object_id()` of the `uid` field of the enclosing VERSION",
but the WORKED EXAMPLE says the full `87284370-…::uk.nhs.ehr1::2` "would be
copied" — no truncation step. Only
`RM/docs/common/master03-archetyped_package.adoc` §Unique Node Identification
truncates in its example ("the Guid `87284370-…` would be copied"). Same section
also carries "The `_uid_` attribute will usually be empty in most EHR data".
ITS-REST side: `ITS-REST/specifications/docs/overview/Resources.md`
§Identifier types NOTE (scoped to COMPOSITION, "strongly recommended"/"should")
copies the FULL three-part value; the same section's addressing bullet derives
`versioned_object_uid` + version from `COMPOSITION.uid.value`.
PARTY is the only type with an invariant: `…demographic.party.adoc`
§Invariants `Uid_mandatory: uid /= Void` (also RM demographic
`master02-demographic_package.adoc` §Party Identification types the id
`OBJECT_VERSION_ID`).

## `VERSIONED_OBJECT.owner_id` — THREE concrete examples, not one
No prose anywhere (`RM/docs/`, `SM/docs/`, ITS-REST docs text) assigns
`namespace`/`type` values; only OAS examples do:
- `ITS-REST/specifications/schemas/ehr/VersionedComposition.yaml` → `local` / `EHR`
- `ITS-REST/specifications/schemas/ehr/VersionedEhrStatus.yaml` → `local` / `EHR`
- `ITS-REST/specifications/schemas/demographic/VersionedParty.yaml` → `local` / `SYSTEM`
  (`SYSTEM` is NOT an RM class, contra `BASE/docs/UML/classes/
  org.openehr.base.base_types.object_ref.adoc` `type` = "Name of the class …
  from the relevant reference model" — released-example defect)
The five `schemas/demographic/ItemTagOf*.yaml` examples use the same
`local`/`SYSTEM` pair but for the different `ITEM_TAG.owner_id` attribute.
Reverse edge IS an invariant: `…rm.ehr.ehr.adoc` §Invariants
`Ehr_status_valid` / `Ehr_access_valid` / `Compositions_valid` / `Directory_valid`.
Prose on owner_id: `RM/docs/common/master06-change_control_package.adoc` L27/L35/L205.

Related: [[version-lifecycle-and-identification-location]],
[[versioned-object-read-ops-location]], [[its-rest-wire-contract-location]].

## Closure proofs re-derived 2026-08-21 (use these, don't re-grep blind)
- **The five uid NOTEs are COMPLETE for the RM's top-level types.** The RM's
  versioned-container set is exactly six class files
  (`RM/docs/UML/classes/org.openehr.rm.{common.versioned_folder,
  common.versioned_object,demographic.versioned_party,ehr.versioned_composition,
  ehr.versioned_ehr_access,ehr.versioned_ehr_status}.adoc`; the six
  `ehr_extract.x_versioned_*` are Extract mirrors), so the versioned CONTENT
  types are FOLDER/PARTY/COMPOSITION/EHR_ACCESS/EHR_STATUS — the same five that
  carry the NOTE. `PARTY_RELATIONSHIP` has NO versioned container class in RM
  and no uid NOTE. Corroborating prose: `BASE/docs/architecture_overview/
  master09-identification.adoc` §Levels of Identification L57 ("content
  structures such as `COMPOSITION`, `EHR_STATUS`, `EHR_ACCESS`, `PARTY` etc.")
  — that section states NO population rule and no normative keyword.
- **The whole RM UML class set contains only THREE uid-bearing invariants**
  (`grep "uid" RM/docs/UML/classes/*.adoc | grep "/= Void"`):
  `original_version.adoc` Other_input_version_uids_valid,
  `version.adoc` Preceding_version_uid_validity, and
  `demographic.party.adoc` L70 `Uid_mandatory: uid /= Void`. So "only PARTY's
  uid is invariant-backed" is provable, not merely unfound. FOLDER's class
  table has NO Invariants section at all.
- **`SYSTEM` is not a class in ANY vendored BMM** — checked all 18 files under
  `tools/openehr-codegen/vendor/bmm/` (RM 1.0.2/1.0.3/1.0.4/1.1.0/1.2.0, BASE
  1.0.4-1.3.0, AM, LANG, TERM): zero class names containing "SYSTEM", and no
  `=== SYSTEM` heading in RM/BASE/SM docs. `OBJECT_REF.type`'s only escape
  hatch is the literal `ANY` ("can be used to indicate that any type is
  accepted"), which does not cover `SYSTEM`. Upstream-report #2524.
- The uid-population rule is recommendation-strength EVERYWHERE it appears:
  five RM NOTEs ("It is strongly recommended") + `ITS-REST/specifications/
  docs/overview/Resources.md` L42-43 ("strongly recommended" + lowercase
  "should be copied"), while that same file uses uppercase MUST/SHOULD for real
  requirements (L172/L174) — upstream-report #2523.
