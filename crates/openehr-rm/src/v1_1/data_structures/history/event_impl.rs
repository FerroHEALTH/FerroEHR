// @generated-from-template templates/openehr-rm/data_structures/history/event_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for the abstract `EVENT<T>` enum.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.event.adoc`.
//! `EVENT.offset() = time.diff(parent.origin)` — the parent back-reference is
//! not stored (repo convention: no owning back-refs), so the offset is
//! computed from an explicitly supplied `HISTORY.origin`
//! ([`Event::offset_from`]); the `HISTORY` invariant checks call it with their
//! own origin.

use crate::v1_1::data_structures::history::event::Event;
use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
use crate::v1_1::data_types::quantity::date_time::dv_duration::DvDuration;

/// Offset in seconds of an event `time` from a history `origin`, when both
/// magnitudes are available.
pub(crate) fn offset_seconds(time: &DvDateTime, origin: &DvDateTime) -> Option<f64> {
    Some(time.magnitude()? - origin.magnitude()?)
}

impl<T> Event<T> {
    /// The event `time` (common to both `POINT_EVENT` and `INTERVAL_EVENT`).
    #[must_use]
    pub fn time(&self) -> &DvDateTime {
        match self {
            Self::IntervalEvent(e) => &e.time,
            Self::PointEvent(e) => &e.time,
        }
    }

    /// The event `data`.
    #[must_use]
    pub fn data(&self) -> &T {
        match self {
            Self::IntervalEvent(e) => &e.data,
            Self::PointEvent(e) => &e.data,
        }
    }

    /// RM `EVENT.offset()`: offset of this event from the history origin,
    /// `time.diff(origin)`, or `None` when either value is not a valid
    /// ISO-8601 date-time. The `parent` back-reference of the spec signature is
    /// supplied explicitly (no owning back-refs; see module doc).
    ///
    /// `DV_DATE_TIME.diff` is that operation, so this is that call — the
    /// duration is rendered by the BASE ISO-8601 type that owns durations
    /// rather than hand-formatted from a seconds count here.
    #[must_use]
    pub fn offset_from(&self, origin: &DvDateTime) -> Option<DvDuration> {
        self.time().diff(origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_structures::history::point_event::PointEvent;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::validate::Validate;

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

    fn event(time: &str) -> Event<i32> {
        Event::PointEvent(PointEvent {
            name: text("event"),
            archetype_node_id: "at0002".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            time: date_time(time),
            state: None,
            data: 1,
        })
    }

    #[test]
    fn offset_is_time_minus_origin() {
        let origin = date_time("2021-01-01T00:00:00");
        let e = event("2021-01-01T01:00:00");

        // The offset is asserted by MAGNITUDE, not by the exact string. The
        // class requires a `DV_DURATION`, and `PT1H` and `PT3600S` are the same
        // duration — no openEHR spec picks a rendering for a computed `diff`,
        // so pinning one pins whichever formatter produced it rather than
        // anything the spec requires.
        let offset = e.offset_from(&origin).expect("both values are well formed");
        assert_eq!(offset.magnitude(), Some(3600.0));
        assert!(
            offset.invariants().is_empty(),
            "the offset must be a valid ISO-8601 duration: {:?}",
            offset.value
        );

        // Event before origin → negative duration.
        let early = event("2020-12-31T23:59:00");
        let before = early
            .offset_from(&origin)
            .expect("both values are well formed");
        assert_eq!(before.magnitude(), Some(-60.0));
        assert!(before.invariants().is_empty(), "{:?}", before.value);

        // A malformed value on either side has no offset.
        assert!(event("bad").offset_from(&origin).is_none());
        assert!(e.offset_from(&date_time("bad")).is_none());
    }

    #[test]
    fn offset_unavailable_for_malformed_time() {
        let origin = date_time("not-a-time");
        assert!(event("2021-01-01T01:00:00").offset_from(&origin).is_none());
    }
}
