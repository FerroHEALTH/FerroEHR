// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM2 `C_INTEGER`/`C_REAL` interval readings the `v2_4` Web Template
//! front end ([`super::builder_v2_4`]) turns into input ranges and value lists.
//!
//! A primitive constraint is a list of intervals and admits a value inside any
//! one of them (AOM2 `master04.5-constraint_model-class_definitions.adoc`
//! §"Conformance semantics: C_ORDERED").

#![expect(
    clippy::disallowed_types,
    reason = "a Web Template range bound is a JSON number on the wire (ITS-REST Simplified \
              Formats), so serde_json::Value is the subject matter (#1694)"
)]

use openehr_base::prelude::{Interval, ProperInterval};

use super::model::WebTemplateRange;

fn json_f64(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

/// The one interval of a `C_INTEGER`/`C_REAL` constraint, which an input
/// range can carry; a list of several intervals admits a value in ANY of them
/// (AOM2 `master04.5-constraint_model-class_definitions.adoc` §"Conformance
/// semantics: C_ORDERED"), so it is never narrowed to its first.
pub(super) fn sole_interval<T>(constraint: Option<&Vec<Interval<T>>>) -> Option<&Interval<T>> {
    match constraint.map(Vec::as_slice) {
        Some([iv]) => Some(iv),
        _ => None,
    }
}

/// The values of a constraint made of two or more point intervals (the ADL2
/// list form `{10, 15, 20}`, ADL2 `master04.5-cadl_primitive_types.adoc`
/// §"List of Integers").
pub(super) fn point_list<T: Clone>(constraint: Option<&Vec<Interval<T>>>) -> Option<Vec<T>> {
    let ivs = constraint.filter(|ivs| ivs.len() > 1)?;
    ivs.iter()
        .map(|iv| match iv {
            Interval::PointInterval(p) => p.lower.clone(),
            Interval::ProperInterval(_) => None,
        })
        .collect()
}

/// A `WebTemplateRange` from a `C_REAL` interval (bounds + inclusivity ops).
pub(super) fn real_range(iv: &Interval<f64>) -> Option<WebTemplateRange> {
    let (lo, hi, lo_inc, hi_inc, lo_unb, hi_unb) = interval_bounds(iv)?;
    let min = if lo_unb { None } else { lo };
    let max = if hi_unb { None } else { hi };
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.map(|_| if lo_inc { ">=" } else { ">" }.to_owned()),
        min: min.map(json_f64),
        max_op: max.map(|_| if hi_inc { "<=" } else { "<" }.to_owned()),
        max: max.map(json_f64),
    })
}

/// A `WebTemplateRange` from a `C_INTEGER` interval (used for count/precision).
pub(super) fn int_range(iv: &Interval<i32>) -> Option<WebTemplateRange> {
    let (lo, hi, lo_inc, hi_inc, lo_unb, hi_unb) = interval_bounds(iv)?;
    let min = if lo_unb { None } else { lo }.map(|v| if lo_inc { v } else { v + 1 });
    let max = if hi_unb { None } else { hi }.map(|v| if hi_inc { v } else { v - 1 });
    if min.is_none() && max.is_none() {
        return None;
    }
    Some(WebTemplateRange {
        min_op: min.map(|_| ">=".to_owned()),
        min: min.map(serde_json::Value::from),
        max_op: max.map(|_| "<=".to_owned()),
        max: max.map(serde_json::Value::from),
    })
}

/// `(lower, upper, lower_included, upper_included, lower_unbounded,
/// upper_unbounded)` of an `Interval<T>` — both the point and proper forms.
#[expect(
    clippy::type_complexity,
    reason = "the six-element tuple IS the interval-bounds result named in the doc comment; a struct would only wrap it for one call site"
)]
fn interval_bounds<T: Clone>(
    iv: &Interval<T>,
) -> Option<(Option<T>, Option<T>, bool, bool, bool, bool)> {
    match iv {
        Interval::PointInterval(p) => Some((
            p.lower.clone(),
            p.lower.clone(),
            true,
            true,
            p.lower_unbounded,
            p.lower_unbounded,
        )),
        Interval::ProperInterval(ProperInterval::ProperInterval(p)) => Some((
            p.lower.clone(),
            p.upper.clone(),
            p.lower_included,
            p.upper_included,
            p.lower_unbounded,
            p.upper_unbounded,
        )),
        Interval::ProperInterval(ProperInterval::MultiplicityInterval(_)) => None,
    }
}

pub(super) fn point_i32(iv: &Interval<i32>) -> Option<i32> {
    match iv {
        Interval::PointInterval(p) => p.lower,
        Interval::ProperInterval(_) => None,
    }
}

pub(super) fn point_f64(iv: &Interval<f64>) -> Option<f64> {
    match iv {
        Interval::PointInterval(p) => p.lower,
        Interval::ProperInterval(_) => None,
    }
}
