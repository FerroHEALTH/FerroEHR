---
name: unknown-key-open-vs-closed-objects-location
description: Where the "may/must a server refuse an unknown JSON key" answer lives — ITS-REST Resources.md MUST/SHOULD split, closed ITS-JSON vs open ITS-REST validation-OAS, and the RM1.1.0-schema-vs-RM1.2.0-model attribute deltas
metadata:
  type: reference
---

# Unknown-key / open-vs-closed-object adjudication — file map

## The four load-bearing locations (oracle order)

1. **ITS-REST docs text** —
   `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
   §Data representation. The MUST/SHOULD asymmetry lives here:
   - **L75** (XML): payloads+responses **MUST** conform to the published XSDs.
   - **L87** (JSON): payload+result **SHOULD** be valid against the published
     JSON-Schemas, + "NOTE: The JSON-Schema project is under development."
   - **L91** lowercase "must" on snake_case attribute naming → NOT normative,
     per `overview/Preface.md` §Requirements (BCP14 "only when … all capitals").
   - **L107** "Metadata attributes (those that are not also RM attributes) will
     always be prefixed by a `'_'`" — the only sanctioned non-RM key form.
   - **L120/L122**: Null/empty SHOULD be absent; attribute ORDER is free.
2. **ITS-JSON (closed)** — `crates/openehr-its/schemas/json/openehr_rm_1.1.0_all.json`
   (byte-identical to `docs/specs/openehr/ITS-JSON/components/openehr_rm_1.1.0_all.json`):
   draft-07, 134 definitions, **`additionalProperties: false` on 128**; the 6
   without are degenerate stubs (ARRAY/LIST/SET/URI/VALIDITY_KIND/VERSION_STATUS,
   `properties` = `_type` only). Closure works because
   `docs/specs/openehr/ITS-JSON/README.md` §Design choices: "no inheritance in
   definitions, so definitions contain all their fields directly". **Status =
   DEVELOPMENT** (README + `manifest.json` `spec_status`).
3. **ITS-REST OAS (open)** — the release's own designated payload-validation
   artifact per `overview/Specifications.md` ("optimized for data validation …
   used by (mock-)servers or applications to validate … payloads", and it
   "flatten[s] all these requirements"). In
   `crates/openehr-its/vendor/rest-oas/ehr-validation.openapi.yaml`: 114 schemas,
   exactly **2 closed** (`Archetyped`, `UpdateItemTag`), 0 open:true, 112 silent
   → open by default. Decomposed docs-side mirror:
   `docs/specs/openehr/ITS-REST/specifications/schemas/` — closed only
   `common/{Archetyped,ItemTag,UpdateItemTag}.yaml`; `additionalProperties: true`
   only `query/{ResultSetMetadata,QueryParameters}.yaml`.
   ⇒ **The two computable artifacts contradict each other** on RM objects.
4. **Release strategy = NOT VENDORED.** `grep -rl release_strategy
   docs/specs/openehr` → empty. The minor-release compatibility promise quoted in
   `docs/VERSIONS.md` §Spec version policy is an EXTERNAL governance page, not
   spec text. No vendored sentence tells a server to tolerate newer-minor keys.
5. **RM/BASE prose** — no unknown-attribute rule anywhere (grep for
   `unknown (attribute|element|propert|field|key)` / `undeclared` /
   `extra attribute` across all vendored adoc/md returns only unrelated hits).
   Closest: `BASE/docs/architecture_overview/master04-design_principles.adoc:62`
   "runtime data now conform … concretely to the reference model"
   (non-normative, no BCP14 keyword).

## Status code if refusing
`docs/specs/openehr/ITS-REST/specifications/responses/400.yaml` — "could not be
parsed or is invalid (… syntactically invalid header, parameter or **content**)"
vs `422.yaml` — "syntax is correct, **could be converted to a resource**, but …
semantic validation errors". Unknown key ⇒ **400**. Overview table at
`Requests_and_responses.md` L223/L233 concurs.

## RM 1.1.0-schema vs RM 1.2.0-model attribute deltas (decides WHAT to close over)
The ITS-JSON 1.1.0 schema and the generated model disagree BOTH ways:
- schema-allows / model-lacks: `DV_QUANTITY.property` (RM 1.2.0 class table
  `RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantity.adoc` has no
  `property`); `reverse_relationships` on PERSON/ORGANISATION/ROLE/GROUP/AGENT
  (removed as an ATTRIBUTE by SPECRM-124,
  `RM/docs/demographic/master00-amendment_record.adoc:8`; survives only as a
  FUNCTION in `org.openehr.rm.demographic.party.adoc:46` — note the `()`);
  LOCATABLE attrs on SYNC_EXTRACT/SYNC_EXTRACT_REQUEST.
- model-allows / schema-lacks: `EHR.tags` (RM 1.2.0 ITEM_TAG package).
⇒ Close over the **generated RM model**, never the ITS-JSON 1.1.0 schema.

## Two live TRANSLATION_DETAILS spellings (both correct)
`RM/docs/UML/classes/org.openehr.rm.common.translation_details.adoc:24` =
**`accreditaton`** (typo preserved; RM's AOM-1.4-only copy, see
`RM/docs/common/master08-resource_package.adoc:3`) vs
`BASE/docs/UML/classes/org.openehr.base.resource.translation_details.adoc:24` =
**`accreditation`** (corrected by SPECPUB-6,
`BASE/docs/resource/master00-amendment_record.adoc:49`).

## Corpus exclusion machinery (for blast-radius scans)
`crates/openehr-its/tests/it/common.rs::excluded()` is the by-name documented
exclusion list; `fidelity.rs` (~L176-180) and `xml_roundtrip.rs` (L27-30) carry
SEPARATE shape/name-based skips; `crates/openehr-its/tests/vendor/PROVENANCE.md`
describes a third, inaccurate version. Check all three before claiming a fixture
is live.
