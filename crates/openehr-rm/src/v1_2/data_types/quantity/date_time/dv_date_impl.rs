// @generated-from-template templates/openehr-rm/data_types/quantity/date_time/dv_date_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `DV_DATE`.
//!
//! `Value_valid` (`valid_iso8601_date(value)`) —
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_date.adoc`
//! §Invariants — plus the inherited DV_QUANTIFIED `Magnitude_status_valid` and
//! the DV_ORDERED `Normal_range_and_status_consistency`.
//!
//! `value` is carried as a `String`, so `valid_iso8601_date` is a runtime check
//! here rather than a parse-time type guarantee (the ISO-8601 helpers live in
//! `crate::v1_2::validate`).

use crate::v1_2::data_types::quantity::date_time::dv_date::DvDate;
use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::dv_absolute_quantity_impl::{
    difference_accuracy, displaced_accuracy,
};
use crate::v1_2::data_types::quantity::dv_amount_impl::CombinedAccuracy;
use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_2::data_types::quantity::dv_ordered_impl::push_normal_range_consistency;
use crate::v1_2::validate::valid_iso8601_date;
use openehr_base::v1_3::foundation_types::time::iso8601_date::Iso8601Date;
use openehr_base::v1_3::foundation_types::time::iso8601_duration::Iso8601Duration;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvDate {
    /// Addition of a duration to this date, or `None` when either value is not
    /// a valid ISO-8601 string.
    ///
    /// Spec: `dv_date.adoc` §Functions `add` — "Addition of a Duration to this
    /// Date" — effecting `dv_absolute_quantity.adoc`'s abstract `add`, whose
    /// accuracy rule is realized by [`displaced_accuracy`].
    ///
    /// The calendar arithmetic is BASE's `Iso8601_date.add`, so leap years and
    /// month lengths are answered once, in the type that owns the calendar.
    #[must_use]
    pub fn add(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().add(&duration_of(a_diff))?, a_diff))
    }

    /// Subtraction of a duration from this date, under the same conditions as
    /// [`Self::add`].
    ///
    /// Spec: `dv_date.adoc` §Functions `subtract`.
    #[must_use]
    pub fn subtract(&self, a_diff: &DvDuration) -> Option<Self> {
        Some(self.displaced(self.iso().subtract(&duration_of(a_diff))?, a_diff))
    }

    /// Difference between this date and `other`, as a duration.
    ///
    /// Spec: `dv_date.adoc` §Functions `diff` — "Difference between this Date
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

    /// Returns `true` when this date is considered equal to `other`.
    ///
    /// Spec: `dv_date.adoc` §Functions `is_equal`, effecting
    /// `dv_quantified.adoc`'s abstract `is_equal`.
    ///
    /// Equality is of the DATE, not of how it was written: `less_than` on this
    /// class is defined over `magnitude()`, and `2024-01-01` and `20240101`
    /// name the same day, so comparing the strings would make a value neither
    /// less than, greater than, nor equal to its own extended form.
    #[must_use]
    pub fn is_equal(&self, other: &Self) -> bool {
        match (self.magnitude(), other.magnitude()) {
            (Some(left), Some(right)) => left == right,
            // A value with no magnitude is not a date this function can
            // compare; only an identical string can still be the same one.
            _ => self.value == other.value,
        }
    }

    /// This date's `value` as the BASE ISO-8601 type that owns the calendar.
    fn iso(&self) -> Iso8601Date {
        Iso8601Date {
            value: self.value.clone(),
        }
    }

    /// This date at a new value, carrying the displaced accuracy.
    fn displaced(&self, value: Iso8601Date, a_diff: &DvDuration) -> Self {
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

impl Validate for DvDate {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::temporal_value_core(
            "DV_DATE",
            valid_iso8601_date(&self.value),
            out,
        );
        crate::v1_2::validate::generated::magnitude_status_core(
            "DV_DATE",
            self.magnitude_status.as_deref(),
            out,
        );
        // Inherited DV_ORDERED Normal_range_and_status_consistency (the
        // normal_range element type is the DvOrdered enum here).
        push_normal_range_consistency(
            out,
            "DV_DATE",
            self.normal_status.as_ref(),
            self.normal_range.as_deref(),
            &DvOrdered::DvDate(self.clone()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> DvDate {
        DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            value: value.to_owned(),
        }
    }

    #[test]
    fn valid_date() {
        assert!(date("2021-05-17").invariants().is_empty());
        assert!(date("2021").invariants().is_empty());
    }

    #[test]
    fn invalid_date() {
        let v = date("2021-13-40").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Value_valid failed on type DV_DATE")
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

    /// `DV_DATE` declares `add`, and BASE defines that as "arithmetic addition
    /// of a duration to a date" — the NOMINAL calendar semantics belong to
    /// `add_nominal`, which this class does not declare. So `P1M` is the
    /// duration's own average month, not "the same day next month": adding it
    /// to 31 January lands on 1 March, not on the end of February.
    #[test]
    fn arithmetic_is_the_spec_s_arithmetic_addition_not_the_nominal_one() {
        assert_eq!(
            date("2024-01-31")
                .add(&duration("P1M"))
                .expect("valid")
                .value,
            "2024-03-01",
            "P1M is an average month of seconds, not a nominal calendar month"
        );

        // Exact durations still land exactly, leap day included.
        assert_eq!(
            date("2024-02-28")
                .add(&duration("P1D"))
                .expect("valid")
                .value,
            "2024-02-29"
        );
        assert_eq!(
            date("2024-03-01")
                .subtract(&duration("P1D"))
                .expect("valid")
                .value,
            "2024-02-29"
        );
    }

    /// `diff` returns the difference as a duration.
    #[test]
    fn diff_returns_a_duration() {
        let difference = date("2024-03-01").diff(&date("2024-02-01")).expect("valid");
        assert_eq!(difference.magnitude(), Some(29.0 * 86_400.0));
    }

    /// A value that is not a valid ISO-8601 date has no arithmetic.
    #[test]
    fn an_invalid_value_has_no_arithmetic() {
        let bad = date("2021-13-40");
        assert!(bad.add(&duration("P1D")).is_none());
        assert!(bad.diff(&date("2024-01-01")).is_none());
        assert!(date("2024-01-01").add(&duration("one day")).is_none());
    }

    /// Equality is of the DATE, not of the string: the extended and basic forms
    /// name the same day, and `less_than` already orders them by magnitude.
    #[test]
    fn is_equal_compares_the_date_not_the_string() {
        assert!(date("2024-01-01").is_equal(&date("20240101")));
        assert!(!date("2024-01-01").is_equal(&date("2024-01-02")));
    }

    /// "The sum of the accuracies of the operands, if both present, or unknown,
    /// if either or both operand accuracies are unknown."
    #[test]
    fn accuracy_follows_the_absolute_quantity_rule() {
        let mut precise = date("2024-01-01");
        precise.accuracy = Some(duration("PT30M"));
        let mut offset = duration("P1D");
        offset.accuracy = Some(60.0);
        offset.accuracy_is_percent = Some(false);

        let moved = precise.add(&offset).expect("valid");
        assert_eq!(
            moved.accuracy.expect("both present").magnitude(),
            Some(1860.0)
        );

        // An operand with no accuracy makes the result's unknown.
        assert!(
            precise
                .add(&duration("P1D"))
                .expect("valid")
                .accuracy
                .is_none()
        );
        assert!(
            date("2024-01-01")
                .add(&offset)
                .expect("valid")
                .accuracy
                .is_none()
        );
    }
}
