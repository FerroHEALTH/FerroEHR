// @generated-from-template templates/openehr-rm/composition/content/entry/entry_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `ENTRY`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.entry.adoc`
//! §Functions + §Invariants. `ENTRY` is abstract, so the generated `Entry` is
//! the closed subtype enum and the function dispatches over the five concrete
//! entry types.

use crate::v1_2::common::generic::party_proxy::PartyProxy;
use crate::v1_2::composition::content::entry::entry::Entry;

impl Entry {
    /// Returns `true` when this entry is about the subject of the EHR.
    ///
    /// Spec: `org.openehr.rm.composition.entry.adoc` §Functions
    /// `subject_is_self` — "Returns True if this Entry is about the subject of
    /// the EHR, in which case the subject attribute is of type `PARTY_SELF`"
    /// (`Post_condition: Result implies subject.generating_type =
    /// "PARTY_SELF"`, restated as the §Invariants `Subject_validity`). The
    /// generating type of `subject` is therefore the whole test: `PARTY_SELF`
    /// is the "party proxy representing the subject of the record"
    /// (`org.openehr.rm.common.party_self.adoc` §Description), so a
    /// `PARTY_IDENTIFIED` subject is by construction someone else.
    #[must_use]
    pub fn subject_is_self(&self) -> bool {
        matches!(self.subject(), PartyProxy::PartySelf(_))
    }

    /// The subject of this entry, whichever concrete entry type carries it.
    ///
    /// Spec: `org.openehr.rm.composition.entry.adoc` §Attributes `subject`.
    #[must_use]
    pub fn subject(&self) -> &PartyProxy {
        match self {
            Self::Action(entry) => &entry.subject,
            Self::AdminEntry(entry) => &entry.subject,
            Self::Evaluation(entry) => &entry.subject,
            Self::Instruction(entry) => &entry.subject,
            Self::Observation(entry) => &entry.subject,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::composition::content::entry::admin_entry::AdminEntry;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_2::data_structures::item_structure::item_tree::ItemTree;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    fn code(terminology: &str, code: &str) -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: terminology.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    fn entry(subject: PartyProxy) -> Entry {
        Entry::AdminEntry(AdminEntry {
            name: text("admin"),
            archetype_node_id: "openEHR-EHR-ADMIN_ENTRY.admin.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            encoding: code("IANA_character-sets", "UTF-8"),
            other_participations: None,
            workflow_id: None,
            subject,
            provider: None,
            data: ItemStructure::ItemTree(Box::new(ItemTree {
                name: text("tree"),
                archetype_node_id: "at0001".to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                items: None,
            })),
        })
    }

    /// `Result implies subject.generating_type = "PARTY_SELF"`: true exactly
    /// for a `PARTY_SELF` subject, both with and without an `external_ref`
    /// (which `party_self.adoc` says "may or may not" be set).
    #[test]
    fn a_party_self_subject_is_the_record_subject() {
        assert!(entry(PartyProxy::PartySelf(PartySelf { external_ref: None })).subject_is_self());
    }

    /// An identified subject is someone other than the record's owner, so the
    /// post-condition's antecedent must be false.
    #[test]
    fn an_identified_subject_is_not_the_record_subject() {
        let other = entry(PartyProxy::PartyIdentified(
            PartyIdentified::PartyIdentified(PartyIdentifiedData {
                external_ref: None,
                name: Some("Relative of the subject".to_owned()),
                identifiers: None,
            }),
        ));
        assert!(!other.subject_is_self());
    }
}
