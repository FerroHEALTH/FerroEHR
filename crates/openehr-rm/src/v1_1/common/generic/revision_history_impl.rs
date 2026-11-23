// @generated-from-template templates/openehr-rm/common/generic/revision_history_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `REVISION_HISTORY` functions
//! (`org.openehr.rm.common.revision_history.adoc` §Functions): both read the
//! last item, its first audit for the time.

use crate::v1_1::common::generic::audit_details::AuditDetails;
use crate::v1_1::common::generic::revision_history::RevisionHistory;

impl RevisionHistory {
    /// `REVISION_HISTORY.most_recent_version`: the last item's version id.
    #[must_use]
    pub fn most_recent_version(&self) -> Option<&str> {
        self.items.last().map(|i| i.version_id.value())
    }

    /// `REVISION_HISTORY.most_recent_version_time_committed`: the last item's
    /// first audit's `time_committed`.
    #[must_use]
    pub fn most_recent_version_time_committed(&self) -> Option<&str> {
        Some(match self.items.last()?.audits.head() {
            AuditDetails::AuditDetails(d) => d.time_committed.value.as_str(),
            AuditDetails::Attestation(a) => a.time_committed.value.as_str(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::generic::attestation::Attestation;
    use crate::v1_1::common::generic::audit_details::AuditDetailsData;
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::common::generic::revision_history_item::RevisionHistoryItem;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::{ObjectVersionId, TerminologyId};
    use openehr_term::codes::AuditChangeType;

    fn dv_date_time(value: &str) -> DvDateTime {
        DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    fn change_type(code: &str, rubric: &str) -> DvCodedText {
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

    fn commit_audit(time: &str) -> AuditDetails {
        AuditDetails::AuditDetails(AuditDetailsData {
            system_id: "ferroehr.local".to_owned(),
            time_committed: dv_date_time(time),
            change_type: change_type(
                AuditChangeType::Creation.code(),
                AuditChangeType::Creation.rubric(),
            ),
            description: None,
            committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
        })
    }

    /// An attestation after the commit audit, whose later time must not be
    /// reported.
    fn attestation(time: &str) -> AuditDetails {
        AuditDetails::Attestation(Box::new(Attestation {
            system_id: "ferroehr.local".to_owned(),
            time_committed: dv_date_time(time),
            change_type: change_type(
                AuditChangeType::Attestation.code(),
                AuditChangeType::Attestation.rubric(),
            ),
            description: None,
            committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            attested_view: None,
            proof: None,
            items: openehr_base::containers::present_nonempty(Vec::new()),
            reason: DvText::DvText(DvTextData {
                value: "witnessed".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
            }),
            is_pending: false,
        }))
    }

    fn item(version_id: &str, audits: Vec<AuditDetails>) -> RevisionHistoryItem {
        RevisionHistoryItem {
            version_id: ObjectVersionId::new(version_id.to_owned())
                .expect("a well-formed identifier"),
            audits: openehr_base::containers::NonEmptyVec::new(audits)
                .expect("a fixture container declared 1..* must have members"),
        }
    }

    fn history() -> RevisionHistory {
        RevisionHistory {
            items: nonempty(vec![
                item(
                    "8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::1",
                    vec![commit_audit("2026-07-07T10:11:12Z")],
                ),
                item(
                    "8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2",
                    vec![
                        commit_audit("2026-07-08T09:00:00Z"),
                        attestation("2026-07-09T09:00:00Z"),
                    ],
                ),
            ]),
        }
    }

    /// A fixture container the model declares `1..*`.
    fn nonempty<T>(members: Vec<T>) -> openehr_base::containers::NonEmptyVec<T> {
        openehr_base::containers::NonEmptyVec::new(members).expect("the fixture states members")
    }

    /// `Post: Result.is_equal (items.last.version_id.value)`.
    #[test]
    fn most_recent_version_is_the_last_items_version_id() {
        let h = history();
        assert_eq!(
            h.most_recent_version(),
            h.items.last().map(|i| i.version_id.value())
        );
        assert_eq!(
            h.most_recent_version(),
            Some("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2")
        );
    }

    /// `Post: Result.is_equal (items.last.audits.first.time_committed.value)`
    /// — the FIRST audit of the last item, never a later attestation's instant.
    #[test]
    fn most_recent_time_committed_is_the_last_items_first_audit() {
        let h = history();
        assert_eq!(
            h.most_recent_version_time_committed(),
            Some("2026-07-08T09:00:00Z")
        );
    }

    /// An empty `items` or `audits` cannot be constructed: both are `1..*`.
    #[test]
    fn the_spec_unrepresentable_shapes_are_refused_at_construction() {
        assert!(
            openehr_base::containers::NonEmptyVec::<RevisionHistoryItem>::new(Vec::new()).is_err(),
            "an empty REVISION_HISTORY.items must not be constructible"
        );
        assert!(
            openehr_base::containers::NonEmptyVec::<AuditDetails>::new(Vec::new()).is_err(),
            "an empty REVISION_HISTORY_ITEM.audits must not be constructible"
        );
    }

    /// A populated history still reports its most recent version and that
    /// version's commit time (the behaviour the removed absence cases framed).
    #[test]
    fn a_populated_history_reports_its_most_recent_version() {
        let h = history();
        assert_eq!(
            h.most_recent_version(),
            Some("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::2")
        );
        assert!(h.most_recent_version_time_committed().is_some());
    }
}
