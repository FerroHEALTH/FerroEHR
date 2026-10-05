// @generated-from-template templates/openehr-rm/composition/event_context_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `EVENT_CONTEXT`.
//!
//! The class page
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.event_context.adoc`
//! §Invariants) declares three invariants; every one is enforced, each at the
//! layer its inputs live at:
//!
//! - `location_valid` (`location /= Void implies not location.is_empty`) —
//!   here, via the generated `event_context_core`.
//! - `Participations_validity` — by construction: the field emits
//!   `Option<NonEmptyVec<PARTICIPATION>>`, so a present-but-empty list is
//!   unrepresentable and the strict readers refuse `[]` at parse.
//! - `Setting_valid` — terminology-bound, enforced in `validate::terminology`
//!   against the `openehr-term` bundle (it needs the openEHR terminology
//!   group, which this layer does not hold).
//!
//! `start_time` presence is structural: the attribute is `1..1`, so it emits
//! a mandatory field.

use crate::v1_2::composition::event_context::EventContext;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for EventContext {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::event_context_core(self.location.as_deref(), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_3::prelude::TerminologyId;

    fn setting() -> DvCodedText {
        DvCodedText {
            value: "other care".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: "238".to_owned(),
                preferred_term: None,
            },
        }
    }

    fn context(location: Option<&str>) -> EventContext {
        EventContext {
            start_time: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T09:00:00".to_owned(),
            },
            end_time: None,
            location: location.map(str::to_owned),
            setting: setting(),
            other_context: None,
            health_care_facility: None,
            participations: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }

    #[test]
    fn valid_context() {
        assert!(context(Some("ward A3")).invariants().is_empty());
        assert!(context(None).invariants().is_empty());
    }

    #[test]
    fn empty_location_invalid() {
        assert_eq!(
            context(Some("")).invariants()[0].message,
            "Invariant location_valid failed on type EVENT_CONTEXT"
        );
    }
}
