---
name: extract-import-validation-location
description: Where the (non-)obligation to re-validate IMPORTED EHR-Extract content lives — RM ehr_extract master09/master02, RM common master06 §6.4.1.1 "faithful copy" + §6.2.5, RM ehr master04 §Versioning Scenarios Case 2/3, SM I_EHR_EXTRACT_SERVICE (no I_VALIDITY_CHECKER, no preconditions), and the total ITS-REST/CNF silence
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# "Must an importer re-validate received VERSIONs?" — navigation

Answer shape: **no released text requires it, and several passages positively
support verbatim storage**; only BASE Arch. Overview permits it ("can be used").

## The load-bearing sentences (file → line)
- `RM/docs/common/master06-change_control_package.adoc`
  - **L259** (§Semantics in Distributed Systems > Copying > The Copy Operation):
    "the `ORIGINAL_VERSION` instance is **never modified** - it remains a
    **faithful copy** of its original" — the verbatim-preservation clause.
  - **L271** "there is of course **no obligation to do anything** with the
    received information" (inside §Subsequent Local Modifications).
  - **L84/L86** (§Committal and Audits) import ⇒ IMPORTED_VERSION wrapper;
    "the audit from initial creation … is preserved no matter how many times".
  - **L65** (§Contributions) `_import of item_` ⇒ change_type `249|creation|`.
  - **L104** signing an IMPORTED_VERSION signs *the act of importing*.
  - **L111** signatures in Extracts assure a receiver "who has no knowledge of
    the quality of the processes used in the originating system" — the ONLY
    trust mechanism the spec names for a receiver.
- `RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc` **L126-132**
  `commit_imported_version` — **NO precondition at all** (contrast
  `commit_original_version` L108 which has one).
- `RM/docs/ehr/master04-ehr_package.adoc` §Versioning Scenarios **L197-199**
  = the MODE SPLIT: Case 2 feeder ⇒ IMPORTED+ORIGINAL pair (converted);
  **Case 3** ORIGINAL_VERSION received in an `EHR_EXTRACT` ⇒ a local author
  manually creates a NEW locally-authored `ORIGINAL_VERSION`.
- `RM/docs/ehr_extract/master09-semantics.adoc` §Versioning Semantics L10-17:
  "reconstruct a complete facsimile"; "no assumptions should made on sender or
  receiver systems"; "Whole Compositions can always be processed by even the
  simplest systems".
- `RM/docs/ehr_extract/master02-requirements.adoc` §Archetypes and Terminology
  **L407-408**: "archetypes are not themselves included in Extracts, and have
  to be resolved separately" — the importer may not HAVE the definitions.
- `AM/docs/Identification/master07-referencing.adoc` **L367**: "data imported
  from another site without the relevant template(s)" — spec explicitly
  contemplates templateless imported data.
- `BASE/docs/architecture_overview/master10-archetypes.adoc`
  §Validation during Data Capture **L239-241**: "Archetype-based validation
  **can** be used in a GUI application or **in a data import service**" — the
  ONLY permissive sentence; it is about archetype-based data CREATION from an
  input stream, not about re-checking received VERSIONs.

## SM side
`SM/docs/UML/classes/i_ehr_extract_service.adoc` — 4 calls
(`export_ehrs`, `export_ehr_extracts`, `import_ehr`, `import_ehr_extract`),
**zero Pre_/Post_/Errors**. Rasterizing
`SM/docs/UML/diagrams/SM-platform.interface.message.svg` (`rsvg-convert -w 2400`,
0 `<text>`) confirms first-hand: all three message interfaces inherit
**I_STATUS directly, NOT I_VALIDITY_CHECKER** — unlike ch.5, where
`i_ehr_composition.adoc` L103-104/L124-125 carry
`Pre_composition_definitions_valid` + `Pre_content_valid`. So the validity
checker is wired to the native commit path only.
SM `master03-common_package.adoc` §Version Update Semantics: `UPDATE_VERSION`
creates `ORIGINAL_VERSION`s only — SM has NO import-side update structure.

## Wire / CNF silence (grep-verified 2026-09-01)
- ITS-REST: `IMPORTED_VERSION` appears ONLY in read-union schemas
  (`schemas/{ehr,demographic}/UVersionOf*.yaml` + `UMImportedVersionOf*.yaml`);
  **no request body anywhere accepts one** (`NewContribution.versions` items are
  `UpdateVersion`). No extract/message/import path exists. The native-commit
  contrast: `operations/composition_{create,update}.yaml` carry `422` whose
  `responses/422.yaml` says "the underlying template is not known or is not
  validating the supplied resource"; `contribution_create.yaml` has NO 422.
- CNF `master13-func_tc_messaging.adoc`: all TBD; sections exist for THREE
  non-existent export ops, and `import_ehr`/`import_ehr_extract` get NO section.

## Released-text defects found here
- `master09-semantics.adoc` §Creation Semantics: "create an
  `X_VERSIONED_COMPOSITION`, and set `_is_primary_`" — `is_primary` is on
  `EXTRACT_CONTENT_ITEM`, not on `X_VERSIONED_OBJECT`/`X_VERSIONED_COMPOSITION`.
  Same list mixes `*` and `**` nesting levels (L81-83 render at top level).
- `master04-common_package.adoc` L131-133 says the actual retrieved entities are
  "known by in the **receiver** system" / "retrievable in the **receiver**
  system" — must be the responding/source system.
- `master05-openehr_extract_package.adoc` L28 "openEHR sytem";
  master09 L15 "no assumptions should made".
- `EXTRACT_CONTENT_ITEM.is_masked` + invariant `is_masked xor item /= Void`:
  a received item can legitimately carry NO content.
