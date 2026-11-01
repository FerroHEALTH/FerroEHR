// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM2 `C_INTEGER`/`C_REAL` interval readings the `v2_4` Web Template
//! front end ([`super::builder_v2_4`]) turns into input ranges and value lists,
//! and the `defaultValue` an assumed value gives when its intervals admit it,
//! and the occurrences, existence and cardinality intervals it turns into a
//! node's `min` and `max`.
//!
//! A primitive constraint is a list of intervals and admits a value inside any
//! one of them (AOM2 `master04.5-constraint_model-class_definitions.adoc`
//! §"Conformance semantics: C_ORDERED").

#![expect(
    clippy::disallowed_types,
    reason = "a Web Template range bound is a JSON number on the wire (ITS-REST Simplified \
              Formats), so serde_json::Value is the subject matter (#1694)"
)]

use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_base::prelude::{Interval, MultiplicityInterval, ProperInterval};

use super::inputs::default_string;
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

/// The ranges of a constraint made of two or more intervals that are not all
/// points, a union of ranges admitting a value inside any one of them (AOM2
/// `master04.5-constraint_model-class_definitions.adoc` §"Conformance
/// semantics: C_ORDERED"); `None` for any other constraint, and when one of
/// the intervals is unbounded at both ends, which admits every value.
pub(super) fn range_union<T>(
    constraint: Option<&Vec<Interval<T>>>,
    range: fn(&Interval<T>) -> Option<WebTemplateRange>,
) -> Option<Vec<WebTemplateRange>> {
    let ivs = constraint.filter(|ivs| ivs.len() > 1)?;
    if ivs
        .iter()
        .all(|iv| matches!(iv, Interval::PointInterval(_)))
    {
        return None;
    }
    ivs.iter().map(range).collect()
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

/// `(min, max)` from an object's occurrences; an unset occurrences defaults to
/// `0..1`, a permitted, single, optional node: the safe default that never
/// forces a spurious mandatory node into the example skeleton.
pub(super) fn occurrences(iv: Option<&MultiplicityInterval>) -> (Option<i32>, i32) {
    iv.map_or((Some(0), 1), occ)
}

/// `(min, max)` from a set occurrences, existence or cardinality interval;
/// an unbounded upper end is `-1`.
pub(super) fn occ(iv: &MultiplicityInterval) -> (Option<i32>, i32) {
    let min = if iv.lower_unbounded { None } else { iv.lower };
    let max = if iv.upper_unbounded {
        -1
    } else {
        iv.upper.unwrap_or_else(|| iv.lower.unwrap_or(0))
    };
    (min, max)
}

/// Whether an AOM2 interval-list constraint admits `value`: an absent or empty
/// list is no constraint, else any member interval must `has` it
/// (AM `UML/classes/org.openehr.am.aom2.c_integer.adoc` §`constraint`).
pub(super) fn admits<T: PartialOrd>(constraint: Option<&Vec<Interval<T>>>, value: &T) -> bool {
    constraint.is_none_or(|intervals| {
        intervals.is_empty() || intervals.iter().any(|interval| interval.has(value))
    })
}

/// The `defaultValue` of a `C_INTEGER`: its assumed value when integral and
/// admitted (the generated AOM2 model carries `assumed_value` as a Real).
pub(super) fn integer_default(ci: &CInteger) -> Option<serde_json::Value> {
    let rendered = ci.assumed_value?.to_string();
    let value = rendered.parse::<i32>().ok()?;
    admits(ci.constraint.as_ref(), &value).then(|| default_string(&value))
}

/// The `defaultValue` of a `C_REAL`: its assumed value when admitted.
pub(super) fn real_default(cr: &CReal) -> Option<serde_json::Value> {
    cr.assumed_value
        .filter(|v| admits(cr.constraint.as_ref(), v))
        .map(|v| default_string(&v))
}

#[cfg(test)]
mod default_value_tests {
    use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
    use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
    use openehr_base::prelude::{Interval, PointInterval, ProperInterval, ProperIntervalData};

    use super::{integer_default, real_default};

    fn up_to<T>(lower: T, upper: T) -> Interval<T> {
        Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }))
    }

    fn integer(assumed: Option<f64>, constraint: Vec<Interval<i32>>) -> CInteger {
        CInteger {
            parent: None,
            soc_parent: None,
            rm_type_name: "Integer".to_owned(),
            occurrences: None,
            node_id: "id9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: assumed,
            is_enumerated_type_constraint: None,
            constraint: Some(constraint),
        }
    }

    fn real(assumed: Option<f64>, constraint: Vec<Interval<f64>>) -> CReal {
        CReal {
            parent: None,
            soc_parent: None,
            rm_type_name: "Real".to_owned(),
            occurrences: None,
            node_id: "id9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: assumed,
            is_enumerated_type_constraint: None,
            constraint: Some(constraint),
        }
    }

    fn text(v: Option<serde_json::Value>) -> Option<String> {
        v.and_then(|v| v.as_str().map(str::to_owned))
    }

    /// AOM2 `C_PRIMITIVE_OBJECT.assumed_value` becomes `defaultValue` only when
    /// the interval constraint admits it (AM `org.openehr.am.aom2.c_integer.adoc`).
    #[test]
    fn numeric_assumed_values_are_offered_when_admitted() {
        assert_eq!(
            text(integer_default(&integer(Some(3.0), vec![up_to(0, 10)]))),
            Some("3".to_owned())
        );
        assert_eq!(
            text(integer_default(&integer(Some(30.0), vec![up_to(0, 10)]))),
            None
        );
        let points = vec![Interval::PointInterval(PointInterval {
            lower: Some(2),
            upper: Some(2),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        })];
        assert_eq!(
            text(integer_default(&integer(Some(2.0), points))),
            Some("2".to_owned())
        );
        assert_eq!(text(integer_default(&integer(Some(2.5), Vec::new()))), None);
        assert_eq!(
            text(real_default(&real(Some(0.21), vec![up_to(0.0, 1.0)]))),
            Some("0.21".to_owned())
        );
        assert_eq!(
            text(real_default(&real(Some(21.0), vec![up_to(0.0, 1.0)]))),
            None
        );
        assert_eq!(text(real_default(&real(None, Vec::new()))), None);
    }
}
