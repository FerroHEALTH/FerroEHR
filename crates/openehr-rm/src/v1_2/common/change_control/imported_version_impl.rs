// @generated-from-template templates/openehr-rm/common/change_control/imported_version_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written spec functions of `IMPORTED_VERSION` (hand-written spec
//! behaviour).
//!
//! Spec: RM
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.imported_version.adoc`
//! §Functions. `IMPORTED_VERSION` "acts as a wrapper of an `ORIGINAL_VERSION`
//! … Its `_uid_` and `_preceding_version_` are defined as functions, returning
//! the corresponding attribute values from the wrapped `ORIGINAL_VERSION`
//! object" (RM common `master06-change_control_package.adoc` §Version and its
//! Subtypes), and the class table effects four of `VERSION`'s abstract
//! functions as delegations over `item`:
//!
//! - `uid ()` — `Post: Result = item.uid`;
//! - `preceding_version_uid ()` — `Post: Result = item.preceding_version_uid`;
//! - `lifecycle_state ()` — "derived as `_item.lifecycle_state_`";
//! - `data ()` — "Original content of this Version".
//!
//! Each is a pure read of the wrapped original, so each is realized here as a
//! borrow rather than a clone: the delegation is identity, and a caller that
//! needs an owned value clones at its own site.
//!
//! NOTE: two functions the class table types `1..1` delegate to
//! `ORIGINAL_VERSION` attributes the same spec declares `0..1`
//! (`…original_version.adoc` §Attributes; "Void if this is the first
//! version"; a deleted version's data "set to Void", master06
//! §Contributions) — so both return `Option`, reporting the Void the wrapped
//! original may genuinely hold instead of fabricating a value.

use crate::v1_2::common::change_control::imported_version::ImportedVersion;
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_base::v1_3::prelude::ObjectVersionId;

impl<T> ImportedVersion<T> {
    /// `IMPORTED_VERSION.uid`: the wrapped original's own version identifier —
    /// an imported version "does not have its own version identifier distinct
    /// from the version it is wrapping" (`Post: Result = item.uid`).
    #[must_use]
    pub fn uid(&self) -> &ObjectVersionId {
        &self.item.uid
    }

    /// `IMPORTED_VERSION.preceding_version_uid`: the wrapped original's
    /// predecessor (`Post: Result = item.preceding_version_uid`).
    ///
    /// `None` when the wrapped original is a first version (see the module
    /// docs).
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

    /// `IMPORTED_VERSION.data`: the original content of this Version — the
    /// wrapped original's own data.
    ///
    /// `None` for a logically deleted wrapped original, whose data is Void (see
    /// the module docs).
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
            change_type: coded("249", "creation"),
            description: None,
            committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
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
            commit_audit: audit("remote.example"),
            uid: version_id("8849182c-82ad-4088-a07f-48ead4180515::remote.example::2"),
            preceding_version_uid: Some(version_id(
                "8849182c-82ad-4088-a07f-48ead4180515::remote.example::1",
            )),
            other_input_version_uids: openehr_base::containers::present_nonempty(Vec::new()),
            lifecycle_state: coded("532", "complete"),
            attestations: openehr_base::containers::present_nonempty(Vec::new()),
            data: Some("content".to_owned()),
        }
    }

    fn imported(item: OriginalVersion<String>) -> ImportedVersion<String> {
        ImportedVersion {
            contribution: contribution("22222222-2222-4222-8222-222222222222"),
            signature: Some("local-wrapper-signature".to_owned()),
            commit_audit: audit("local.example"),
            item,
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
        deleted.lifecycle_state = coded("523", "deleted");
        deleted.data = None;
        assert_eq!(imported(deleted).data(), None);
    }
}
