// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Integer intervals as the narrowing check reads them.

use crate::opt14::types::Intervalofinteger;

/// The inclusive integer bounds of an interval, `None` for an unbounded side.
///
/// An exclusive bound moves one step inward; an absent `*_included` reads as
/// included, the form `Template.xsd` writes for a closed bound.
fn inclusive_bounds(interval: &Intervalofinteger) -> (Option<i64>, Option<i64>) {
    let lower = if interval.lower_unbounded {
        None
    } else {
        interval.lower.map(|v| {
            let v = i64::from(v);
            if interval.lower_included.unwrap_or(true) {
                v
            } else {
                v.saturating_add(1)
            }
        })
    };
    let upper = if interval.upper_unbounded {
        None
    } else {
        interval.upper.map(|v| {
            let v = i64::from(v);
            if interval.upper_included.unwrap_or(true) {
                v
            } else {
                v.saturating_sub(1)
            }
        })
    };
    (lower, upper)
}

/// Returns whether every value of `inner` lies in `outer`.
pub(super) fn is_within(inner: &Intervalofinteger, outer: &Intervalofinteger) -> bool {
    let (inner_lower, inner_upper) = inclusive_bounds(inner);
    let (outer_lower, outer_upper) = inclusive_bounds(outer);
    let lower_ok = match (inner_lower, outer_lower) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(i), Some(o)) => i >= o,
    };
    let upper_ok = match (inner_upper, outer_upper) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(i), Some(o)) => i <= o,
    };
    lower_ok && upper_ok
}

/// Returns whether the interval admits no value above zero, the `{0}` that
/// marks a deleted node.
pub(super) fn is_zero(interval: &Intervalofinteger) -> bool {
    matches!(inclusive_bounds(interval), (_, Some(upper)) if upper <= 0)
}

/// Returns whether the interval requires at least one value.
pub(super) fn is_mandatory(interval: &Intervalofinteger) -> bool {
    matches!(inclusive_bounds(interval), (Some(lower), _) if lower >= 1)
}

/// Renders the interval in the cADL `lower..upper` form for a diagnostic.
pub(super) fn display(interval: &Intervalofinteger) -> String {
    let (lower, upper) = inclusive_bounds(interval);
    format!(
        "{{{}..{}}}",
        lower.map_or_else(|| "*".to_owned(), |v| v.to_string()),
        upper.map_or_else(|| "*".to_owned(), |v| v.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::{display, is_mandatory, is_within, is_zero};
    use crate::opt14::types::Intervalofinteger;

    fn closed(lower: i32, upper: i32) -> Intervalofinteger {
        Intervalofinteger {
            lower_included: Some(true),
            upper_included: Some(true),
            lower_unbounded: false,
            upper_unbounded: false,
            lower: Some(lower),
            upper: Some(upper),
        }
    }

    fn open_upper(lower: i32) -> Intervalofinteger {
        Intervalofinteger {
            lower_included: Some(true),
            upper_included: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower: Some(lower),
            upper: None,
        }
    }

    #[test]
    fn a_narrower_interval_is_within() {
        assert!(is_within(&closed(1, 1), &closed(0, 1)));
        assert!(is_within(&closed(0, 3), &open_upper(0)));
        assert!(is_within(&open_upper(2), &open_upper(0)));
    }

    #[test]
    fn a_wider_interval_is_not_within() {
        assert!(!is_within(&closed(0, 1), &closed(1, 1)));
        assert!(!is_within(&open_upper(0), &closed(0, 5)));
        assert!(!is_within(&closed(0, 2), &closed(0, 1)));
    }

    #[test]
    fn an_exclusive_bound_moves_inward() {
        let mut exclusive = closed(0, 2);
        exclusive.lower_included = Some(false);
        exclusive.upper_included = Some(false);
        assert!(is_within(&exclusive, &closed(1, 1)));
        assert_eq!(display(&exclusive), "{1..1}");
    }

    #[test]
    fn zero_and_mandatory_read_the_bounds() {
        assert!(is_zero(&closed(0, 0)));
        assert!(!is_zero(&closed(0, 1)));
        assert!(is_mandatory(&closed(1, 1)));
        assert!(!is_mandatory(&open_upper(0)));
        assert_eq!(display(&open_upper(0)), "{0..*}");
    }
}
