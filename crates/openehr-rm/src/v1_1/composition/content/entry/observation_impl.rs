// @generated-from-template templates/openehr-rm/composition/content/entry/observation_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `OBSERVATION`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.entry.adoc`
//! §Invariants (`OBSERVATION` declares no invariant of its own) +
//! `…org.openehr.rm.common.locatable.adoc` §Invariants. Enforced here:
//! - `Is_archetype_root` (`is_archetype_root`): an ENTRY is an archetype root,
//!   so `archetype_details` must be present.
//! - the inherited LOCATABLE `Archetype_node_id_valid`: `archetype_node_id`
//!   non-empty.
//!
//! The other three ENTRY invariants need no check in this crate:
//! `Language_valid` / `Encoding_valid` are code-set-bound and are enforced in
//! the terminology-aware path (`validate::terminology`, the ENTRY-subtype
//! slots) against the `openehr-term` bundle; `Other_participations_valid`
//! (`other_participations /= Void implies not …is_empty`) is unrepresentable
//! against an `Option<NonEmptyVec<…>>` field (`openehr_base::containers`); and
//! `Subject_validity` restates the §Functions post-condition of
//! `subject_is_self`, whose only ground here is `subject`'s own variant, so
//! there is nothing independent left to compare.

use crate::v1_1::composition::content::entry::observation::Observation;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Observation {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::entry_root_core(
            "OBSERVATION",
            self.archetype_details.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::archetyped::archetyped::Archetyped;
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_structures::history::history::History;
    use crate::v1_1::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::{ArchetypeId, TerminologyId};

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

    fn data() -> History<ItemStructure> {
        History {
            name: text("history"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            origin: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T00:00:00".to_owned(),
            },
            period: None,
            duration: None,
            summary: None,
            events: openehr_base::containers::present(Vec::new()),
        }
    }

    fn observation() -> Observation {
        Observation {
            name: text("BP"),
            archetype_node_id: "openEHR-EHR-OBSERVATION.bp.v1".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: Some(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-OBSERVATION.bp.v1".to_owned(),
                },
                template_id: None,
                rm_version: "1.1.0".to_owned(),
            }),
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            encoding: code("IANA_character-sets", "UTF-8"),
            other_participations: openehr_base::containers::present_nonempty(Vec::new()),
            workflow_id: None,
            subject: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            provider: None,
            protocol: None,
            guideline_id: None,
            data: data(),
            state: None,
        }
    }

    #[test]
    fn valid_observation() {
        assert!(observation().invariants().is_empty());
    }

    #[test]
    fn missing_archetype_details_invalid() {
        let mut o = observation();
        o.archetype_details = None;
        let v = o.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Is_archetype_root failed on type OBSERVATION"),
            "got {v:?}"
        );
    }
}
