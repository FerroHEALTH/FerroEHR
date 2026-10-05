// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM spec functions for the `DV_ORDERED` / `DV_QUANTIFIED` family.
//!
//! The surface is `magnitude()`, `is_strictly_comparable_to()`,
//! `less_than()`, `is_simple()`, `is_normal()`, plus the [`OrderedLimit`]
//! comparison surface `DV_INTERVAL` / `REFERENCE_RANGE` need.
//!
//! Spec: RM 1.2.0 `docs/specs/openehr/RM/docs/UML/classes/`
//! `org.openehr.rm.data_types.{dv_ordered,dv_quantified,dv_amount,dv_quantity,`
//! `dv_count,dv_proportion,dv_ordinal,dv_scale,dv_date,dv_time,dv_date_time,`
//! `dv_duration}.adoc`. Magnitude semantics:
//!
//! - `DV_QUANTITY.magnitude` / `DV_COUNT.magnitude` — stored fields.
//! - `DV_PROPORTION.magnitude()` — "effective magnitude represented by ratio"
//!   = `numerator / denominator`.
//! - `DV_DATE.magnitude()` — days since the calendar origin `0001-01-01`.
//! - `DV_TIME.magnitude()` — seconds since the start of day `00:00:00`.
//! - `DV_DATE_TIME.magnitude()` — seconds since `0001-01-01T00:00:00Z`.
//! - `DV_DURATION.magnitude()` — seconds, computed via `Iso8601_duration.
//!   to_seconds()` (BASE): non-definite components use the *nominal* averages
//!   from BASE `Time_definitions` — `Average_days_in_year` = 365.24,
//!   `Average_days_in_month` = 30.42
//!   (`org.openehr.base.foundation_types.{iso8601_duration,time_definitions}.adoc`).
//!
//! `less_than` carries the spec precondition `is_strictly_comparable_to(other)`
//! and, for partial/malformed temporal values, an unavailable magnitude — both
//! are surfaced as `Option::None` rather than a panic (no-`unwrap` rule).
//!
//! NOTE: the *indexed* query-path realisation of these ordering
//! semantics is the AQL engine's `openehr_magnitude` SQL function; this module is the
//! in-process RM authority the interval/reference-range invariants use, and the
//! two must stay semantically aligned.

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

// ── ISO-8601 component extraction (partial precision permitted) ─────────────
//
// The inputs are first checked with the `crate::v1_2::validate` ISO-8601 subset
// validators (the same accept-set as the `Value_valid` invariants), then
// decomposed. Missing date parts default to month 1 / day 1; missing time
// parts default to 0 — the natural embedding of a partial value at the start
// of its interval (the spec leaves partial-value magnitude undefined; recorded
// here as the deterministic choice).

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

/// Seconds since `00:00:00` local for an openEHR ISO-8601 time (RM
/// `DV_TIME.magnitude` — the timezone, if any, does not shift the
/// start-of-day origin).
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

/// Total seconds of an openEHR ISO-8601 duration (BASE
/// `Iso8601_duration.to_seconds`): definite components are exact; `Y`/`M`
/// components use `Average_days_in_year` / `Average_days_in_month`. A leading
/// `-` negates the whole value (openEHR deviation).
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

/// The comparison surface `DV_INTERVAL<T>` / `REFERENCE_RANGE` need from a
/// limit.
///
/// A limit type provides openEHR strict comparability (RM
/// `DV_ORDERED.is_strictly_comparable_to`) and ordered-magnitude comparison
/// (RM `DV_ORDERED.less_than` and the derived `<=`).
///
/// `None` means "not decidable at this type / for this value" (e.g. a
/// `serde_json::Value` element, or a temporal value whose magnitude is
/// unavailable) — invariant checks treat undecidable as not-violated, leaving
/// structural errors to the codec/schema layer.
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
}

/// A `serde_json::Value` element carries no RM ordering — every comparison is
/// undecidable (the validator's structural fallback path).
impl OrderedLimit for serde_json::Value {}

/// Generate the [`OrderedLimit`] + inherent comparison functions for a
/// concrete `DV_ORDERED` subtype from its comparability rule and its ordering
/// key.
macro_rules! ordered_limit {
    // The ordering arm, for a type whose order is NOT a per-value key: the
    // expression yields `Option<Ordering>` for the pair directly. `DV_PROPORTION`
    // needs it — comparing two ratios exactly is a cross-multiplication, and a
    // key would have to divide, which is where an order and an equality drift
    // apart.
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
        }

        ordered_limit!(@surface $ty);
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

            /// RM `less_than` (`<`); `None` when the precondition
            /// `is_strictly_comparable_to(other)` fails or a magnitude is
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

// DV_COUNT: any two counts are comparable; ordered by `magnitude`, which is an
// `Integer64` and is compared as one — `less_than`'s post-condition is an exact
// integer comparison, and routing it through `f64` made two counts at 2^53 that
// differ by one compare as neither less nor greater.
ordered_limit!(
    DvCount,
    "DV_COUNT",
    comparable(_a, _b) = true,
    key(v) = Some(v.magnitude)
);

// DV_PROPORTION: comparable iff same `type` (PROPORTION_KIND); ordered by the
// exact ratio comparison the class's own `is_equal` uses, so `<`, `=` and `>`
// cannot disagree (they did when this divided in `f64`).
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
    /// RM `DV_PROPORTION.magnitude`: the effective magnitude represented by
    /// the ratio, `numerator / denominator`. `None` when `denominator` is 0
    /// (which violates the `Valid_denominator` invariant).
    #[must_use]
    pub fn magnitude(&self) -> Option<f64> {
        (self.denominator != 0.0).then(|| self.numerator / self.denominator)
    }

    /// RM `DV_PROPORTION.is_integral`: `true` if the `numerator` and
    /// `denominator` values are integers.
    ///
    /// Spec: `dv_proportion.adoc` §Functions — "True if the `numerator` and
    /// `denominator` values are integers, i.e. if `precision` is 0." That "i.e."
    /// equates two tests that differ whenever `precision` is absent, and the
    /// class's four invariants cannot both be non-vacuous under either reading.
    /// The VALUE test wins: under the precision reading `Fraction_validity`
    /// becomes "a fraction must declare `precision = 0`", which rejects a
    /// perfectly good `1/2` that states no precision, and `Precision_validity`
    /// collapses to `precision = 0 implies precision = 0`. Under this reading
    /// only `Is_integral_validity` goes vacuous, and nothing valid is refused.
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
    /// RM `DV_DURATION.magnitude`: the duration as a number of seconds,
    /// computed per BASE `Iso8601_duration.to_seconds()` (nominal-average `Y`
    /// and `M` components). `None` when `value` is not a valid ISO-8601
    /// duration.
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

    /// RM `DV_QUANTIFIED.magnitude` for the quantified subtypes: `DV_QUANTITY`
    /// / `DV_COUNT` / `DV_PROPORTION` / `DV_DATE` (days) / `DV_TIME` /
    /// `DV_DATE_TIME` / `DV_DURATION` (seconds). `None` for the non-quantified
    /// `DV_ORDINAL` / `DV_SCALE` (which order by `value`, not magnitude) and
    /// for unavailable temporal magnitudes.
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

    /// RM `DV_ORDERED.is_strictly_comparable_to`: `true` only when both values
    /// are the same concrete subtype *and* that subtype's own comparability
    /// rule holds (same units for `DV_QUANTITY`, same proportion kind for
    /// `DV_PROPORTION`, …).
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

    /// RM `DV_ORDERED.is_normal`: whether the value lies in its normal range
    /// (when `normal_range` is present) or is flagged normal by
    /// `normal_status` = `"N"`. `None` when neither is present (the spec
    /// precondition) or the range comparison is undecidable.
    #[must_use]
    pub fn is_normal(&self) -> Option<bool> {
        macro_rules! by_status {
            ($x:expr) => {
                $x.normal_status.as_ref().map(|s| s.code_string == "N")
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
}

/// DV_ORDERED `Normal_range_and_status_consistency`:
/// `(normal_range /= Void and normal_status /= Void) implies
/// (normal_status.code_string.is_equal("N") xor not normal_range.has(self))`.
/// Shared by every concrete `DV_ORDERED` subtype's `Validate` impl; an
/// undecidable `has` (unavailable magnitude / incomparable limits) runs no
/// check.
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
        // A fractional hour/minute is not a valid openEHR time — "only
        // fractional seconds are supported" (BASE
        // `foundation_types/master06-time_types.adoc` §"ISO 8601 semantics not
        // included in these types") — so it has no magnitude.
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
