// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `REVISION_HISTORY`.
//!
//! Spec: RM
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.revision_history.adoc`
//! §Functions declares two, each pinned by a postcondition over `items`:
//!
//! - `most_recent_version ()`: "The version id of the most recent item, as a
//!   String" — `Post: Result.is_equal (items.last.version_id.value)`.
//! - `most_recent_version_time_committed ()`: "The commit date/time of the most
//!   recent item, as a String" —
//!   `Post: Result.is_equal (items.last.audits.first.time_committed.value)`.
//!
//! Both read the LAST item, because `items` is "The items in this history in
//! most-recent-last order" (§Attributes), and the second reads the FIRST audit
//! of that item, because `REVISION_HISTORY_ITEM.audits` holds "the commit audit
//! … there may also be further attestations"
//! (`…org.openehr.rm.common.revision_history_item.adoc` §Attributes).
//!
//! NOTE: both functions are `1..1` and stay total — `items` and
//! `REVISION_HISTORY_ITEM.audits` are `NonEmptyVec`, so the empty cases the
//! spec model forbids are unrepresentable rather than merely rejected.

use crate::v1_2::common::generic::audit_details::AuditDetails;
use crate::v1_2::common::generic::revision_history::RevisionHistory;

impl RevisionHistory {
    /// `REVISION_HISTORY.most_recent_version`: the version id of the most
    /// recent item (the last one — `items` is in most-recent-last order), as
    /// its `String` value.
    ///
    /// Returns `None` when the history holds no items, a state the spec model
    /// cannot express (see the module docs).
    #[must_use]
    pub fn most_recent_version(&self) -> Option<&str> {
        self.items.last().map(|i| i.version_id.value())
    }

    /// `REVISION_HISTORY.most_recent_version_time_committed`: the commit
    /// date/time of the most recent item — the `time_committed` of that item's
    /// FIRST audit, which is its commit audit — as its `String` value.
    ///
    /// Returns `None` only when `items` has no last element — `NonEmptyVec`
    /// offers no total `last`, unlike the `head` the audits are read through.
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
    use crate::v1_2::common::generic::attestation::Attestation;
    use crate::v1_2::common::generic::audit_details::AuditDetailsData;
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::common::generic::revision_history_item::RevisionHistoryItem;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::{ObjectVersionId, TerminologyId};

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
            change_type: change_type("249", "creation"),
            description: None,
            committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
        })
    }

    /// An ATTESTATION appended AFTER the commit audit — the second audit of an
    /// item (`revision_history_item.adoc` §Attributes: "there may also be
    /// further attestations"), whose later `time_committed` must NOT be the
    /// value the function reports.
    fn attestation(time: &str) -> AuditDetails {
        AuditDetails::Attestation(Attestation {
            system_id: "ferroehr.local".to_owned(),
            time_committed: dv_date_time(time),
            change_type: change_type("666", "attestation"),
            description: None,
            committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
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
        })
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

    /// The states this used to construct — an empty `items` and an item with no
    /// `audits` — are now UNREPRESENTABLE rather than merely reported as
    /// absent: `REVISION_HISTORY.items` and `REVISION_HISTORY_ITEM.audits` are
    /// both `1..*`
    /// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.revision_history.adoc`
    /// and `…revision_history_item.adoc` §Attributes), and the emission shape
    /// carries those bounds. Refusal at construction is strictly stronger than
    /// reporting `None`, so the assertions move to the refusal.
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
