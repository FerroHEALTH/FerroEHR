// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `INTERVAL_EVENT.interval_start_time` (RM 1.2.0
//! `org.openehr.rm.data_structures.interval_event.adoc`) and the inherited
//! `Archetype_node_id_valid`; `Math_function_validity` is terminology-bound.

use crate::v1_2::data_structures::history::interval_event::IntervalEvent;
use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_base::validate::{InvariantViolation, Validate};

impl<T> IntervalEvent<T> {
    /// RM `INTERVAL_EVENT.interval_start_time()`: `time - width`, or `None` when
    /// either is not a valid ISO 8601 value.
    #[must_use]
    pub fn interval_start_time(&self) -> Option<DvDateTime> {
        self.time.subtract(&self.width)
    }
}

impl<T> Validate for IntervalEvent<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::archetype_node_id_core(
            "INTERVAL_EVENT",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::TerminologyId;

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

    fn date_time(value: &str) -> DvDateTime {
        DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    fn duration(value: &str) -> DvDuration {
        DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: value.to_owned(),
        }
    }

    fn interval_event(time: &str, width: &str) -> IntervalEvent<i32> {
        IntervalEvent {
            name: text("event"),
            archetype_node_id: "at0003".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            time: date_time(time),
            state: None,
            data: 1,
            width: duration(width),
            sample_count: None,
            math_function: DvCodedText {
                value: "mean".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
                defining_code: CodePhrase {
                    terminology_id: TerminologyId {
                        value: "openehr".to_owned(),
                    },
                    code_string: "146".to_owned(),
                    preferred_term: None,
                },
            },
        }
    }

    #[test]
    fn interval_start_time_is_time_minus_width() {
        let e = interval_event("2021-05-17T10:00:00", "PT1H");
        assert_eq!(
            e.interval_start_time().map(|t| t.value),
            Some("2021-05-17T09:00:00".to_owned())
        );
        // Width crossing a day boundary.
        let e = interval_event("2021-05-17T00:30:00", "PT1H");
        assert_eq!(
            e.interval_start_time().map(|t| t.value),
            Some("2021-05-16T23:30:00".to_owned())
        );
        // The original timezone suffix is preserved.
        let e = interval_event("2021-05-17T10:00:00+02:00", "PT30M");
        assert_eq!(
            e.interval_start_time().map(|t| t.value),
            Some("2021-05-17T09:30:00+02:00".to_owned())
        );
    }

    #[test]
    fn interval_start_time_unavailable_for_malformed_parts() {
        assert!(
            interval_event("bad", "PT1H")
                .interval_start_time()
                .is_none()
        );
        assert!(
            interval_event("2021-05-17T10:00:00", "bad")
                .interval_start_time()
                .is_none()
        );
    }

    #[test]
    fn archetype_node_id_checked() {
        let mut e = interval_event("2021-05-17T10:00:00", "PT1H");
        e.archetype_node_id = String::new();
        let v = e.invariants();
        assert!(v.iter().any(
            |m| m.message == "Invariant Archetype_node_id_valid failed on type INTERVAL_EVENT"
        ));
    }
}
