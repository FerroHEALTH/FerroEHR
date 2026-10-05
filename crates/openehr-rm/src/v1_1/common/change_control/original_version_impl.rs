// @generated-from-template templates/openehr-rm/common/change_control/original_version_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written spec function of `ORIGINAL_VERSION` (hand-written spec
//! behaviour).
//!
//! Spec: RM
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.original_version.adoc`
//! §Functions declares one — `is_merged (): Boolean`, "True if this Version was
//! created from more than just the preceding (checked out) version" — and pins
//! it against the merge-provenance attribute in §Invariants:
//! `Is_merged_validity`: `other_input_version_ids = Void xor is_merged`
//! (quoted verbatim, upstream's `_ids`/`_uids` spelling slip included; the
//! attribute the same class table declares is `other_input_version_uids`).
//!
//! The semantics come from RM common `master06-change_control_package.adoc`
//! §Version Merging: a merged version records "the ids of other versions merged
//! into the current one", so a version is merged exactly when that list holds
//! something.

use crate::v1_1::common::change_control::original_version::OriginalVersion;

impl<T> OriginalVersion<T> {
    /// `ORIGINAL_VERSION.is_merged`: whether this version was created from more
    /// than just its preceding version — the derived boolean of
    /// `other_input_version_uids`.
    ///
    /// `Is_merged_validity` (`other_input_version_ids = Void xor is_merged`,
    /// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.original_version.adoc`
    /// §Invariants) is stated against `Void`, and the optional-container
    /// emission shape (`Option<Vec<T>>`) carries `Void` directly — so this is
    /// the invariant read verbatim: merged iff the attribute is present. The
    /// companion `Other_input_version_uids_valid` (`/= Void implies not
    /// is_empty`) is realized separately, on the same value.
    #[must_use]
    pub fn is_merged(&self) -> bool {
        self.other_input_version_uids.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_2::prelude::{
        HierObjectId, ObjectId, ObjectRef, ObjectRefData, ObjectVersionId, TerminologyId,
    };

    fn version(other_input: Vec<&str>) -> OriginalVersion<String> {
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
                change_type: DvCodedText {
                    value: "modification".to_owned(),
                    hyperlink: None,
                    formatting: None,
                    mappings: openehr_base::containers::present_nonempty(Vec::new()),
                    language: None,
                    encoding: None,
                    defining_code: CodePhrase {
                        terminology_id: TerminologyId {
                            value: "openehr".to_owned(),
                        },
                        code_string: "251".to_owned(),
                        preferred_term: None,
                    },
                },
                description: None,
                committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            }),
            uid: ObjectVersionId::new(
                "8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::3".to_owned(),
            )
            .expect("a well-formed identifier"),
            preceding_version_uid: Some(
                ObjectVersionId::new(
                    "8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2".to_owned(),
                )
                .expect("a well-formed identifier"),
            ),
            other_input_version_uids: openehr_base::containers::present_nonempty(
                other_input
                    .into_iter()
                    .map(|value| {
                        ObjectVersionId::new(value.to_owned()).expect("a well-formed identifier")
                    })
                    .collect(),
            ),
            lifecycle_state: DvCodedText {
                value: "complete".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
                defining_code: CodePhrase {
                    terminology_id: TerminologyId {
                        value: "openehr".to_owned(),
                    },
                    code_string: "532".to_owned(),
                    preferred_term: None,
                },
            },
            attestations: openehr_base::containers::present_nonempty(Vec::new()),
            data: Some("content".to_owned()),
        }
    }

    /// `Is_merged_validity`: `other_input_version_ids = Void xor is_merged`.
    #[test]
    fn is_merged_is_the_merge_provenance_list_being_non_empty() {
        assert!(!version(Vec::new()).is_merged());
        assert!(
            version(vec![
                "8849182c-82ad-4088-a07f-48ead4180515::other.example::2.1.1"
            ])
            .is_merged()
        );
        assert!(
            version(vec![
                "8849182c-82ad-4088-a07f-48ead4180515::other.example::2.1.1",
                "8849182c-82ad-4088-a07f-48ead4180515::third.example::2.2.1",
            ])
            .is_merged()
        );
    }
}
