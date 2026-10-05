// @generated-from-template templates/openehr-rm/data_types/quantity/date_time/dv_temporal_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `DV_TEMPORAL`.
//!
//! `DV_TEMPORAL` is abstract, so the generated `DvTemporal` is the closed
//! subtype enum over `DV_DATE` / `DV_TIME` / `DV_DATE_TIME`, and the three
//! functions it declares are realized here as the dispatch across it. The
//! arithmetic itself is not re-derived: each concrete type already effects
//! `add` / `subtract` / `diff` over the BASE ISO-8601 calendar, including the
//! accuracy rule `DV_ABSOLUTE_QUANTITY` states.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_temporal.adoc`
//! §Functions — the class "whose diff type is `DV_DURATION`", which is what
//! narrows its inherited `DV_ABSOLUTE_QUANTITY` signatures from any
//! `DV_AMOUNT` to a duration.

use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::date_time::dv_temporal::DvTemporal;

impl DvTemporal {
    /// Returns this temporal value displaced forward by `a_diff`, or `None`
    /// when the value or the duration is not a readable ISO-8601 string.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_temporal.adoc` §Functions `add`
    /// (alias `+`) — "Addition of a Duration to this temporal entity."
    #[must_use]
    pub fn add(&self, a_diff: &DvDuration) -> Option<Self> {
        match self {
            Self::DvDate(value) => value.add(a_diff).map(Self::DvDate),
            Self::DvDateTime(value) => value.add(a_diff).map(Self::DvDateTime),
            Self::DvTime(value) => value.add(a_diff).map(Self::DvTime),
        }
    }

    /// Returns this temporal value displaced backward by `a_diff`, under the
    /// same conditions as [`Self::add`].
    ///
    /// Spec: `org.openehr.rm.data_types.dv_temporal.adoc` §Functions
    /// `subtract` (alias `-`) — "Subtract a Duration from this temporal
    /// entity."
    #[must_use]
    pub fn subtract(&self, a_diff: &DvDuration) -> Option<Self> {
        match self {
            Self::DvDate(value) => value.subtract(a_diff).map(Self::DvDate),
            Self::DvDateTime(value) => value.subtract(a_diff).map(Self::DvDateTime),
            Self::DvTime(value) => value.subtract(a_diff).map(Self::DvTime),
        }
    }

    /// Returns the duration between this temporal value and `other`, or `None`
    /// when the two are not the same concrete type or a value is unreadable.
    ///
    /// Spec: `org.openehr.rm.data_types.dv_temporal.adoc` §Functions `diff`
    /// (alias `-`) — "Difference between this temporal entity and `other`."
    /// Only two values of the same concrete type have one: the difference
    /// between a date and a time of day is not a duration, which is why the
    /// mixed pairs answer `None` rather than a number
    /// (`org.openehr.rm.data_types.dv_absolute_quantity.adoc` §Functions
    /// `diff`, "Difference of two quantities").
    #[must_use]
    pub fn diff(&self, other: &Self) -> Option<DvDuration> {
        match (self, other) {
            (Self::DvDate(left), Self::DvDate(right)) => left.diff(right),
            (Self::DvDateTime(left), Self::DvDateTime(right)) => left.diff(right),
            (Self::DvTime(left), Self::DvTime(right)) => left.diff(right),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::quantity::date_time::dv_date::DvDate;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::quantity::date_time::dv_time::DvTime;

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

    fn date(value: &str) -> DvTemporal {
        DvTemporal::DvDate(DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        })
    }

    fn date_time(value: &str) -> DvTemporal {
        DvTemporal::DvDateTime(DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        })
    }

    fn time(value: &str) -> DvTemporal {
        DvTemporal::DvTime(DvTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        })
    }

    /// The concrete date inside a temporal value.
    fn as_date(value: &DvTemporal) -> Option<&DvDate> {
        match value {
            DvTemporal::DvDate(date) => Some(date),
            DvTemporal::DvDateTime(_) | DvTemporal::DvTime(_) => None,
        }
    }

    /// The concrete date-time inside a temporal value.
    fn as_date_time(value: &DvTemporal) -> Option<&DvDateTime> {
        match value {
            DvTemporal::DvDateTime(date_time) => Some(date_time),
            DvTemporal::DvDate(_) | DvTemporal::DvTime(_) => None,
        }
    }

    /// A bare date, for comparing an arithmetic result against.
    fn plain_date(value: &str) -> DvDate {
        DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    /// The dispatch keeps the concrete type: displacing a date yields a date,
    /// on the calendar day that type's own `add` lands on.
    #[test]
    fn adding_a_duration_keeps_the_concrete_type() {
        let moved = date("2024-01-31")
            .add(&duration("P1D"))
            .expect("a readable date and duration");
        let moved = as_date(&moved).expect("a displaced date is a date");
        assert!(moved.is_equal(&plain_date("2024-02-01")), "{moved:?}");
    }

    /// A displaced date-time moves forward in time — the same displacement its
    /// own type effects, reached through the abstract dispatch.
    #[test]
    fn a_displaced_date_time_moves_forward() {
        let start = date_time("2024-01-31T23:00:00Z");
        let moved = start.add(&duration("PT2H")).expect("a readable date-time");
        let start = as_date_time(&start).expect("a date-time");
        let moved = as_date_time(&moved).expect("a displaced date-time is a date-time");
        assert_eq!(start.less_than(moved), Some(true));
        assert_eq!(moved.less_than(start), Some(false));
    }

    /// Subtraction is the inverse displacement, over the same calendar — 2024
    /// is a leap year, so the day before the first of March is the 29th.
    #[test]
    fn subtracting_a_duration_is_the_inverse_displacement() {
        let back = date("2024-03-01")
            .subtract(&duration("P1D"))
            .expect("a readable date and duration");
        let back = as_date(&back).expect("a displaced date is a date");
        assert!(back.is_equal(&plain_date("2024-02-29")), "{back:?}");
    }

    /// A difference exists only within one concrete type — a date minus a time
    /// of day is not a duration.
    #[test]
    fn a_difference_needs_two_values_of_one_type() {
        assert!(date("2024-01-02").diff(&date("2024-01-01")).is_some());
        assert!(date("2024-01-02").diff(&time("10:00:00")).is_none());
        assert!(
            time("11:00:00")
                .diff(&date_time("2024-01-01T00:00:00Z"))
                .is_none()
        );
    }

    /// An unreadable value has no displacement to report, and says so rather
    /// than inventing one.
    #[test]
    fn an_unreadable_value_has_no_arithmetic() {
        assert!(date("the fifth of never").add(&duration("P1D")).is_none());
        assert!(date("2024-01-01").add(&duration("one day")).is_none());
    }
}
