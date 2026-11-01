// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The 1.4-shaped `v2_4` object constructors and the primitive, code-phrase and
//! code-reference constraint mappings.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date::CDate;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_date_time::CDateTime;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_duration::CDuration;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_time::CTime;
use openehr_base::prelude::{
    Interval, Iso8601Date, Iso8601DateTime, Iso8601Duration, Iso8601Time, PointInterval,
    ProperInterval, ProperIntervalData,
};
use openehr_its::opt14;

use super::RootCx;
use super::value::{enumerated_and_range, int_interval, map_mult, real_interval};

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

pub(super) fn terminology_code(
    rm_type_name: &str,
    node_id: &str,
    occurrences: &opt14::types::Intervalofinteger,
    constraint: String,
) -> CObject {
    CObject::CTerminologyCode(CTerminologyCode {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint,
        constraint_status: None,
    })
}

/// Maps a `C_PRIMITIVE_OBJECT` (its wrapped `C_PRIMITIVE`) to the matching
/// `v2_4` primitive constraint node.
///
/// The converter passes primitive nodes through untouched. A node without
/// `item` maps as the unconstrained constraint of the primitive type its
/// `rm_type_name` names (`unconstrained_primitive`); a name that is no AOM 1.4
/// primitive type maps to an unconstrained object of that type, reported in
/// `conversion_details`.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per primitive C_* struct literal — a flat mapping table"
)]
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
    match item {
        opt14::types::CPrimitive::CBoolean(p) => {
            let mut constraint = Vec::new();
            if p.true_valid {
                constraint.push(true);
            }
            if p.false_valid {
                constraint.push(false);
            }
            CObject::CBoolean(CBoolean {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value,
                is_enumerated_type_constraint: None,
                constraint: openehr_base::containers::present(constraint),
            })
        }
        opt14::types::CPrimitive::CString(p) => {
            let constraint = if !p.list.is_empty() {
                p.list.clone()
            } else if let Some(pat) = &p.pattern {
                vec![pat.clone()]
            } else {
                Vec::new()
            };
            c_string(rm, node_id, occ, constraint, p.assumed_value.clone())
        }
        opt14::types::CPrimitive::CInteger(p) => {
            let constraint = openehr_base::containers::present(enumerated_and_range(
                &p.list,
                p.range.as_ref().map(int_interval),
            ));
            CObject::CInteger(CInteger {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value.map(f64::from),
                is_enumerated_type_constraint: None,
                constraint,
            })
        }
        opt14::types::CPrimitive::CReal(p) => {
            let constraint = openehr_base::containers::present(enumerated_and_range(
                &p.list,
                p.range.as_ref().map(real_interval),
            ));
            CObject::CReal(CReal {
                parent: None,
                soc_parent: None,
                rm_type_name: rm.to_owned(),
                occurrences: Some(map_mult(occ)),
                node_id: node_id.to_owned(),
                alternative_ids: openehr_base::containers::present(Vec::new()),
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: p.assumed_value,
                is_enumerated_type_constraint: None,
                constraint,
            })
        }
        opt14::types::CPrimitive::CDate(p) => CObject::CDate(CDate {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p.assumed_value.clone().map(|value| Iso8601Date { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Date { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
        opt14::types::CPrimitive::CDateTime(p) => CObject::CDateTime(CDateTime {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p
                .assumed_value
                .clone()
                .map(|value| Iso8601DateTime { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601DateTime { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
        opt14::types::CPrimitive::CDuration(p) => CObject::CDuration(CDuration {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p
                .assumed_value
                .clone()
                .map(|value| Iso8601Duration { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Duration { value },
            )),
            pattern_constraint: p.pattern.clone(),
        }),
        opt14::types::CPrimitive::CTime(p) => CObject::CTime(CTime {
            parent: None,
            soc_parent: None,
            rm_type_name: rm.to_owned(),
            occurrences: Some(map_mult(occ)),
            node_id: node_id.to_owned(),
            alternative_ids: openehr_base::containers::present(Vec::new()),
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value: p.assumed_value.clone().map(|value| Iso8601Time { value }),
            is_enumerated_type_constraint: None,
            constraint: openehr_base::containers::present(temporal_interval(
                p.range.as_ref().map(|r| {
                    (
                        r.lower.clone(),
                        r.upper.clone(),
                        r.lower_unbounded,
                        r.upper_unbounded,
                        r.lower_included,
                        r.upper_included,
                    )
                }),
                |value| Iso8601Time { value },
            )),
            pattern_constraint: date_time_pattern(
                p.pattern.clone(),
                p.range.is_some(),
                rm,
                node_id,
                cx,
            ),
        }),
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
    CObject::CString(CString {
        parent: None,
        soc_parent: None,
        rm_type_name: rm_type_name.to_owned(),
        occurrences: Some(map_mult(occurrences)),
        node_id: node_id.to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(constraint),
    })
}
