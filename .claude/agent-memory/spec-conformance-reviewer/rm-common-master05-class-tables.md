---
name: rm-common-master05-class-tables
description: Verified findings for RM common master05 §5.2 (FOLDER + VERSIONED_FOLDER class tables) — the ITS-XML v1-lineage FOLDER has NO details element (default-served XML is schema-invalid), the uid-NOTE self-contradiction is a FIVE-class family, and the OAS narrows FOLDER.items ids
metadata:
  type: feedback
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

Verified 2026-08-02 against RM 1.2.0
`RM/docs/UML/classes/org.openehr.rm.common.{folder,versioned_folder,versioned_object}.adoc`
(the two §5.2 includes at `master05-directory_package.adoc:32,34`).

**PARTLY SUPERSEDED 2026-08-19 (see [[rm-common-ch567-fix-verification]]): the
lineage-content fact below still holds, but the SERVED default is now v2
(`ferroehr-rest/src/overview/negotiate.rs:366`, owner ruling #1666) and the
self-deriving XSD gate exists (`openehr-its/tests/it/xml_xsd_validity.rs`), so
the "default-served XML is schema-invalid" half is closed.**

**THE BIG ONE — ITS-XML lineage is NOT "namespace only":**
`crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/ALL/Structure.xsd:34-43`
types FOLDER as `folders` + `items` ONLY — **no `details` element** (RM
1.0.2-era). `details` first appears in the RM Release-1.1.0 schemas, which
upstream re-stamped to namespace **v2** only. Our codec always writes
`details` (`crates/openehr-its/src/xml/generated/impls.rs:10338`) and the
default served lineage is **v1** (`tests/it/xml_namespace.rs:34`), so a
canonical-XML FOLDER carrying `details` declares a namespace whose published
FOLDER type forbids it — against ITS-REST `Resources.md` §XML Format
("responses MUST conform to the [published XSDs]", link target = ITS-XML
**latest** = 2.0.0/v2). Pinned as correct at
`app/ferroehr-rest/tests/it/directory_http.rs:1252-1265`; the fixture
`cnf.directory.v2.xml` (FOLDER with `details`, `xmlns=…/v1`) carries
`validity: valid` in the corpus MANIFEST. The repo-wide premise "the two
lineages differ only by the root xmlns" (schemas `PROVENANCE.md`,
`xml_namespace.rs` header) is FALSE for FOLDER — re-check it for any class
added after RM 1.0.2 before trusting it again. There is NO XSD-validation
gate anywhere (only C14N via xmllint), so nothing catches this class of
defect.

**The uid NOTE is a FIVE-class family, not three** (AMB-65 input): the
identical self-contradicting NOTE ("copied from the `_object_id()_` …" then a
worked example copying the full 3-part value) appears verbatim on FOLDER,
COMPOSITION, EHR_STATUS, EHR_ACCESS and PARTY class tables (`grep
'object_id()' RM/docs/UML/classes/*.adoc` = 5 hits). AMB-65's `source` names
only master03 + EHR_STATUS. Extra corroboration for the fixed handling on
FOLDER specifically: the released OAS `ITS-REST specifications/schemas/ehr/
Folder.yaml` example carries `uid: {_type: OBJECT_VERSION_ID, value:
"…::openEHRSys.example.com::1"}`, and the ITS-JSON FOLDER schema types `uid`
as `OBJECT_VERSION_ID | HIER_OBJECT_ID`.

**FOLDER.items id-type width is an unregistered released-vs-released split:**
RM/BASE `object_ref.adoc` types `id: OBJECT_ID` (six subtypes) and ITS-JSON
`OBJECT_REF.id` enumerates all six — but the released OAS narrows
`Folder.items` to `UObjectRefOfUidBasedId` → `UUidBasedId` = oneOf
{HIER_OBJECT_ID, OBJECT_VERSION_ID}. A **version-pinned OBJECT_VERSION_ID
items target is therefore ADMITTED** (and accepted by our stack), while a
GENERIC_ID/TERMINOLOGY_ID target is admitted by RM+ITS-JSON and excluded by
the OAS — we accept it (lax by the OAS reading). Neither is registered.

**Conformant, verified (don't re-check):** the generated `Folder` /
`VersionedFolder` shapes are exact; `folder_impl.rs` is now correctly
grounded (FOLDER declares NO invariants; only LOCATABLE's
`Archetype_node_id_valid`); `VERSIONED_*` container bodies ARE now built as
the generated `VersionedObject` subtype and serialized through the codec
(`app/ferroehr/src/versioning/wire.rs:217` — the old hand-built `json!`
finding is FIXED); `stamp_version_uid` (`versioning/change.rs:448`) +
`with_uid` (`service/ehr/meta.rs:128`) stamp the **root only**, matching the
NOTE's "_top-level_ (i.e. tree-root)" scope; `FOLDER.details` foreign-type
refusal works and the §5.1 predicted JSON/XML asymmetry really does NOT
reproduce — mechanism: `fast.rs class_slot_conforms` cannot vouch (DV_TEXT ∉
ITEM_STRUCTURE.descendants) → falls back to the typed `run::<Folder>` decode
→ `record_type_mismatch`.

**Honest gaps found:** `VERSIONED_OBJECT.revision_history()` is realized for
COMPOSITION / EHR_STATUS / demographic parties but NOT for VERSIONED_FOLDER —
and neither SM `i_ehr_directory.adoc` (10 operations, no revision history) nor
ITS-REST surfaces it; AMB-24 covers only `get_versioned_directory`, so the
rest of the withheld container surface is unregistered.

**Citation discipline:** #1658 purged archie from `folder_impl.rs`, but the
FOLDER enforcement chain still cites it —
`crates/openehr-rm/src/validate.rs:113-116` ("the exact message the reference
implementation's `RMObjectValidator` emits"),
`crates/openehr-base/src/base_types/identification/object_ref_impl.rs:3`
("`Namespace_valid` (archie `ObjectRef`)" — an invariant name BASE does not
have; the regex itself IS released, in the `namespace` Meaning row), and the
@generated `validate/generated.rs:4` (emitter fix).
