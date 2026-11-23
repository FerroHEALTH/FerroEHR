// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `OBSERVATION` invariants (`org.openehr.rm.composition.entry.adoc`):
//! `Is_archetype_root` and `Archetype_node_id_valid`. `Language_valid` and
//! `Encoding_valid` are in `validate::terminology`;
//! `Other_participations_valid` holds by the `NonEmptyVec` type.

use crate::v1_2::composition::content::entry::observation::Observation;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Observation {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::entry_root_core(
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
    use crate::v1_2::common::archetyped::archetyped::Archetyped;
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::data_structures::history::history::History;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::{ArchetypeId, TerminologyId};

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
            archetype_details: Some(Box::new(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-OBSERVATION.bp.v1".to_owned(),
                },
                template_id: None,
                rm_version: "1.1.0".to_owned(),
            })),
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            encoding: code("IANA_character-sets", "UTF-8"),
            other_participations: openehr_base::containers::present_nonempty(Vec::new()),
            workflow_id: None,
            subject: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            provider: None,
            protocol: None,
            guideline_id: None,
            data: Box::new(data()),
            state: None,
        }
    }

    #[test]
    fn valid_observation() {
        assert_eq!(observation().invariants(), Vec::<InvariantViolation>::new());
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
