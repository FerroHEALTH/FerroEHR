// @generated-from-template templates/openehr-rm/common/change_control/version_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions of `VERSION`.
//!
//! The surface is `canonical_form()` over an already-serialised Version value,
//! plus the subtype-dispatching `uid()` / `owner_id()` / `is_branch()` /
//! `preceding_version_uid()` / `lifecycle_state()` / `data()` on the closed
//! `VERSION` subtype set.
//!
//! Spec authority:
//! - RM common §"Digital Signature"
//!   (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`):
//!   "a Version object (an `ORIGINAL_VERSION` or `IMPORTED_VERSION`) is
//!   serialised into canonical form which is then hashed to produce a digest …
//!   note that the signature attribute will be Void at this point". For an
//!   `IMPORTED_VERSION` "all attributes of the object are serialised".
//! - `VERSION.canonical_form()`
//!   (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.version.adoc`):
//!   "A canonical serial form of this Version, created by serialising all
//!   attributes except signature, suitable for generating reliable hashes and
//!   signatures."
//!
//! NOTE: the spec leaves the exact serialization `[.tbd]`, so the signed
//! bytes are OUR OWN extension: canonical openEHR JSON with the top-level
//! `signature` removed, canonicalised per RFC 8785 (`serde_jcs`) so the bytes
//! are deterministic — the single source for both signing and verification;
//! the adjudication (including RFC 8785 over the ODIN the spec speculates
//! about) is recorded on the tracker.
//!
//! The signed input is always assembled as a `serde_json::Value` (the shape the
//! versioning service and the wire boundary already hold), so this module works
//! purely on a `Value` — the canonical-JSON serialization of a *typed* Version
//! is a wire-boundary concern that lives with the codec in `openehr-its`, not
//! here.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;

use crate::v1_1::common::change_control::version::Version;
use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
use openehr_base::v1_2::prelude::{HierObjectId, ObjectVersionId};

impl<T> Version<T> {
    /// `VERSION.uid`: the version's own three-part identifier.
    ///
    /// `VERSION` declares this abstract; each subtype effects it — an
    /// `ORIGINAL_VERSION` stores it, and an `IMPORTED_VERSION` derives it from
    /// the version it wraps (`Post: Result = item.uid`, RM
    /// `UML/classes/org.openehr.rm.common.imported_version.adoc` §Functions).
    /// This is that dispatch.
    #[must_use]
    pub fn uid(&self) -> &ObjectVersionId {
        match self {
            Version::OriginalVersion(original) => &original.uid,
            Version::ImportedVersion(imported) => imported.uid(),
        }
    }

    /// `VERSION.owner_id`: "Copy of the owning `VERSIONED_OBJECT._uid_` value;
    /// extracted from the local `_uid_` property's `_object_id_`"
    /// (`Post: Result.value.is_equal (uid.object_id.value)`; the same equality
    /// is the class's `Owner_id_valid` invariant).
    ///
    /// The extraction is the BASE accessor `OBJECT_VERSION_ID.object_id()`, so
    /// there is one lexical decoder for the three-part form rather than a
    /// second one here.
    #[must_use]
    pub fn owner_id(&self) -> HierObjectId {
        HierObjectId::from(self.uid().object_id())
    }

    /// `VERSION.is_branch`: "True if this Version represents a branch. Derived
    /// from `_uid_` attribute" — delegated to
    /// `OBJECT_VERSION_ID.is_branch()`, which is itself
    /// `version_tree_id.is_branch` (BASE
    /// `UML/classes/org.openehr.base.base_types.version_tree_id.adoc`
    /// §Functions: "has `_branch_number()_` and `_branch_version()_` parts").
    #[must_use]
    pub fn is_branch(&self) -> bool {
        self.uid().is_branch()
    }

    /// `VERSION.preceding_version_uid`: the identifier of the version this one
    /// succeeds, or `None` when this is the first version.
    ///
    /// `VERSION` declares this abstract and each subtype effects it: an
    /// `ORIGINAL_VERSION` stores it (`0..1`,
    /// `UML/classes/org.openehr.rm.common.original_version.adoc` §Attributes),
    /// an `IMPORTED_VERSION` derives it from the version it wraps
    /// (`Post: Result = item.preceding_version_uid`,
    /// `UML/classes/org.openehr.rm.common.imported_version.adoc` §Functions).
    /// The absent case is the spec's own: `VERSION.preceding_version_uid` is
    /// documented "Void if this is the first version"
    /// (`UML/classes/org.openehr.rm.common.version.adoc` §Functions).
    #[must_use]
    pub fn preceding_version_uid(&self) -> Option<&ObjectVersionId> {
        match self {
            Version::OriginalVersion(original) => original.preceding_version_uid.as_ref(),
            Version::ImportedVersion(imported) => imported.preceding_version_uid(),
        }
    }

    /// `VERSION.lifecycle_state`: the lifecycle state of the content item in
    /// this version.
    ///
    /// The same dispatch: stored by an `ORIGINAL_VERSION`, derived as
    /// `item.lifecycle_state` by an `IMPORTED_VERSION`
    /// (`UML/classes/org.openehr.rm.common.imported_version.adoc` §Functions).
    #[must_use]
    pub fn lifecycle_state(&self) -> &DvCodedText {
        match self {
            Version::OriginalVersion(original) => &original.lifecycle_state,
            Version::ImportedVersion(imported) => imported.lifecycle_state(),
        }
    }

    /// `VERSION.data`: the content of this version, or `None` when it carries
    /// none.
    ///
    /// The same dispatch, over `ORIGINAL_VERSION.data`, which the spec
    /// declares `0..1`. The absent case is a real version state rather than a
    /// missing value: a logical delete commits "a new `ORIGINAL_VERSION` whose
    /// data attribute is set to Void"
    /// (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
    /// §Contributions).
    #[must_use]
    pub fn data(&self) -> Option<&T> {
        match self {
            Version::OriginalVersion(original) => original.data.as_ref(),
            Version::ImportedVersion(imported) => imported.data(),
        }
    }
}

/// Failure to produce a Version canonical form.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalFormError {
    /// RFC 8785 (JCS) canonicalisation failed.
    #[error("RFC 8785 (JCS) canonicalisation: {0}")]
    Canonicalize(#[source] serde_json::Error),
}

/// Produces the spec `canonical_form` of a serialised Version JSON value.
///
/// The top-level `signature` attribute is dropped (Void during serialisation
/// per RM common §"Digital Signature") and the RFC 8785 (JCS) canonical string
/// is emitted.
///
/// The application service assembles the Version as a `serde_json::Value` before
/// persistence and calls this, so signing and verification share one source of
/// the signed bytes.
///
/// Spec authority: RM common §"Digital Signature"
/// (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`) and
/// `VERSION.canonical_form()`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.version.adoc`).
///
/// # Errors
/// Returns [`CanonicalFormError::Canonicalize`] if RFC 8785 canonicalisation
/// of the value fails.
pub fn canonical_form_of_json(value: &Value) -> Result<String, CanonicalFormError> {
    // The `signature` attribute is dropped through a shallow reference view —
    // never a deep copy of the (potentially large) version body; RFC 8785
    // orders keys itself, so the view's map order is immaterial.
    match value {
        Value::Object(map) if map.contains_key("signature") => {
            let trimmed: std::collections::BTreeMap<&str, &Value> = map
                .iter()
                .filter(|(k, _)| k.as_str() != "signature")
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            serde_jcs::to_string(&trimmed).map_err(CanonicalFormError::Canonicalize)
        }
        _ => serde_jcs::to_string(value).map_err(CanonicalFormError::Canonicalize),
    }
}

/// Produces the spec `canonical_form` of a serialised Version JSON value whose
/// `data` attribute joins by reference.
///
/// Byte-identical to [`canonical_form_of_json`] over the fully-assembled value
/// (RFC 8785 orders keys), without deep-copying the — potentially large —
/// `data` subtree into the envelope. A JSON-null `data` stays absent, matching
/// a Version with Void data (RM common §"Logical Deletion"); a `signature` on
/// the envelope is dropped exactly as [`canonical_form_of_json`] drops it.
///
/// # Errors
/// Returns [`CanonicalFormError::Canonicalize`] if RFC 8785 canonicalisation
/// of the joined value fails.
pub fn canonical_form_of_json_with_data(
    envelope: &Value,
    data: &Value,
) -> Result<String, CanonicalFormError> {
    let Value::Object(map) = envelope else {
        return canonical_form_of_json(envelope);
    };
    let mut view: std::collections::BTreeMap<&str, &Value> = map
        .iter()
        .filter(|(k, _)| k.as_str() != "signature")
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    if !data.is_null() {
        view.insert("data", data);
    }
    serde_jcs::to_string(&view).map_err(CanonicalFormError::Canonicalize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::change_control::imported_version::ImportedVersion;
    use crate::v1_1::common::change_control::original_version::OriginalVersion;
    use crate::v1_1::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_2::prelude::{ObjectId, ObjectRef, ObjectRefData, TerminologyId};

    // The corpus-backed `canonical_form_of_json` tests are integration tests
    // (`tests/it/version_canonical_form.rs`): their fixture lives outside this
    // crate's published package, which a `src/` test cannot read.

    /// A plain `ORIGINAL_VERSION` whose `uid` is `value`, to exercise the
    /// identity-derived functions of `VERSION`.
    fn typed_original(value: &str) -> OriginalVersion<String> {
        OriginalVersion {
            contribution: ObjectRef::ObjectRef(ObjectRefData {
                namespace: "local".to_owned(),
                r#type: "CONTRIBUTION".to_owned(),
                id: ObjectId::HierObjectId(
                    HierObjectId::new("11111111-1111-4111-8111-111111111111".to_owned())
                        .expect("a well-formed identifier"),
                ),
            }),
            signature: None,
            commit_audit: AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "ferroehr.local".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-07T10:11:12Z".to_owned(),
                },
                change_type: coded("249", "creation"),
                description: None,
                committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            }),
            uid: ObjectVersionId::new(value.to_owned()).expect("a well-formed identifier"),
            preceding_version_uid: None,
            other_input_version_uids: openehr_base::containers::present_nonempty(Vec::new()),
            lifecycle_state: coded("532", "complete"),
            attestations: openehr_base::containers::present_nonempty(Vec::new()),
            data: Some("content".to_owned()),
        }
    }

    /// That same `ORIGINAL_VERSION` in the `VERSION` slot.
    fn typed_version(value: &str) -> Version<String> {
        Version::OriginalVersion(typed_original(value))
    }

    fn coded(code: &str, rubric: &str) -> DvCodedText {
        DvCodedText {
            value: rubric.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: code.to_owned(),
                preferred_term: None,
            },
        }
    }

    /// `Post: Result.value.is_equal (uid.object_id.value)` — the same equality
    /// the `Owner_id_valid` invariant states.
    #[test]
    fn owner_id_is_the_uids_object_id() {
        let version = typed_version("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2.1.1");
        assert_eq!(
            version.owner_id().value(),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        assert_eq!(
            version.owner_id(),
            HierObjectId::from(version.uid().object_id()),
            "Owner_id_valid"
        );
    }

    /// "True if this Version represents a branch. Derived from `uid` attribute"
    /// — a bare trunk version id is not a branch, a three-part
    /// `VERSION_TREE_ID` is.
    #[test]
    fn is_branch_follows_the_version_tree_id() {
        assert!(
            !typed_version("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2").is_branch()
        );
        assert!(
            typed_version("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2.1.1")
                .is_branch()
        );
    }

    /// The `IMPORTED_VERSION` arm of the same dispatch: the wrapper reports the
    /// WRAPPED original's identity, so both derived functions read the foreign
    /// version id (`Post: Result = item.uid`).
    #[test]
    fn an_imported_version_derives_its_identity_from_the_wrapped_original() {
        let item = typed_original("8849182c-82ad-4088-a07f-48ead4180515::remote.example::3.2.1");
        let wrapper = Version::ImportedVersion(ImportedVersion {
            contribution: ObjectRef::ObjectRef(ObjectRefData {
                namespace: "local".to_owned(),
                r#type: "CONTRIBUTION".to_owned(),
                id: ObjectId::HierObjectId(
                    HierObjectId::new("22222222-2222-4222-8222-222222222222".to_owned())
                        .expect("a well-formed identifier"),
                ),
            }),
            signature: Some("local-wrapper-signature".to_owned()),
            commit_audit: AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "local.example".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-09T08:00:00Z".to_owned(),
                },
                change_type: coded("249", "creation"),
                description: None,
                committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            }),
            item,
        });
        assert_eq!(
            wrapper.uid().value(),
            "8849182c-82ad-4088-a07f-48ead4180515::remote.example::3.2.1"
        );
        assert_eq!(
            wrapper.owner_id().value(),
            "8849182c-82ad-4088-a07f-48ead4180515"
        );
        assert!(wrapper.is_branch());
    }

    /// The three remaining abstract functions dispatch to what the subtype
    /// stores: an `ORIGINAL_VERSION` answers from its own attributes.
    #[test]
    fn an_original_version_answers_from_its_own_attributes() {
        let mut original =
            typed_original("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2");
        original.preceding_version_uid = Some(
            ObjectVersionId::new(
                "8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1".to_owned(),
            )
            .expect("a well-formed identifier"),
        );
        let version = Version::OriginalVersion(original);

        assert_eq!(
            version.preceding_version_uid().map(ObjectVersionId::value),
            Some("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1")
        );
        assert_eq!(version.lifecycle_state().value, "complete");
        assert_eq!(version.data(), Some(&"content".to_owned()));
    }

    /// "Void if this is the first version" — a first version reports no
    /// predecessor, and a logically deleted version reports no data
    /// (master06 §Contributions).
    #[test]
    fn the_absent_cases_are_version_states_not_missing_values() {
        let mut original =
            typed_original("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1");
        original.data = None;
        let version = Version::OriginalVersion(original);

        assert!(version.preceding_version_uid().is_none());
        assert!(version.data().is_none());
    }

    /// An `IMPORTED_VERSION` answers from the version it WRAPS, not from its
    /// own audit — the delegation the class table states.
    #[test]
    fn an_imported_version_answers_from_the_wrapped_original() {
        let mut item = typed_original("8849182c-82ad-4088-a07f-48ead4180515::remote.example::3");
        item.preceding_version_uid = Some(
            ObjectVersionId::new(
                "8849182c-82ad-4088-a07f-48ead4180515::remote.example::2".to_owned(),
            )
            .expect("a well-formed identifier"),
        );
        item.data = Some("imported content".to_owned());
        let wrapper = Version::ImportedVersion(ImportedVersion {
            contribution: ObjectRef::ObjectRef(ObjectRefData {
                namespace: "local".to_owned(),
                r#type: "CONTRIBUTION".to_owned(),
                id: ObjectId::HierObjectId(
                    HierObjectId::new("33333333-3333-4333-8333-333333333333".to_owned())
                        .expect("a well-formed identifier"),
                ),
            }),
            signature: None,
            commit_audit: AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "local.example".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-09T08:00:00Z".to_owned(),
                },
                change_type: coded("249", "creation"),
                description: None,
                committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            }),
            item,
        });

        assert_eq!(
            wrapper.preceding_version_uid().map(ObjectVersionId::value),
            Some("8849182c-82ad-4088-a07f-48ead4180515::remote.example::2")
        );
        assert_eq!(wrapper.lifecycle_state().value, "complete");
        assert_eq!(wrapper.data(), Some(&"imported content".to_owned()));
    }
}
