// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The 1.4-shaped `v2_4` object constructors and the primitive, code-phrase and
//! code-reference constraint mappings.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_base::prelude::{
    Interval, Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time, PointInterval,
    ProperInterval, ProperIntervalData,
};
use openehr_its::opt14;

use super::RootCx;
use super::value::{enumerated_and_range, int_interval, map_mult, real_interval};
use crate::aom::build::PrimitiveNode;

pub(super) fn complex(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    attributes: Vec<CAttribute>,
) -> CObject {
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(attributes),
        attribute_tuples: openehr_base::containers::present(Vec::new()),
    }))
}

/// The `v2_4` primitive node of a 1.4 object: its RM type, its node id and
/// its occurrences, which every 1.4 `C_OBJECT` carries.
fn primitive_node(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
) -> PrimitiveNode {
    PrimitiveNode {
        rm_type_name: rm_type_name.to_owned(),
        node_id: node_id.to_owned(),
        occurrences: Some(map_mult(occurrences)),
    }
}

pub(super) fn terminology_code(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    constraint: String,
) -> CObject {
    CObject::CTerminologyCode(
        primitive_node(rm_type_name, node_id, occurrences)
            .c_terminology_code(constraint, None, None),
    )
}

/// The six facets of a 1.4 `IntervalOf*` temporal range, for
/// [`temporal_interval`]: the XSD types share these fields and no Rust type.
macro_rules! range_facets {
    ($range:expr) => {
        $range.as_ref().map(|r| {
            (
                r.lower.clone(),
                r.upper.clone(),
                r.lower_unbounded,
                r.upper_unbounded,
                r.lower_included,
                r.upper_included,
            )
        })
    };
}

/// Maps a `C_PRIMITIVE_OBJECT` (its wrapped `C_PRIMITIVE`) to the matching
/// `v2_4` primitive constraint node.
///
/// The converter passes primitive nodes through untouched. A node without
/// `item` maps as the unconstrained constraint of the primitive type its
/// `rm_type_name` names (`unconstrained_primitive`); a name that is no AOM 1.4
/// primitive type maps to an unconstrained object of that type, reported in
/// `conversion_details`.
pub(super) fn map_primitive_object(c: &opt14::types::CPrimitiveObject, cx: &mut RootCx) -> CObject {
    let rm = c.rm_type_name.as_str();
    let node_id = c.node_id.as_str();
    let occ = &c.occurrences;
    let Some(item) = c.item.as_deref() else {
        if let Some(item) = unconstrained_primitive(rm) {
            let with_item = opt14::types::CPrimitiveObject {
                item: Some(Box::new(item)),
                ..c.clone()
            };
            return map_primitive_object(&with_item, cx);
        }
        cx.note(
            format!("primitive_without_item.{rm}.{node_id}"),
            format!(
                "C_PRIMITIVE_OBJECT node {node_id:?} carries no item and its rm_type_name \
                 {rm:?} names no AOM 1.4 primitive type; it converts to an unconstrained \
                 {rm} object"
            ),
        );
        return complex(rm, node_id, occ, Vec::new());
    };
    let node = primitive_node(rm, node_id, occ);
    match item {
        opt14::types::CPrimitive::CBoolean(p) => {
            let mut constraint = Vec::new();
            if p.true_valid {
                constraint.push(true);
            }
            if p.false_valid {
                constraint.push(false);
            }
            CObject::CBoolean(node.c_boolean(constraint, p.assumed_value))
        }
        opt14::types::CPrimitive::CString(p) => {
            let constraint = if !p.list.is_empty() {
                p.list.clone()
            } else if let Some(pat) = &p.pattern {
                vec![pat.clone()]
            } else {
                Vec::new()
            };
            CObject::CString(node.c_string(constraint, p.assumed_value.clone()))
        }
        opt14::types::CPrimitive::CInteger(p) => CObject::CInteger(node.c_integer(
            enumerated_and_range(&p.list, p.range.as_ref().map(int_interval)),
            p.assumed_value.map(f64::from),
        )),
        opt14::types::CPrimitive::CReal(p) => CObject::CReal(node.c_real(
            enumerated_and_range(&p.list, p.range.as_ref().map(real_interval)),
            p.assumed_value,
        )),
        opt14::types::CPrimitive::CDate(p) => CObject::CDate(node.c_date(
            temporal_interval(range_facets!(p.range), |value| Iso8601Date { value }),
            date_time_pattern(p.pattern.clone(), p.range.is_some(), rm, node_id, cx),
            p.assumed_value.clone().map(|value| Iso8601Date { value }),
        )),
        opt14::types::CPrimitive::CDateTime(p) => CObject::CDateTime(
            node.c_date_time(
                temporal_interval(range_facets!(p.range), |value| Iso8601DateTime { value }),
                date_time_pattern(p.pattern.clone(), p.range.is_some(), rm, node_id, cx),
                p.assumed_value
                    .clone()
                    .map(|value| Iso8601DateTime { value }),
            ),
        ),
        opt14::types::CPrimitive::CDuration(p) => CObject::CDuration(
            node.c_duration(
                temporal_interval(range_facets!(p.range), |value| Iso8601Duration { value }),
                p.pattern.clone(),
                p.assumed_value
                    .clone()
                    .map(|value| Iso8601Duration { value }),
            ),
        ),
        opt14::types::CPrimitive::CTime(p) => CObject::CTime(node.c_time(
            temporal_interval(range_facets!(p.range), |value| Iso8601Time { value }),
            date_time_pattern(p.pattern.clone(), p.range.is_some(), rm, node_id, cx),
            p.assumed_value.clone().map(|value| Iso8601Time { value }),
        )),
    }
}

/// Returns the constraint that admits every value of the AOM 1.4 primitive
/// type `rm_type_name` names, or `None` when it names none.
///
/// A `C_PRIMITIVE_OBJECT` without `item` constrains nothing beyond its type
/// (`item` is `minOccurs="0"` in `its-xml-1.0.2-nsv1/ALL/Archetype.xsd`
/// `C_PRIMITIVE_OBJECT`), so it converts to the
/// unconstrained form of the `C_PRIMITIVE` subtype for that type
/// (AM `AOM1.4/master06-primitive_package.adoc` §Class Descriptions). The
/// type names are matched case-insensitively.
pub(super) fn unconstrained_primitive(rm_type_name: &str) -> Option<opt14::types::CPrimitive> {
    use opt14::types::CPrimitive;
    let name = rm_type_name.to_ascii_uppercase();
    Some(match name.as_str() {
        "BOOLEAN" => CPrimitive::CBoolean(opt14::types::CBoolean {
            true_valid: true,
            false_valid: true,
            assumed_value: None,
        }),
        "STRING" => CPrimitive::CString(opt14::types::CString {
            pattern: None,
            list: Vec::new(),
            list_open: None,
            assumed_value: None,
        }),
        "INTEGER" => CPrimitive::CInteger(opt14::types::CInteger {
            list: Vec::new(),
            range: None,
            assumed_value: None,
        }),
        "REAL" => CPrimitive::CReal(opt14::types::CReal {
            list: Vec::new(),
            range: None,
            assumed_value: None,
        }),
        "DATE" => CPrimitive::CDate(opt14::types::CDate {
            pattern: None,
            timezone_validity: None,
            range: None,
            assumed_value: None,
        }),
        "DATE_TIME" => CPrimitive::CDateTime(opt14::types::CDateTime {
            pattern: None,
            timezone_validity: None,
            range: None,
            assumed_value: None,
        }),
        "TIME" => CPrimitive::CTime(opt14::types::CTime {
            pattern: None,
            timezone_validity: None,
            range: None,
            assumed_value: None,
        }),
        "DURATION" => CPrimitive::CDuration(opt14::types::CDuration {
            pattern: None,
            range: None,
            assumed_value: None,
        }),
        _ => return None,
    })
}

/// Date/time/date-time constraints carry a pattern XOR a range on the ADL2
/// surface — the mixed `pattern/interval` form is defined for durations only
/// (`ADL2/master04.5-cadl_primitive_types.adoc` §Mixed Pattern and Interval
/// sits under Duration Constraints). When a 1.4 node carries both, the range
/// (the value constraint) is kept — the safe, still-narrowing half — and the
/// dropped format pattern reported in `conversion_details`.
pub(super) fn date_time_pattern(
    pattern: Option<String>,
    has_range: bool,
    rm: &str,
    node_id: &str,
    cx: &mut RootCx,
) -> Option<String> {
    match pattern {
        Some(p) if has_range => {
            cx.note(
                format!("temporal_pattern.{rm}.{node_id}"),
                format!(
                    "{rm} node {node_id:?} carries both an ISO8601 pattern ({p}) and a range; \
                     ADL2 defines the combined form for durations only — the range was kept, \
                     the pattern is recorded here"
                ),
            );
            None
        }
        other => other,
    }
}

/// Build the `Interval<Iso8601_*>` constraint list from a 1.4 temporal range
/// (string bounds), mirroring [`int_interval`]'s point/proper split.
#[expect(
    clippy::type_complexity,
    reason = "one tuple threading the six ADL 1.4 interval facets through a \
              single private helper"
)]
pub(super) fn temporal_interval<T>(
    range: Option<(
        Option<String>,
        Option<String>,
        bool,
        bool,
        Option<bool>,
        Option<bool>,
    )>,
    wrap: impl Fn(String) -> T,
) -> Vec<Interval<T>> {
    let Some((lower, upper, lower_unbounded, upper_unbounded, lower_included, upper_included)) =
        range
    else {
        return Vec::new();
    };
    if lower == upper && lower.is_some() {
        let v = lower.clone().unwrap_or_default();
        return vec![Interval::PointInterval(PointInterval {
            lower: lower.map(&wrap),
            upper: Some(wrap(v)),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        })];
    }
    vec![Interval::ProperInterval(ProperInterval::ProperInterval(
        ProperIntervalData {
            lower: lower.map(&wrap),
            upper: upper.map(&wrap),
            lower_unbounded,
            upper_unbounded,
            lower_included: lower_included.unwrap_or(!lower_unbounded),
            upper_included: upper_included.unwrap_or(!upper_unbounded),
        },
    ))]
}

pub(super) fn c_string(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    constraint: Vec<String>,
    assumed_value: Option<String>,
) -> CObject {
    CObject::CString(
        primitive_node(rm_type_name, node_id, occurrences).c_string(constraint, assumed_value),
    )
}
