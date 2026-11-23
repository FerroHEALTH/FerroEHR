// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `VERSION` functions: `canonical_form()` over a serialised version, and
//! the subtype dispatch of `uid`, `owner_id`, `is_branch`,
//! `preceding_version_uid`, `lifecycle_state` and `data`.
//!
//! NOTE: RM `master06-change_control_package.adoc` §Digital Signature leaves the
//! serialization `[.tbd]`; ours is canonical JSON without `signature`, under
//! RFC 8785.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde_json::Value;

use crate::v1_2::common::change_control::canonical_form_impl::{canonical_json, canonical_object};
use crate::v1_2::common::change_control::version::Version;
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_base::v1_3::prelude::{HierObjectId, ObjectVersionId};

impl<T> Version<T> {
    /// `VERSION.uid`: stored by an original, `item.uid` for an import.
    #[must_use]
    pub fn uid(&self) -> &ObjectVersionId {
        match self {
            Version::OriginalVersion(original) => &original.uid,
            Version::ImportedVersion(imported) => imported.uid(),
        }
    }

    /// `VERSION.owner_id`: the `object_id` of `uid` (`Owner_id_valid`).
    #[must_use]
    pub fn owner_id(&self) -> HierObjectId {
        HierObjectId::from(self.uid().object_id())
    }

    /// `VERSION.is_branch`: `uid`'s `version_tree_id.is_branch`.
    #[must_use]
    pub fn is_branch(&self) -> bool {
        self.uid().is_branch()
    }

    /// `VERSION.preceding_version_uid`, `None` for a first version ("Void if this
    /// is the first version").
    #[must_use]
    pub fn preceding_version_uid(&self) -> Option<&ObjectVersionId> {
        match self {
            Version::OriginalVersion(original) => original.preceding_version_uid.as_ref(),
            Version::ImportedVersion(imported) => imported.preceding_version_uid(),
        }
    }

    /// `VERSION.lifecycle_state`, through the same dispatch.
    #[must_use]
    pub fn lifecycle_state(&self) -> &DvCodedText {
        match self {
            Version::OriginalVersion(original) => &original.lifecycle_state,
            Version::ImportedVersion(imported) => imported.lifecycle_state(),
        }
    }

    /// `VERSION.data`, `None` for a logical delete, whose data "is set to Void"
    /// (`master06` §Contributions).
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
    #[error("RFC 8785 (JCS) canonicalisation failed")]
    Canonicalize(#[source] serde_json::Error),
}

/// Produces the `canonical_form` of a serialised Version: RFC 8785 without the
/// top-level `signature` (RM common §"Digital Signature").
///
/// # Errors
/// Returns [`CanonicalFormError::Canonicalize`] if RFC 8785 canonicalisation
/// of the value fails.
pub fn canonical_form_of_json(value: &Value) -> Result<String, CanonicalFormError> {
    // The `signature` attribute is dropped by joining the remaining members by
    // reference, never by a deep copy of the (potentially large) version body.
    match value {
        Value::Object(map) => canonical_object(
            map.iter()
                .filter(|(k, _)| k.as_str() != "signature")
                .map(|(k, v)| (k.as_str(), v)),
        ),
        _ => canonical_json(value),
    }
    .map_err(CanonicalFormError::Canonicalize)
}

/// [`canonical_form_of_json`] with `data` joined by reference; a JSON-null
/// `data` stays absent.
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
    let joined = (!data.is_null()).then_some(("data", data));
    let members = map
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .filter(|(k, _)| *k != "signature" && !(joined.is_some() && *k == "data"))
        .chain(joined);
    canonical_object(members).map_err(CanonicalFormError::Canonicalize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::change_control::imported_version::ImportedVersion;
    use crate::v1_2::common::change_control::original_version::OriginalVersion;
    use crate::v1_2::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_3::prelude::{ObjectId, ObjectRef, ObjectRefData, TerminologyId};
    use openehr_term::codes::{AuditChangeType, VersionLifecycleState};

    // The corpus-backed tests are in `tests/it/version_canonical_form.rs`.

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
            commit_audit: Box::new(AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "ferroehr.local".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-07T10:11:12Z".to_owned(),
                },
                change_type: coded(
                    AuditChangeType::Creation.code(),
                    AuditChangeType::Creation.rubric(),
                ),
                description: None,
                committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            })),
            uid: ObjectVersionId::new(value.to_owned()).expect("a well-formed identifier"),
            preceding_version_uid: None,
            other_input_version_uids: openehr_base::containers::present_nonempty(Vec::new()),
            lifecycle_state: coded(
                VersionLifecycleState::Complete.code(),
                VersionLifecycleState::Complete.rubric(),
            ),
            attestations: openehr_base::containers::present_nonempty(Vec::new()),
            data: Some("content".to_owned()),
        }
    }

    /// That same `ORIGINAL_VERSION` in the `VERSION` slot.
    fn typed_version(value: &str) -> Version<String> {
        Version::OriginalVersion(Box::new(typed_original(value)))
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

    /// A trunk version id is not a branch; a three-part one is.
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

    /// An `IMPORTED_VERSION` reports the wrapped original's identity.
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
            commit_audit: Box::new(AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "local.example".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-09T08:00:00Z".to_owned(),
                },
                change_type: coded(
                    AuditChangeType::Creation.code(),
                    AuditChangeType::Creation.rubric(),
                ),
                description: None,
                committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            })),
            item: Box::new(item),
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
        let version = Version::OriginalVersion(Box::new(original));

        assert_eq!(
            version.preceding_version_uid().map(ObjectVersionId::value),
            Some("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1")
        );
        assert_eq!(
            version.lifecycle_state().value,
            VersionLifecycleState::Complete.rubric()
        );
        assert_eq!(version.data(), Some(&"content".to_owned()));
    }

    /// A first version has no predecessor; a deleted one has no data.
    #[test]
    fn the_absent_cases_are_version_states_not_missing_values() {
        let mut original =
            typed_original("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1");
        original.data = None;
        let version = Version::OriginalVersion(Box::new(original));

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
            commit_audit: Box::new(AuditDetails::AuditDetails(AuditDetailsData {
                system_id: "local.example".to_owned(),
                time_committed: DvDateTime {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                    magnitude_status: None,
                    accuracy: None,
                    value: "2026-07-09T08:00:00Z".to_owned(),
                },
                change_type: coded(
                    AuditChangeType::Creation.code(),
                    AuditChangeType::Creation.rubric(),
                ),
                description: None,
                committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            })),
            item: Box::new(item),
        });

        assert_eq!(
            wrapper.preceding_version_uid().map(ObjectVersionId::value),
            Some("8849182c-82ad-4088-a07f-48ead4180515::remote.example::2")
        );
        assert_eq!(
            wrapper.lifecycle_state().value,
            VersionLifecycleState::Complete.rubric()
        );
        assert_eq!(wrapper.data(), Some(&"imported content".to_owned()));
    }
}
