// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `DV_DATE_TIME`.
//!
//! `Value_valid`: `value` is a valid (possibly partial) ISO-8601 date-time. Plus
//! the inherited DV_QUANTIFIED `Magnitude_status_valid`. See `dv_date_impl` for
//! the NOTE on why this is an explicit invariant.

use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::dv_absolute_quantity_impl::{
    difference_accuracy, displaced_accuracy,
};
use crate::v1_2::data_types::quantity::dv_amount_impl::CombinedAccuracy;
use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_2::validate::valid_iso8601_date_time;
use openehr_base::v1_3::foundation_types::time::iso8601_date_time::Iso8601DateTime;
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvDateTime {
    /// Addition of a duration to this date-time, or `None` when either value is
    /// not a valid ISO-8601 string.
    ///
    /// Spec: `dv_date_time.adoc` §Functions `add`, effecting
    /// `dv_absolute_quantity.adoc`'s abstract `add`, whose accuracy rule is
    /// realized by [`displaced_accuracy`].
    #[must_use]
    pub fn add(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().add(&duration_of(a_diff))?, a_diff))
    }

    /// Subtraction of a duration from this date-time, under the same conditions
    /// as [`Self::add`].
    ///
    /// Spec: `dv_date_time.adoc` §Functions `subtract`.
    #[must_use]
    pub fn subtract(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().subtract(&duration_of(a_diff))?, a_diff))
    }

    /// Difference between this date-time and `other`, as a duration.
    ///
    /// Spec: `dv_date_time.adoc` §Functions `diff`.
    #[must_use]
    pub fn diff(&self, other: &Self) -> Option<DvDuration> {
        let difference = self.iso().diff(&other.iso())?;
        let (accuracy, accuracy_is_percent) =
            match difference_accuracy(self.accuracy.as_ref(), other.accuracy.as_ref()) {
                CombinedAccuracy::Unrepresentable => return None,
                CombinedAccuracy::Unknown => (None, None),
                CombinedAccuracy::Known {
                    accuracy,
                    is_percent,
                } => (Some(accuracy), Some(is_percent)),
            };
        Some(DvDuration {
            value: difference.value,
            magnitude_status: None,
            accuracy,
            accuracy_is_percent,
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        })
    }

    /// This date-time's `value` as the BASE ISO-8601 type that owns the
    /// calendar.
    fn iso(&self) -> Iso8601DateTime {
        Iso8601DateTime {
            value: self.value.clone(),
        }
    }

    /// This date-time at a new value, carrying the displaced accuracy.
    fn displaced(&self, value: Iso8601DateTime, a_diff: &DvDuration) -> Self {
        Self {
            value: value.value,
            magnitude_status: None,
            accuracy: displaced_accuracy(self.accuracy.as_ref(), a_diff),
            normal_range: None,
            normal_status: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }
}

/// A `DV_DURATION`'s `value` as the BASE ISO-8601 type.
fn duration_of(duration: &DvDuration) -> Iso8601Duration {
    Iso8601Duration {
        value: duration.value.clone(),
    }
}

impl Validate for DvDateTime {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::temporal_value_core(
            "DV_DATE_TIME",
            valid_iso8601_date_time(&self.value),
            out,
        );
        crate::v1_2::validate::generated::magnitude_status_core(
            "DV_DATE_TIME",
            self.magnitude_status.as_deref(),
            out,
        );
        // Inherited DV_ORDERED Normal_range_and_status_consistency.
        push_normal_range_consistency(
            out,
            "DV_DATE_TIME",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            &DvOrdered::DvDateTime(self.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date_time(value: &str) -> DvDateTime {
        DvDateTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    #[test]
    fn valid_date_time() {
        assert!(date_time("2021-05-17T10:30:00").invariants().is_empty());
        assert!(date_time("2021-05-17T10:30:00Z").invariants().is_empty());
    }

    #[test]
    fn invalid_date_time() {
        let v = date_time("2021-05-17T99:00:00").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Value_valid failed on type DV_DATE_TIME")
        );
    }

    fn duration(value: &str) -> DvDuration {
        DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: value.to_owned(),
        }
    }

    /// Date-times displace across the day boundary and across the calendar,
    /// because BASE answers both in one type.
    #[test]
    fn arithmetic_crosses_the_day_and_the_calendar() {
        let evening = date_time("2024-02-28T23:00:00");
        assert_eq!(
            evening.add(&duration("PT2H")).expect("valid").value,
            "2024-02-29T01:00:00",
            "into the leap day"
        );
        assert_eq!(
            evening.subtract(&duration("P1D")).expect("valid").value,
            "2024-02-27T23:00:00"
        );
        assert_eq!(
            evening
                .diff(&date_time("2024-02-28T21:00:00"))
                .expect("valid")
                .magnitude(),
            Some(7200.0)
        );
    }

    /// A value that is not a valid ISO-8601 date-time has no arithmetic.
    #[test]
    fn an_invalid_value_has_no_arithmetic() {
        let bad = date_time("2021-05-17T99:00:00");
        assert!(bad.add(&duration("PT1H")).is_none());
        assert!(bad.diff(&date_time("2024-01-01T00:00:00")).is_none());
    }

    /// The `DV_ABSOLUTE_QUANTITY` accuracy rule, including a percentage
    /// half-range resolved against the differential amount's own magnitude.
    #[test]
    fn accuracy_follows_the_absolute_quantity_rule() {
        let mut precise = date_time("2024-01-01T00:00:00");
        precise.accuracy = Some(duration("PT5S"));
        let mut offset = duration("PT100S");
        offset.accuracy = Some(10.0);
        offset.accuracy_is_percent = Some(true);

        let moved = precise.add(&offset).expect("valid");
        assert_eq!(
            moved.accuracy.expect("both present").magnitude(),
            Some(15.0),
            "5s + 10% of 100s"
        );

        assert!(
            precise
                .add(&duration("PT100S"))
                .expect("valid")
                .accuracy
                .is_none()
        );
    }
}
