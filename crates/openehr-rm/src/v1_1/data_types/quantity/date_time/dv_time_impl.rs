// @generated-from-template templates/openehr-rm/data_types/quantity/date_time/dv_time_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `DV_TIME`.
//!
//! `Value_valid`: `value` is a valid (possibly partial) ISO-8601 time. Plus the
//! inherited DV_QUANTIFIED `Magnitude_status_valid`. See `dv_date_impl` for the
//! NOTE on why this is an explicit invariant.

use crate::v1_1::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_1::data_types::quantity::date_time::dv_time::DvTime;
use crate::v1_1::data_types::quantity::dv_absolute_quantity_impl::{
    difference_accuracy, displaced_accuracy,
};
use crate::v1_1::data_types::quantity::dv_amount_impl::CombinedAccuracy;
use crate::v1_1::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_1::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_1::validate::valid_iso8601_time;
use openehr_base::v1_2::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::v1_2::foundation_types::time::iso8601_time::Iso8601Time;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvTime {
    /// Addition of a duration to this time, or `None` when either value is not
    /// a valid ISO-8601 string.
    ///
    /// Spec: `dv_time.adoc` §Functions `add` — "Addition of a Duration to this
    /// Time" — effecting `dv_absolute_quantity.adoc`'s abstract `add`, whose
    /// accuracy rule is realized by [`displaced_accuracy`].
    #[must_use]
    pub fn add(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().add(&duration_of(a_diff))?, a_diff))
    }

    /// Subtraction of a duration from this time, under the same conditions as
    /// [`Self::add`].
    ///
    /// Spec: `dv_time.adoc` §Functions `subtract`.
    #[must_use]
    pub fn subtract(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().subtract(&duration_of(a_diff))?, a_diff))
    }

    /// Difference between this time and `other`, as a duration.
    ///
    /// Spec: `dv_time.adoc` §Functions `diff` — "Difference between this Time
    /// and `other`."
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

    /// This time's `value` as the BASE ISO-8601 type that owns the arithmetic.
    fn iso(&self) -> Iso8601Time {
        Iso8601Time {
            value: self.value.clone(),
        }
    }

    /// This time at a new value, carrying the displaced accuracy.
    fn displaced(&self, value: Iso8601Time, a_diff: &DvDuration) -> Self {
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

impl Validate for DvTime {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::temporal_value_core(
            "DV_TIME",
            valid_iso8601_time(&self.value),
            out,
        );
        crate::v1_1::validate::generated::magnitude_status_core(
            "DV_TIME",
            self.magnitude_status.as_deref(),
            out,
        );
        // Inherited DV_ORDERED Normal_range_and_status_consistency.
        push_normal_range_consistency(
            out,
            "DV_TIME",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            &DvOrdered::DvTime(self.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(value: &str) -> DvTime {
        DvTime {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    #[test]
    fn valid_time() {
        assert!(time("10:30:00").invariants().is_empty());
        assert!(time("10:30:00+01:00").invariants().is_empty());
    }

    #[test]
    fn invalid_time() {
        let v = time("25:99").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Value_valid failed on type DV_TIME")
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

    /// Times displace by a duration and difference into one.
    #[test]
    fn arithmetic_displaces_within_the_day() {
        let noon = time("12:00:00");
        assert_eq!(
            noon.add(&duration("PT90M")).expect("valid").magnitude(),
            Some(48_600.0)
        );
        assert_eq!(
            noon.subtract(&duration("PT30M"))
                .expect("valid")
                .magnitude(),
            Some(41_400.0)
        );
        assert_eq!(
            noon.diff(&time("11:00:00")).expect("valid").magnitude(),
            Some(3600.0)
        );
    }

    /// A value that is not a valid ISO-8601 time has no arithmetic.
    #[test]
    fn an_invalid_value_has_no_arithmetic() {
        let bad = time("25:99");
        assert!(bad.add(&duration("PT1H")).is_none());
        assert!(bad.diff(&time("12:00:00")).is_none());
    }

    /// The `DV_ABSOLUTE_QUANTITY` accuracy rule: summed when both operands
    /// record one, unknown when either does not.
    #[test]
    fn accuracy_follows_the_absolute_quantity_rule() {
        let mut precise = time("12:00:00");
        precise.accuracy = Some(duration("PT10S"));
        let mut offset = duration("PT1H");
        offset.accuracy = Some(5.0);
        offset.accuracy_is_percent = Some(false);

        let moved = precise.add(&offset).expect("valid");
        assert_eq!(
            moved.accuracy.expect("both present").magnitude(),
            Some(15.0)
        );

        // `diff` reports the summed accuracy as the resulting amount's own.
        let mut other = time("11:00:00");
        other.accuracy = Some(duration("PT20S"));
        let difference = precise.diff(&other).expect("valid");
        assert_eq!(difference.accuracy, Some(30.0));
        assert_eq!(difference.accuracy_is_percent, Some(false));
    }
}
