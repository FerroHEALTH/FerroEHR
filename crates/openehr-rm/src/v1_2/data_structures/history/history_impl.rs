// @generated-from-template templates/openehr-rm/data_structures/history/history_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants + functions for `HISTORY`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.history.adoc`
//! §Invariants:
//! - `Events_valid` (`(events /= Void and then not events.is_empty) or summary
//!   /= Void`): at least one event, or a summary, must be present.
//! - `Period_consistency`: `is_periodic implies events.for_all (e |
//!   e.offset.to_seconds mod period.to_seconds = 0)` — checked over the
//!   available temporal magnitudes (an event whose `time` or a `period` whose
//!   value is malformed runs no check; well-formedness is the value's own
//!   `Value_valid` concern).
//! - `is_periodic()`: `period` is set (`Periodic_validity` makes the two
//!   equivalent).
//! - Inherited LOCATABLE `Archetype_node_id_valid`.
//!
//! `Periodic_validity` (`is_periodic xor period = Void`, same §Invariants
//! section) is unfalsifiable here: `is_periodic()` is computed from `period`
//! rather than stored, so the two sides of the xor can never disagree.

use crate::v1_2::data_structures::history::event_impl::offset_seconds;
use crate::v1_2::data_structures::history::history::History;
use openehr_base::validate::{InvariantViolation, Validate};

/// Tolerance (seconds) for the periodicity modulo test — absorbs f64 rounding
/// from the nominal-seconds conversion.
const PERIOD_EPSILON: f64 = 1e-6;

impl<T> History<T> {
    /// RM `HISTORY.is_periodic()`: whether this history is periodic, i.e. has
    /// a `period` (`Periodic_validity`).
    #[must_use]
    pub fn is_periodic(&self) -> bool {
        self.period.is_some()
    }
}

impl<T> Validate for History<T> {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Events_valid + inherited Archetype_node_id_valid via the generated
        // core; Period_consistency stays typed-only (needs the event-offset
        // arithmetic below, which the fast path declines to reproduce).
        crate::v1_2::validate::generated::history_basic_core(
            self.events.as_ref().is_none_or(Vec::is_empty),
            self.summary.is_some(),
            &self.archetype_node_id,
            out,
        );

        // Period_consistency: every event offset is a whole multiple of period.
        if let Some(period) = self.period.as_ref()
            && let Some(period_secs) = period.magnitude()
            && period_secs > 0.0
        {
            let violated = self.events.iter().flatten().any(|e| {
                offset_seconds(e.time(), &self.origin).is_some_and(|offset| {
                    let rem = offset.rem_euclid(period_secs);
                    rem.min(period_secs - rem) > PERIOD_EPSILON
                })
            });
            if violated {
                out.push(InvariantViolation::here(
                    "Invariant Period_consistency failed on type HISTORY",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_structures::history::event::Event;
    use crate::v1_2::data_structures::history::point_event::PointEvent;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

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

    fn origin() -> DvDateTime {
        DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            value: "2021-01-01T00:00:00".to_owned(),
        }
    }

    fn history(events: Vec<Event<i32>>) -> History<i32> {
        History {
            name: text("history"),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            origin: origin(),
            period: None,
            duration: None,
            summary: None,
            events: openehr_base::containers::present(events),
        }
    }

    #[test]
    fn history_with_event_valid() {
        let event = Event::PointEvent(PointEvent {
            name: text("event"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            time: origin(),
            state: None,
            data: 1,
        });
        assert!(history(vec![event]).invariants().is_empty());
    }

    #[test]
    fn empty_history_invalid() {
        let v = history(vec![]).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Events_valid failed on type HISTORY")
        );
    }

    fn event_at(time: &str) -> Event<i32> {
        Event::PointEvent(PointEvent {
            name: text("event"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            time: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: time.to_owned(),
            },
            state: None,
            data: 1,
        })
    }

    fn periodic_history(period: &str, events: Vec<Event<i32>>) -> History<i32> {
        use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
        let mut h = history(events);
        h.period = Some(DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: period.to_owned(),
        });
        h
    }

    #[test]
    fn is_periodic_reflects_period() {
        assert!(!history(vec![event_at("2021-01-01T00:00:00")]).is_periodic());
        assert!(periodic_history("PT1H", vec![event_at("2021-01-01T00:00:00")]).is_periodic());
    }

    #[test]
    fn period_consistency_holds_for_aligned_events() {
        // Origin 00:00, hourly period, events at exact multiples (missing
        // events are allowed by the spec).
        let h = periodic_history(
            "PT1H",
            vec![
                event_at("2021-01-01T00:00:00"),
                event_at("2021-01-01T02:00:00"),
                event_at("2021-01-01T05:00:00"),
            ],
        );
        assert!(h.invariants().is_empty());
    }

    #[test]
    fn period_consistency_fails_for_misaligned_event() {
        let h = periodic_history(
            "PT1H",
            vec![
                event_at("2021-01-01T01:00:00"),
                event_at("2021-01-01T01:30:00"),
            ],
        );
        let v = h.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Period_consistency failed on type HISTORY"),
            "got {v:?}"
        );
    }

    #[test]
    fn period_consistency_skips_malformed_times() {
        // A malformed event time is a Value_valid problem, not periodicity.
        let h = periodic_history("PT1H", vec![event_at("garbage")]);
        assert!(h.invariants().is_empty());
    }
}
