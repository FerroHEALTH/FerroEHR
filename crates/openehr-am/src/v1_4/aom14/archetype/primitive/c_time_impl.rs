// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `C_TIME` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_time.adoc` §Functions +
//! §Invariants.

use crate::v1_4::aom14::archetype::primitive::c_time::CTime;

impl CTime {
    /// Returns true if the constraint is stated as a range rather than as
    /// per-field validity flags.
    ///
    /// `validity_is_range` (`org.openehr.am.aom14.c_time.adoc` §Functions),
    /// pinned by the same page's `Validity_is_range` invariant:
    /// `validity_is_range = (range /= Void)`.
    #[must_use]
    pub fn validity_is_range(&self) -> bool {
        self.range.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::base_types::definitions::validity_kind::ValidityKind;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::point_interval::PointInterval;
    use openehr_base::v1_3::foundation_types::time::iso8601_time::Iso8601Time;

    fn time(range: Option<Interval<Iso8601Time>>) -> CTime {
        CTime {
            assumed_value: None,
            minute_validity: Some(ValidityKind::Optional),
            second_validity: None,
            millisecond_validity: None,
            timezone_validity: None,
            range,
        }
    }

    #[test]
    fn a_field_validity_constraint_is_not_a_range() {
        assert!(!time(None).validity_is_range());
    }

    #[test]
    fn a_set_range_makes_the_constraint_a_range() {
        let point = Interval::PointInterval(PointInterval {
            lower: Some(Iso8601Time {
                value: "09:30:00".to_owned(),
            }),
            upper: Some(Iso8601Time {
                value: "09:30:00".to_owned(),
            }),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        });
        assert!(time(Some(point)).validity_is_range());
    }
}
