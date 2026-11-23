// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `IMPORTED_VERSION` functions, delegations to the wrapped
//! `ORIGINAL_VERSION` (`org.openehr.rm.common.imported_version.adoc`
//! §Functions), returned by borrow.
//!
//! NOTE: `preceding_version_uid` and `data` are typed `1..1` but delegate to
//! `0..1` attributes (`original_version.adoc`), so both return `Option`.

use crate::v1_2::common::change_control::imported_version::ImportedVersion;
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_base::v1_3::prelude::ObjectVersionId;

impl<T> ImportedVersion<T> {
    /// `IMPORTED_VERSION.uid`: `Post: Result = item.uid`.
    #[must_use]
    pub fn uid(&self) -> &ObjectVersionId {
        &self.item.uid
    }

    /// `IMPORTED_VERSION.preceding_version_uid`, `None` for a first version.
    #[must_use]
    pub fn preceding_version_uid(&self) -> Option<&ObjectVersionId> {
        self.item.preceding_version_uid.as_ref()
    }

    /// `IMPORTED_VERSION.lifecycle_state`: the lifecycle state of the content
    /// in the wrapped `ORIGINAL_VERSION`, "derived as `_item.lifecycle_state_`".
    #[must_use]
    pub fn lifecycle_state(&self) -> &DvCodedText {
        &self.item.lifecycle_state
    }

    /// `IMPORTED_VERSION.data`, `None` for a logically deleted original.
    #[must_use]
    pub fn data(&self) -> Option<&T> {
        self.item.data.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::change_control::original_version::OriginalVersion;
    use crate::v1_2::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_base::v1_3::prelude::{
        HierObjectId, ObjectId, ObjectRef, ObjectRefData, TerminologyId,
    };
    use openehr_term::codes::{AuditChangeType, VersionLifecycleState};

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

    fn audit(system_id: &str) -> AuditDetails {
        AuditDetails::AuditDetails(AuditDetailsData {
            system_id: system_id.to_owned(),
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
        })
    }

    fn contribution(id: &str) -> ObjectRef {
        ObjectRef::ObjectRef(ObjectRefData {
            namespace: "local".to_owned(),
            r#type: "CONTRIBUTION".to_owned(),
            id: ObjectId::HierObjectId(
                HierObjectId::new(id.to_owned()).expect("a well-formed identifier"),
            ),
        })
    }

    fn version_id(value: &str) -> ObjectVersionId {
        ObjectVersionId::new(value.to_owned()).expect("a well-formed identifier")
    }

    /// The wrapped original: version 2 of a container created on
    /// `remote.example`, superseding version 1 there, carrying content.
    fn wrapped() -> OriginalVersion<String> {
        OriginalVersion {
            contribution: contribution("11111111-1111-4111-8111-111111111111"),
            signature: Some("foreign-signature".to_owned()),
            commit_audit: Box::new(audit("remote.example")),
            uid: version_id("8849182c-82ad-4088-a07f-48ead4180515::remote.example::2"),
            preceding_version_uid: Some(version_id(
                "8849182c-82ad-4088-a07f-48ead4180515::remote.example::1",
            )),
            other_input_version_uids: openehr_base::containers::present_nonempty(Vec::new()),
            lifecycle_state: coded(
                VersionLifecycleState::Complete.code(),
                VersionLifecycleState::Complete.rubric(),
            ),
            attestations: openehr_base::containers::present_nonempty(Vec::new()),
            data: Some("content".to_owned()),
        }
    }

    fn imported(item: OriginalVersion<String>) -> ImportedVersion<String> {
        ImportedVersion {
            contribution: contribution("22222222-2222-4222-8222-222222222222"),
            signature: Some("local-wrapper-signature".to_owned()),
            commit_audit: Box::new(audit("local.example")),
            item: Box::new(item),
        }
    }

    /// `Post: Result = item.uid` — the wrapper reports the wrapped original's
    /// identifier, never one of its own.
    #[test]
    fn uid_is_the_wrapped_originals_uid() {
        let item = wrapped();
        let iv = imported(item.clone());
        assert_eq!(iv.uid(), &item.uid);
        assert_eq!(
            iv.uid().value(),
            "8849182c-82ad-4088-a07f-48ead4180515::remote.example::2"
        );
    }

    /// `Post: Result = item.preceding_version_uid`.
    #[test]
    fn preceding_version_uid_is_the_wrapped_originals() {
        let item = wrapped();
        let iv = imported(item.clone());
        assert_eq!(
            iv.preceding_version_uid(),
            item.preceding_version_uid.as_ref()
        );
    }

    /// A wrapped FIRST version has no predecessor — the Void the abstract
    /// `VERSION.preceding_version_uid` documents.
    #[test]
    fn a_wrapped_first_version_has_no_preceding_uid() {
        let mut item = wrapped();
        item.uid = version_id("8849182c-82ad-4088-a07f-48ead4180515::remote.example::1");
        item.preceding_version_uid = None;
        assert_eq!(imported(item).preceding_version_uid(), None);
    }

    /// "derived as `_item.lifecycle_state_`".
    #[test]
    fn lifecycle_state_is_the_wrapped_originals() {
        let item = wrapped();
        let iv = imported(item.clone());
        assert_eq!(iv.lifecycle_state(), &item.lifecycle_state);
    }

    /// "Original content of this Version" — and Void for a wrapped original
    /// committed as a logical deletion (master06 §Contributions).
    #[test]
    fn data_is_the_wrapped_originals_content_or_void_when_deleted() {
        assert_eq!(imported(wrapped()).data(), Some(&"content".to_owned()));

        let mut deleted = wrapped();
        deleted.lifecycle_state = coded(
            VersionLifecycleState::Deleted.code(),
            VersionLifecycleState::Deleted.rubric(),
        );
        deleted.data = None;
        assert_eq!(imported(deleted).data(), None);
    }
}
