// @generated-from-template templates/openehr-rm/data_types/quantity/dv_ordered_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The `DV_ORDERED` and `DV_QUANTIFIED` functions and the [`OrderedLimit`]
//! comparison `DV_INTERVAL` and `REFERENCE_RANGE` use (RM 1.2.0
//! `org.openehr.rm.data_types.dv_*.adoc`).
//!
//! Magnitudes: a stored field for `DV_QUANTITY` and `DV_COUNT`, the ratio for
//! `DV_PROPORTION`, days since `0001-01-01` for `DV_DATE`, seconds for the
//! other temporal types, `Iso8601_duration.to_seconds` for `DV_DURATION`.
//!
//! NOTE: the AQL engine's `openehr_magnitude` SQL function realizes the same
//! ordering on the query path; the two must agree.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use crate::v1_2::data_types::quantity::date_time::dv_date::DvDate;
use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::date_time::dv_time::DvTime;
use crate::v1_2::data_types::quantity::dv_count::DvCount;
use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
use crate::v1_2::data_types::quantity::dv_ordinal::DvOrdinal;
use crate::v1_2::data_types::quantity::dv_proportion::DvProportion;
use crate::v1_2::data_types::quantity::dv_quantity::DvQuantity;
use crate::v1_2::data_types::quantity::dv_scale::DvScale;
use crate::v1_2::validate::{
    valid_iso8601_date, valid_iso8601_date_time, valid_iso8601_duration, valid_iso8601_time,
};

// ── BASE Time_definitions constants (nominal durations) ─────────────────────

/// BASE `Time_definitions.Average_days_in_year` = 365.24.
pub const AVERAGE_DAYS_IN_YEAR: f64 = 365.24;
/// BASE `Time_definitions.Average_days_in_month` = 30.42.
pub const AVERAGE_DAYS_IN_MONTH: f64 = 30.42;
/// BASE `Time_definitions.Days_in_week` = 7.
pub const DAYS_IN_WEEK: f64 = 7.0;
/// Seconds in a nominal day (24 × 60 × 60).
pub const SECONDS_IN_DAY: f64 = 86_400.0;

// ISO 8601 component extraction: a missing date part reads as 1 and a missing
// time part as 0, our own design (the spec leaves partial magnitude open).

fn parse_u32(s: &str) -> Option<u32> {
    s.parse::<u32>().ok()
}

/// Decompose an openEHR ISO-8601 date (extended or compact) into (y, m, d),
/// defaulting missing parts to 1.
fn date_parts(s: &str) -> Option<(i64, u32, u32)> {
    if !valid_iso8601_date(s) {
        return None;
    }
    let (y, m, d) = if s.contains('-') {
        let mut it = s.split('-');
        let y = it.next()?.parse::<i64>().ok()?;
        let m = it.next().map_or(Some(1), parse_u32)?;
        let d = it.next().map_or(Some(1), parse_u32)?;
        (y, m, d)
    } else {
        let y = s.get(0..4)?.parse::<i64>().ok()?;
        let m = s.get(4..6).map_or(Some(1), parse_u32)?;
        let d = s.get(6..8).map_or(Some(1), parse_u32)?;
        (y, m, d)
    };
    Some((y, m, d))
}

/// Days from civil date to the proleptic-Gregorian epoch 1970-01-01
/// (Howard Hinnant's `days_from_civil` algorithm).
#[expect(
    clippy::integer_division,
    reason = "days_from_civil is defined in terms of truncating integer division (era/leap-cycle counting); the discarded remainders are the algorithm"
)]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from((m + 9) % 12); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Days since the openEHR calendar origin `0001-01-01` (RM
/// `DV_DATE.magnitude`).
#[must_use]
pub(crate) fn iso_date_magnitude_days(s: &str) -> Option<i64> {
    let (y, m, d) = date_parts(s)?;
    Some(days_from_civil(y, m, d) - days_from_civil(1, 1, 1))
}

/// Split a trailing timezone (`Z` / `±HH[:MM]` / `±HHMM`) off a time string,
/// returning `(core, offset_seconds)`.
fn split_tz(s: &str) -> (&str, f64) {
    if let Some(core) = s.strip_suffix('Z') {
        return (core, 0.0);
    }
    if let Some((core, tz)) = s.rfind(['+', '-']).and_then(|pos| s.split_at_checked(pos)) {
        let sign = if tz.starts_with('-') { -1.0 } else { 1.0 };
        let rest = tz.get(1..).unwrap_or_default();
        let (h, m) = if let Some((h, m)) = rest.split_once(':') {
            (parse_u32(h), parse_u32(m))
        } else {
            match rest.len() {
                2 => (parse_u32(rest), Some(0)),
                4 => (
                    rest.get(0..2).and_then(parse_u32),
                    rest.get(2..4).and_then(parse_u32),
                ),
                _ => (None, None),
            }
        };
        if let (Some(h), Some(m)) = (h, m) {
            return (core, sign * (f64::from(h) * 3600.0 + f64::from(m) * 60.0));
        }
    }
    (s, 0.0)
}

/// Seconds since start of day for an ISO time *core* (no timezone), with the
/// fractional part applied to the last present component.
fn time_core_seconds(s: &str) -> Option<f64> {
    let (base, frac) = match s.split_once(['.', ',']) {
        Some((b, f)) => (b, Some(f)),
        None => (s, None),
    };
    let frac_val = match frac {
        Some(f) => format!("0.{f}").parse::<f64>().ok()?,
        None => 0.0,
    };
    let (h, m, sec, unit) = if base.contains(':') {
        let mut it = base.split(':');
        let h = parse_u32(it.next()?)?;
        let m = it.next().map(parse_u32);
        let s = it.next().map(parse_u32);
        match (m, s) {
            (None, _) => (h, 0, 0, 3600.0),
            (Some(m), None) => (h, m?, 0, 60.0),
            (Some(m), Some(s)) => (h, m?, s?, 1.0),
        }
    } else {
        match base.len() {
            2 => (parse_u32(base)?, 0, 0, 3600.0),
            4 => (
                base.get(0..2).and_then(parse_u32)?,
                base.get(2..4).and_then(parse_u32)?,
                0,
                60.0,
            ),
            6 => (
                base.get(0..2).and_then(parse_u32)?,
                base.get(2..4).and_then(parse_u32)?,
                base.get(4..6).and_then(parse_u32)?,
                1.0,
            ),
            _ => return None,
        }
    };
    Some(f64::from(h) * 3600.0 + f64::from(m) * 60.0 + f64::from(sec) + frac_val * unit)
}

/// Seconds since local `00:00:00` for a time (`DV_TIME.magnitude`); the zone
/// does not shift it.
#[must_use]
pub(crate) fn iso_time_magnitude_seconds(s: &str) -> Option<f64> {
    if !valid_iso8601_time(s) {
        return None;
    }
    let (core, _tz) = split_tz(s);
    time_core_seconds(core)
}

/// Seconds since `0001-01-01T00:00:00Z` for an openEHR ISO-8601 date-time (RM
/// `DV_DATE_TIME.magnitude`); a stated timezone offset is normalised to UTC.
#[must_use]
pub(crate) fn iso_date_time_magnitude_seconds(s: &str) -> Option<f64> {
    if !valid_iso8601_date_time(s) {
        return None;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "day counts over the representable calendar are far below 2^52, where f64 is exact on integers"
    )]
    if let Some((date, time)) = s.split_once('T') {
        let days = iso_date_magnitude_days(date)?;
        let (core, tz_offset) = split_tz(time);
        let secs = time_core_seconds(core)?;
        Some(days as f64 * SECONDS_IN_DAY + secs - tz_offset)
    } else {
        let days = iso_date_magnitude_days(s)?;
        Some(days as f64 * SECONDS_IN_DAY)
    }
}

/// Total seconds of a duration (`Iso8601_duration.to_seconds`), a leading `-`
/// negating it.
#[must_use]
pub(crate) fn iso_duration_to_seconds(s: &str) -> Option<f64> {
    if !valid_iso8601_duration(s) {
        return None;
    }
    let (sign, body) = match s.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, s.strip_prefix('+').unwrap_or(s)),
    };
    let body = body.strip_prefix('P')?;
    let (date_part, time_part) = match body.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (body, None),
    };
    let mut total = 0.0_f64;
    let mut acc = |part: &str, in_time: bool| -> Option<()> {
        let bytes = part.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let start = i;
            while bytes
                .get(i)
                .is_some_and(|b| b.is_ascii_digit() || *b == b'.' || *b == b',')
            {
                i += 1;
            }
            let num: f64 = part.get(start..i)?.replace(',', ".").parse().ok()?;
            let designator = *bytes.get(i)?;
            i += 1;
            let unit_seconds = match (designator, in_time) {
                (b'Y', false) => AVERAGE_DAYS_IN_YEAR * SECONDS_IN_DAY,
                (b'M', false) => AVERAGE_DAYS_IN_MONTH * SECONDS_IN_DAY,
                (b'W', false) => DAYS_IN_WEEK * SECONDS_IN_DAY,
                (b'D', false) => SECONDS_IN_DAY,
                (b'H', true) => 3600.0,
                (b'M', true) => 60.0,
                (b'S', true) => 1.0,
                _ => return None,
            };
            total += num * unit_seconds;
        }
        Some(())
    };
    acc(date_part, false)?;
    if let Some(t) = time_part {
        acc(t, true)?;
    }
    Some(sign * total)
}

// ── the OrderedLimit comparison surface ──────────────────────────────────────

/// The comparison a `DV_INTERVAL` or `REFERENCE_RANGE` limit provides:
/// `is_strictly_comparable_to` and `less_than`.
///
/// `None` is undecidable, which invariant checks treat as not violated.
pub trait OrderedLimit {
    /// RM `DV_ORDERED.is_strictly_comparable_to(other)`.
    fn strictly_comparable(&self, other: &Self) -> Option<bool> {
        let _ = other;
        None
    }
    /// RM `DV_ORDERED.less_than(other)` (`<`).
    fn less_than(&self, other: &Self) -> Option<bool> {
        let _ = other;
        None
    }
    /// Derived `<=` under the same comparability precondition.
    fn less_or_equal(&self, other: &Self) -> Option<bool> {
        let _ = other;
        None
    }
    /// RM `DV_ORDERED.is_simple`: whether this limit carries no reference
    /// ranges, as `REFERENCE_RANGE`'s `Range_is_simple` reads it.
    fn is_simple(&self) -> Option<bool> {
        None
    }
}

/// A `serde_json::Value` element carries no RM ordering — every comparison is
/// undecidable (the validator's structural fallback path).
impl OrderedLimit for serde_json::Value {}

/// A boxed limit orders as the value it holds: the RM layout boxes a large
/// `DV_ORDERED` variant, which changes no comparison.
impl<T: OrderedLimit> OrderedLimit for Box<T> {
    fn strictly_comparable(&self, other: &Self) -> Option<bool> {
        T::strictly_comparable(self, other)
    }
    fn less_than(&self, other: &Self) -> Option<bool> {
        T::less_than(self, other)
    }
    fn less_or_equal(&self, other: &Self) -> Option<bool> {
        T::less_or_equal(self, other)
    }
    fn is_simple(&self) -> Option<bool> {
        T::is_simple(self)
    }
}

/// Generates the [`OrderedLimit`] and comparison functions of a concrete
/// `DV_ORDERED` subtype.
macro_rules! ordered_limit {
    // The pairwise ordering arm, for `DV_PROPORTION`, which compares ratios by
    // cross-multiplication.
    ($ty:ty, $rm:literal, comparable($a:ident, $b:ident) = $cmp:expr, order($x:ident, $y:ident) = $ord:expr) => {
        impl OrderedLimit for $ty {
            fn strictly_comparable(&self, other: &Self) -> Option<bool> {
                let ($a, $b) = (self, other);
                Some($cmp)
            }
            fn less_than(&self, other: &Self) -> Option<bool> {
                if self.strictly_comparable(other) != Some(true) {
                    return None;
                }
                let ($x, $y) = (self, other);
                Some($ord? == core::cmp::Ordering::Less)
            }
            fn less_or_equal(&self, other: &Self) -> Option<bool> {
                if self.strictly_comparable(other) != Some(true) {
                    return None;
                }
                let ($x, $y) = (self, other);
                Some($ord? != core::cmp::Ordering::Greater)
            }
            ordered_limit!(@simple);
        }

        ordered_limit!(@surface $ty);
    };

    ($ty:ty, $rm:literal, comparable($a:ident, $b:ident) = $cmp:expr, key($v:ident) = $key:expr) => {
        impl OrderedLimit for $ty {
            fn strictly_comparable(&self, other: &Self) -> Option<bool> {
                let ($a, $b) = (self, other);
                Some($cmp)
            }
            fn less_than(&self, other: &Self) -> Option<bool> {
                if self.strictly_comparable(other) != Some(true) {
                    return None;
                }
                let a = {
                    let $v = self;
                    $key
                }?;
                let b = {
                    let $v = other;
                    $key
                }?;
                Some(a < b)
            }
            fn less_or_equal(&self, other: &Self) -> Option<bool> {
                if self.strictly_comparable(other) != Some(true) {
                    return None;
                }
                let a = {
                    let $v = self;
                    $key
                }?;
                let b = {
                    let $v = other;
                    $key
                }?;
                Some(a <= b)
            }
            ordered_limit!(@simple);
        }

        ordered_limit!(@surface $ty);
    };

    // `DV_ORDERED.is_simple`, identical for every concrete subtype.
    (@simple) => {
        fn is_simple(&self) -> Option<bool> {
            Some(self.normal_range.is_none() && self.other_reference_ranges.is_none())
        }
    };

    // The RM-facing inherent methods, identical for every arm.
    (@surface $ty:ty) => {
        impl $ty {
            /// RM `is_strictly_comparable_to` for two values of this concrete
            /// type (see the module doc for the per-type rule).
            #[must_use]
            pub fn is_strictly_comparable_to(&self, other: &Self) -> bool {
                OrderedLimit::strictly_comparable(self, other) == Some(true)
            }

            /// RM `less_than`, or `None` when not strictly comparable or a magnitude is
            /// unavailable.
            #[must_use]
            #[expect(
                clippy::same_name_method,
                reason = "the inherent method IS the RM `DV_ORDERED.less_than` surface; it deliberately shares the name of the internal `OrderedLimit` trait method it forwards to"
            )]
            pub fn less_than(&self, other: &Self) -> Option<bool> {
                OrderedLimit::less_than(self, other)
            }
        }
    };
}

// DV_QUANTITY: comparable iff same `units` (and same `units_system` if set);
// ordered by `magnitude`.
ordered_limit!(
    DvQuantity,
    "DV_QUANTITY",
    comparable(a, b) = a.units == b.units && a.units_system == b.units_system,
    key(v) = Some(v.magnitude)
);

// DV_COUNT: always comparable, ordered by its exact `Integer64`.
ordered_limit!(
    DvCount,
    "DV_COUNT",
    comparable(_a, _b) = true,
    key(v) = Some(v.magnitude)
);

// DV_PROPORTION: comparable on the same `type`, ordered as `is_equal` compares.
ordered_limit!(
    DvProportion,
    "DV_PROPORTION",
    comparable(a, b) = a.r#type == b.r#type,
    order(x, y) = x.compare_to(y)
);

// DV_ORDINAL: two ordinals are comparable (finer symbol-set compatibility is
// an archetype-level concern); ordered by `value`.
ordered_limit!(
    DvOrdinal,
    "DV_ORDINAL",
    comparable(_a, _b) = true,
    key(v) = Some(f64::from(v.value))
);

// DV_SCALE: two scale values are comparable; ordered by `value`.
ordered_limit!(
    DvScale,
    "DV_SCALE",
    comparable(_a, _b) = true,
    key(v) = Some(v.value)
);

// DV_DATE: any two dates are comparable; ordered by day-magnitude.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "the day count over the representable calendar is far below 2^52, where f64 is exact on integers"
)]
mod dv_date_limit {
    use super::{DvDate, OrderedLimit, iso_date_magnitude_days};
    ordered_limit!(
        DvDate,
        "DV_DATE",
        comparable(_a, _b) = true,
        key(v) = iso_date_magnitude_days(&v.value).map(|d| d as f64)
    );
}

// DV_TIME: any two times are comparable; ordered by seconds-since-midnight.
ordered_limit!(
    DvTime,
    "DV_TIME",
    comparable(_a, _b) = true,
    key(v) = iso_time_magnitude_seconds(&v.value)
);

// DV_DATE_TIME: any two date-times are comparable; ordered by seconds since
// the calendar origin.
ordered_limit!(
    DvDateTime,
    "DV_DATE_TIME",
    comparable(_a, _b) = true,
    key(v) = iso_date_time_magnitude_seconds(&v.value)
);

// DV_DURATION: any two durations are comparable; ordered by nominal seconds.
ordered_limit!(
    DvDuration,
    "DV_DURATION",
    comparable(_a, _b) = true,
    key(v) = iso_duration_to_seconds(&v.value)
);

// ── per-type magnitude / auxiliary spec functions ────────────────────────────

impl DvQuantity {
    /// RM `DV_QUANTITY.is_integral`: `true` if `precision` = 0, meaning the
    /// magnitude is a whole number.
    #[must_use]
    pub fn is_integral(&self) -> bool {
        self.precision == Some(0)
    }
}

impl DvProportion {
    /// RM `DV_PROPORTION.magnitude`: `numerator / denominator`, or `None` for a
    /// zero denominator.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        (self.denominator != 0.0).then(|| self.numerator / self.denominator)
    }

    /// RM `DV_PROPORTION.is_integral`: the numerator and denominator are integers.
    ///
    /// `dv_proportion.adoc` adds "i.e. if `precision` is 0"; the value reading is
    /// taken, since the precision reading refuses a `1/2` with no precision.
    #[must_use]
    #[expect(
        clippy::float_cmp,
        reason = "an exact-integrality test is precisely a bit-equality question (`x.floor() == x`), not a tolerance comparison"
    )]
    pub fn is_integral(&self) -> bool {
        self.numerator.floor() == self.numerator && self.denominator.floor() == self.denominator
    }
}

impl DvDate {
    /// RM `DV_DATE.magnitude`: days since the calendar origin `0001-01-01`.
    /// `None` when `value` is not a valid ISO-8601 date.
    #[must_use]
    pub fn magnitude(&self) -> Option<i64> {
        iso_date_magnitude_days(&self.value)
    }
}

impl DvTime {
    /// RM `DV_TIME.magnitude`: seconds since the start of day (`00:00:00`).
    /// `None` when `value` is not a valid ISO-8601 time.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        iso_time_magnitude_seconds(&self.value)
    }
}

impl DvDateTime {
    /// RM `DV_DATE_TIME.magnitude`: seconds since `0001-01-01T00:00:00Z`.
    /// `None` when `value` is not a valid ISO-8601 date-time.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        iso_date_time_magnitude_seconds(&self.value)
    }
}

impl DvDuration {
    /// RM `DV_DURATION.magnitude`: seconds by `Iso8601_duration.to_seconds()`, or
    /// `None` when invalid.
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        iso_duration_to_seconds(&self.value)
    }
}

// ── DV_ORDERED (the abstract enum) ───────────────────────────────────────────

impl DvOrdered {
    /// RM `DV_ORDERED.is_simple`: `true` if this value carries no reference
    /// ranges (no `normal_range` and no `other_reference_ranges`).
    #[must_use]
    #[expect(
        clippy::same_name_method,
        reason = "the inherent method IS the RM `DV_ORDERED.is_simple` surface; the `OrderedLimit` method of the same name forwards to it"
    )]
    pub fn is_simple(&self) -> bool {
        macro_rules! simple {
            ($x:expr) => {
                // `other_reference_ranges` is `Option<NonEmptyVec<..>>`:
                // present means non-empty by construction.
                $x.normal_range.is_none() && $x.other_reference_ranges.is_none()
            };
        }
        match self {
            Self::DvCount(x) => simple!(x),
            Self::DvQuantity(x) => simple!(x),
            Self::DvOrdinal(x) => simple!(x),
            Self::DvScale(x) => simple!(x),
            Self::DvProportion(x) => simple!(x),
            Self::DvDate(x) => simple!(x),
            Self::DvDateTime(x) => simple!(x),
            Self::DvDuration(x) => simple!(x),
            Self::DvTime(x) => simple!(x),
        }
    }

    /// RM `DV_QUANTIFIED.magnitude`; `None` for `DV_ORDINAL` and `DV_SCALE` and
    /// for an unavailable temporal magnitude.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "DV_COUNT magnitudes and day counts are far below 2^52, where f64 is exact on integers"
    )]
    pub fn magnitude(&self) -> Option<f64> {
        match self {
            Self::DvQuantity(x) => Some(x.magnitude),
            Self::DvCount(x) => Some(x.magnitude as f64),
            Self::DvProportion(x) => x.magnitude(),
            Self::DvDate(x) => x.magnitude().map(|d| d as f64),
            Self::DvTime(x) => x.magnitude(),
            Self::DvDateTime(x) => x.magnitude(),
            Self::DvDuration(x) => x.magnitude(),
            Self::DvOrdinal(_) | Self::DvScale(_) => None,
        }
    }

    /// RM `DV_ORDERED.is_strictly_comparable_to`: the same concrete subtype and
    /// that subtype's own rule.
    #[must_use]
    pub fn is_strictly_comparable_to(&self, other: &Self) -> bool {
        OrderedLimit::strictly_comparable(self, other) == Some(true)
    }

    /// RM `DV_ORDERED.less_than` (`<`). `None` when the values are not
    /// strictly comparable or a magnitude is unavailable.
    #[must_use]
    #[expect(
        clippy::same_name_method,
        reason = "the inherent method IS the RM `DV_ORDERED.less_than` surface; it deliberately shares the name of the internal `OrderedLimit` trait method it forwards to"
    )]
    pub fn less_than(&self, other: &Self) -> Option<bool> {
        OrderedLimit::less_than(self, other)
    }

    /// RM `DV_ORDERED.is_normal`: inside `normal_range`, or `normal_status` is
    /// `"N"`; `None` when neither is present or undecidable.
    #[must_use]
    pub fn is_normal(&self) -> Option<bool> {
        macro_rules! by_status {
            ($x:expr) => {
                $x.normal_status.as_deref().map(|s| s.code_string == "N")
            };
        }
        // normal_range takes priority (spec Post_range), falling back to the
        // normal_status marker (Post_status).
        match self {
            Self::DvQuantity(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(x))
                .or_else(|| by_status!(x)),
            Self::DvCount(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(x))
                .or_else(|| by_status!(x)),
            Self::DvProportion(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(x))
                .or_else(|| by_status!(x)),
            Self::DvOrdinal(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
            Self::DvScale(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
            Self::DvDate(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
            Self::DvTime(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
            Self::DvDateTime(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
            Self::DvDuration(x) => x
                .normal_range
                .as_ref()
                .and_then(|r| r.has(self))
                .or_else(|| by_status!(x)),
        }
    }
}

impl OrderedLimit for DvOrdered {
    fn strictly_comparable(&self, other: &Self) -> Option<bool> {
        macro_rules! per {
            ($($variant:ident),+) => {
                match (self, other) {
                    $((Self::$variant(a), Self::$variant(b)) =>
                        OrderedLimit::strictly_comparable(a, b),)+
                    _ => Some(false),
                }
            };
        }
        per!(
            DvQuantity,
            DvCount,
            DvProportion,
            DvOrdinal,
            DvScale,
            DvDate,
            DvTime,
            DvDateTime,
            DvDuration
        )
    }

    fn less_than(&self, other: &Self) -> Option<bool> {
        macro_rules! per {
            ($($variant:ident),+) => {
                match (self, other) {
                    $((Self::$variant(a), Self::$variant(b)) =>
                        OrderedLimit::less_than(a, b),)+
                    _ => None,
                }
            };
        }
        per!(
            DvQuantity,
            DvCount,
            DvProportion,
            DvOrdinal,
            DvScale,
            DvDate,
            DvTime,
            DvDateTime,
            DvDuration
        )
    }

    fn less_or_equal(&self, other: &Self) -> Option<bool> {
        macro_rules! per {
            ($($variant:ident),+) => {
                match (self, other) {
                    $((Self::$variant(a), Self::$variant(b)) =>
                        OrderedLimit::less_or_equal(a, b),)+
                    _ => None,
                }
            };
        }
        per!(
            DvQuantity,
            DvCount,
            DvProportion,
            DvOrdinal,
            DvScale,
            DvDate,
            DvTime,
            DvDateTime,
            DvDuration
        )
    }

    fn is_simple(&self) -> Option<bool> {
        Some(Self::is_simple(self))
    }
}

/// DV_ORDERED `Normal_range_and_status_consistency`; an undecidable `has`
/// runs no check.
pub(crate) fn push_normal_range_consistency<T: OrderedLimit>(
    out: &mut Vec<openehr_base::validate::InvariantViolation>,
    rm_type: &str,
    normal_status: Option<&crate::v1_2::data_types::text::code_phrase::CodePhrase>,
    normal_range: Option<&crate::v1_2::data_types::quantity::dv_interval::DvInterval<T>>,
    value: &T,
) {
    if let (Some(status), Some(range)) = (normal_status, normal_range)
        && let Some(in_range) = range.has(value)
    {
        let is_n = status.code_string == "N";
        // The invariant holds iff `is_n xor not in_range`.
        if !(is_n ^ !in_range) {
            out.push(crate::v1_2::validate::invariant_failed(
                "Normal_range_and_status_consistency",
                rm_type,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quantity(magnitude: f64, units: &str) -> DvQuantity {
        DvQuantity {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            magnitude,
            precision: None,
            units: units.to_owned(),
            units_system: None,
            units_display_name: None,
        }
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
    fn date_magnitude_origin_is_zero() {
        assert_eq!(iso_date_magnitude_days("0001-01-01"), Some(0));
        assert_eq!(iso_date_magnitude_days("0001-01-02"), Some(1));
        assert_eq!(iso_date_magnitude_days("0002-01-01"), Some(365));
    }

    #[test]
    fn date_magnitude_partial_defaults() {
        // Partial dates embed at the start of their interval.
        assert_eq!(
            iso_date_magnitude_days("2021"),
            iso_date_magnitude_days("2021-01-01")
        );
        assert_eq!(
            iso_date_magnitude_days("2021-05"),
            iso_date_magnitude_days("2021-05-01")
        );
        // Compact form equals extended form.
        assert_eq!(
            iso_date_magnitude_days("20210517"),
            iso_date_magnitude_days("2021-05-17")
        );
        assert_eq!(iso_date_magnitude_days("garbage"), None);
    }

    #[test]
    fn time_magnitude_forms() {
        assert_eq!(iso_time_magnitude_seconds("00:00:00"), Some(0.0));
        assert_eq!(
            iso_time_magnitude_seconds("10:30:15"),
            Some(10.0 * 3600.0 + 30.0 * 60.0 + 15.0)
        );
        assert_eq!(iso_time_magnitude_seconds("10:30:15.5"), Some(37815.5));
        // "only fractional seconds are supported" (BASE `master06-time_types.adoc`).
        assert_eq!(iso_time_magnitude_seconds("10.5"), None);
        assert_eq!(iso_time_magnitude_seconds("10:05.5"), None);
        // Timezone does not shift the start-of-day origin.
        assert_eq!(iso_time_magnitude_seconds("10:00:00+02:00"), Some(36000.0));
        assert_eq!(iso_time_magnitude_seconds("bad"), None);
    }

    #[test]
    fn date_time_magnitude_utc_normalised() {
        let base = iso_date_time_magnitude_seconds("2021-05-17T10:00:00Z").unwrap();
        // +02:00 local == two hours earlier in UTC.
        let offset = iso_date_time_magnitude_seconds("2021-05-17T10:00:00+02:00").unwrap();
        assert!((base - offset - 7200.0).abs() < 1e-9);
        // Date-only value = midnight.
        let midnight = iso_date_time_magnitude_seconds("2021-05-17").unwrap();
        assert!((base - midnight - 36000.0).abs() < 1e-9);
    }

    #[test]
    fn duration_nominal_seconds() {
        assert_eq!(iso_duration_to_seconds("PT1S"), Some(1.0));
        assert_eq!(iso_duration_to_seconds("PT2H30M"), Some(9000.0));
        assert_eq!(iso_duration_to_seconds("P1D"), Some(86_400.0));
        assert_eq!(iso_duration_to_seconds("P2W"), Some(14.0 * 86_400.0));
        // Nominal month/year lengths (BASE Time_definitions).
        assert_eq!(iso_duration_to_seconds("P1M"), Some(30.42 * 86_400.0));
        assert_eq!(iso_duration_to_seconds("P1Y"), Some(365.24 * 86_400.0));
        assert_eq!(iso_duration_to_seconds("-P1D"), Some(-86_400.0));
        assert_eq!(iso_duration_to_seconds("PT0.5S"), Some(0.5));
        assert_eq!(iso_duration_to_seconds("P"), None);
    }

    #[test]
    fn quantity_comparability_requires_same_units() {
        let kg1 = quantity(50.0, "kg");
        let kg2 = quantity(70.0, "kg");
        let mm = quantity(60.0, "mm[Hg]");
        assert!(kg1.is_strictly_comparable_to(&kg2));
        assert!(!kg1.is_strictly_comparable_to(&mm));
        assert_eq!(kg1.less_than(&kg2), Some(true));
        assert_eq!(kg2.less_than(&kg1), Some(false));
        assert_eq!(kg1.less_than(&mm), None);
    }

    #[test]
    fn proportion_kind_gates_comparison() {
        let mk = |n: f64, d: f64, ty: i32| DvProportion {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            numerator: n,
            denominator: d,
            r#type: ty,
            precision: None,
        };
        let percent = mk(10.0, 100.0, 2);
        let percent2 = mk(20.0, 100.0, 2);
        let ratio = mk(1.0, 2.0, 0);
        assert_eq!(percent.less_than(&percent2), Some(true));
        assert_eq!(percent.less_than(&ratio), None);
        assert_eq!(percent.magnitude(), Some(0.1));
        assert!(mk(1.0, 2.0, 0).magnitude().is_some());
        assert_eq!(mk(1.0, 0.0, 0).magnitude(), None);
        assert!(mk(1.0, 2.0, 0).is_integral());
        assert!(!mk(1.5, 2.0, 0).is_integral());
    }

    #[test]
    fn temporal_ordering() {
        assert_eq!(
            date("2021-05-17").less_than(&date("2021-05-18")),
            Some(true)
        );
        assert_eq!(
            date_time("2021-05-17T10:00:00").less_than(&date_time("2021-05-17T09:00:00")),
            Some(false)
        );
        assert_eq!(time("09:00").less_than(&time("10:00")), Some(true));
        assert_eq!(duration("PT1H").less_than(&duration("P1D")), Some(true));
        // Malformed value → magnitude unavailable → None.
        assert_eq!(date("bad").less_than(&date("2021-01-01")), None);
    }

    #[test]
    fn enum_cross_type_never_comparable() {
        let a = DvOrdered::DvQuantity(quantity(1.0, "kg"));
        let b = DvOrdered::DvDuration(duration("PT1S"));
        assert!(!a.is_strictly_comparable_to(&b));
        assert_eq!(a.less_than(&b), None);
        assert!(a.is_strictly_comparable_to(&DvOrdered::DvQuantity(quantity(2.0, "kg"))));
    }

    #[test]
    fn enum_magnitude() {
        assert_eq!(
            DvOrdered::DvQuantity(quantity(42.5, "kg")).magnitude(),
            Some(42.5)
        );
        assert_eq!(
            DvOrdered::DvDuration(duration("PT1M")).magnitude(),
            Some(60.0)
        );
    }
}
