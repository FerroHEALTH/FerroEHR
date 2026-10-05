// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `EXTRACT_UPDATE_SPEC`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr_extract.extract_update_spec.adoc`
//! §Invariants — `Overall_validity`
//! (`repeat_period /= Void or trigger_events /= Void`), evaluated by the
//! generated core; `Trigger_events_validity` holds by construction
//! (`Option<NonEmptyVec>`). `Send_changes_only_validity` invokes
//! `send_changes_only`, an attribute the class does NOT declare (the class
//! carries `update_method: CODE_PHRASE`; only the intro prose speaks of "the
//! `send_changes_only` flag") — an upstream defect, adjudicated `Excluded`
//! in the generated register and reported.

use crate::v1_2::ehr_extract::common::extract_update_spec::ExtractUpdateSpec;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for ExtractUpdateSpec {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // `Trigger_events_validity` holds by construction:
        // `trigger_events` is `Option<NonEmptyVec<..>>`, so a present list is
        // non-empty. Only `Overall_validity` needs a runnable check.
        crate::v1_2::validate::generated::extract_update_spec_core(
            self.repeat_period.is_some(),
            self.trigger_events.is_some(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(
        repeat: bool,
        triggers: Option<
            openehr_base::containers::NonEmptyVec<
                crate::v1_2::data_types::text::dv_coded_text::DvCodedText,
            >,
        >,
    ) -> ExtractUpdateSpec {
        ExtractUpdateSpec {
            persist_in_server: true,
            repeat_period: repeat.then(|| {
                crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration {
                    value: "P1D".to_owned(),
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: None,
                    magnitude_status: None,
                    accuracy: None,
                    accuracy_is_percent: None,
                }
            }),
            trigger_events: triggers,
            update_method: crate::v1_2::data_types::text::code_phrase::CodePhrase {
                terminology_id:
                    openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId {
                        value: "openehr".to_owned(),
                    },
                code_string: "999".to_owned(),
                preferred_term: None,
            },
        }
    }

    #[test]
    fn neither_mode_is_a_violation_and_either_passes() {
        let v = spec(false, None).invariants();
        assert!(
            v.iter().any(|m| m.message.contains("Overall_validity")),
            "got {v:?}"
        );
        assert!(spec(true, None).invariants().is_empty());
    }
}
