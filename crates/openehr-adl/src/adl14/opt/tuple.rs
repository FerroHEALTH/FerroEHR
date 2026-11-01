// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The domain-type attribute tuples (`DV_ORDINAL`, `DV_QUANTITY`) and the
//! single-valued occurrences cap.

/// The `C_ATTRIBUTE` definition member of an attribute tuple (empty children —
/// the constraints live in the tuple rows).
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_base::prelude::{Interval, PointInterval};
use openehr_its::opt14;

use super::RootCx;
use super::primitive::{c_string, complex, terminology_code};
use super::value::{code_constraint, int_interval, map_mult, real_interval};

pub(super) fn tuple_member(rm_attribute_name: &str) -> CAttribute {
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: rm_attribute_name.to_owned(),
        existence: None,
        children: openehr_base::containers::present(Vec::new()),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// A tuple-row `C_INTEGER` member (point value, or unconstrained when `None`).
pub(super) fn tuple_integer(value: Option<i32>, range: Option<Interval<i32>>) -> CPrimitiveObject {
    CPrimitiveObject::CInteger(CInteger {
        parent: None,
        soc_parent: None,
        rm_type_name: "Integer".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(
            value
                .map(|v| {
                    Interval::PointInterval(PointInterval {
                        lower: Some(v),
                        upper: Some(v),
                        lower_unbounded: false,
                        upper_unbounded: false,
                        lower_included: true,
                        upper_included: true,
                    })
                })
                .into_iter()
                .chain(range)
                .collect::<Vec<_>>(),
        ),
    })
}

/// A tuple-row `C_REAL` member (range, or unconstrained when `None`).
pub(super) fn tuple_real(range: Option<Interval<f64>>) -> CPrimitiveObject {
    CPrimitiveObject::CReal(CReal {
        parent: None,
        soc_parent: None,
        rm_type_name: "Real".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(range.into_iter().collect()),
    })
}

/// A tuple-row `C_STRING` member (single-value list).
pub(super) fn tuple_string(value: &str) -> CPrimitiveObject {
    CPrimitiveObject::CString(CString {
        parent: None,
        soc_parent: None,
        rm_type_name: "String".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: Some(vec![value.to_owned()]),
    })
}

/// A tuple-row `C_TERMINOLOGY_CODE` member carrying the 1.4
/// `terminology::code` encoding the converter core rewrites (shifting local
/// at-codes, minting external ones).
pub(super) fn tuple_code(terminology: Option<&str>, code: &str) -> CPrimitiveObject {
    CPrimitiveObject::CTerminologyCode(CTerminologyCode {
        parent: None,
        soc_parent: None,
        rm_type_name: "CODE_PHRASE".to_owned(),
        occurrences: None,
        node_id: String::new(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: code_constraint(terminology, std::slice::from_ref(&code.to_owned()), None),
        constraint_status: None,
    })
}

/// `DV_STATE`: the 1.4 constrainer carries a state machine; the vendored AM
/// defines no ADL2/AOM2 constraint form for it (the tuple mechanism covers
/// co-varying attribute values, not state machines) — a loose domain-typed
/// complex object is the honest valid constraint. No vendored openEHR spec
/// governs `DV_STATE` conversion — our own design; recorded in
/// `conversion_details`.
pub(super) fn dv_state_loose(c: &opt14::types::CDvState, cx: &mut RootCx) -> CObject {
    cx.note(
        format!("dv_state.{}", c.node_id),
        format!(
            "C_DV_STATE {} state-machine constraint has no ADL2 form; emitted as an \
             unconstrained DV_STATE node",
            c.node_id
        ),
    );
    complex(&c.rm_type_name, &c.node_id, &c.occurrences, Vec::new())
}

/// A `CONSTRAINT_REF` names an ac-code whose definition lives in the
/// flattened ontology's `constraint_definitions` (AOM 1.4
/// `constraint_ref.adoc`); ADL2 folds those into
/// `term_definitions`/`term_bindings` (`master07.13` §Terminology section),
/// so the ac-code constraint resolves (VACDF/VTCBK). An ac-code the ontology
/// does NOT define would dangle — it stays an unconstrained node, reported in
/// `conversion_details`.
pub(super) fn map_constraint_ref(r: &opt14::types::ConstraintRef, cx: &mut RootCx) -> CObject {
    if cx.defined_acs.contains(&r.reference) {
        terminology_code(
            &r.rm_type_name,
            &r.node_id,
            &r.occurrences,
            r.reference.clone(),
        )
    } else {
        cx.note(
            format!("constraint_ref.{}", r.node_id),
            format!(
                "CONSTRAINT_REF {} names {} which the flattened ontology does not define; \
                 emitted unconstrained",
                r.node_id, r.reference
            ),
        );
        terminology_code(&r.rm_type_name, &r.node_id, &r.occurrences, String::new())
    }
}

/// `C_DV_ORDINAL` → the AOM2 `[value, symbol]` attribute tuple
/// (`master04.4-cadl_second_order.adoc` §Tuple Constraints). A 1.4
/// `assumed_value` has no per-tuple AOM2 slot — recorded in
/// `conversion_details`.
pub(super) fn ordinal_tuple(c: &opt14::types::CDvOrdinal, cx: &mut RootCx) -> CObject {
    if let Some(a) = &c.assumed_value {
        cx.note(
            format!("assumed_value.{}", c.node_id),
            format!(
                "C_DV_ORDINAL {} assumed_value (ordinal {}) has no AOM2 tuple representation; \
                 recorded here",
                c.node_id, a.value
            ),
        );
    }
    let tuples = c
        .list
        .iter()
        .map(|o| CPrimitiveTuple {
            // A `[value, symbol]` ordinal row always has two members, so the
            // `1..*` bound of `C_PRIMITIVE_TUPLE.members` holds by construction.
            members: {
                let mut row =
                    openehr_base::containers::NonEmptyVec::of(tuple_integer(Some(o.value), None));
                row.push(tuple_code(
                    Some(o.symbol.defining_code.terminology_id.value.as_str()),
                    &o.symbol.defining_code.code_string,
                ));
                row
            },
        })
        .collect::<Vec<_>>();
    let attribute_tuples = if tuples.is_empty() {
        Vec::new()
    } else {
        vec![CAttributeTuple {
            members: Some(vec![tuple_member("value"), tuple_member("symbol")]),
            tuples: openehr_base::containers::present(tuples),
        }]
    };
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: c.rm_type_name.clone(),
        occurrences: Some(map_mult(&c.occurrences)),
        node_id: c.node_id.clone(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(Vec::new()),
        attribute_tuples: openehr_base::containers::present(attribute_tuples),
    }))
}

/// `C_DV_QUANTITY` → a `property` terminology-code attribute plus the
/// `[units, magnitude(, precision)]` attribute tuple (`master04.4` §Tuple
/// Constraints). The precision member is included only when some item
/// constrains it (uniform tuple arity; rows without one carry an
/// unconstrained `C_INTEGER`). A 1.4 `assumed_value` has no per-tuple AOM2
/// slot — recorded in `conversion_details`.
pub(super) fn quantity_tuple(c: &opt14::types::CDvQuantity, cx: &mut RootCx) -> CObject {
    if c.assumed_value.is_some() {
        cx.note(
            format!("assumed_value.{}", c.node_id),
            format!(
                "C_DV_QUANTITY {} assumed_value has no AOM2 tuple representation; recorded here",
                c.node_id
            ),
        );
    }
    let mut attributes = Vec::new();
    if let Some(p) = &c.property {
        attributes.push(CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "property".to_owned(),
            existence: None,
            children: Some(vec![terminology_code(
                "CODE_PHRASE",
                "",
                &unbounded_occurrences(),
                code_constraint(
                    Some(p.terminology_id.value.as_str()),
                    std::slice::from_ref(&p.code_string),
                    None,
                ),
            )]),
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        });
    }
    // Tuple arity must be uniform, so a tuple is emitted only when EVERY item
    // constrains the magnitude; a mixed set widens to the plain units list,
    // which never rejects valid data (no openEHR spec governs 1.4→2 conversion
    // — our own design).
    let all_magnitude = !c.list.is_empty() && c.list.iter().all(|i| i.magnitude.is_some());
    let all_precision = !c.list.is_empty() && c.list.iter().all(|i| i.precision.is_some());
    let some_dropped = c
        .list
        .iter()
        .any(|i| i.magnitude.is_some() || i.precision.is_some());
    let mut attribute_tuples = Vec::new();
    if all_magnitude {
        if !all_precision && c.list.iter().any(|i| i.precision.is_some()) {
            cx.note(
                format!("quantity_precision.{}", c.node_id),
                format!(
                    "C_DV_QUANTITY {}: precision constrained on only some units; widened \
                     (dropped) to keep tuple arity uniform",
                    c.node_id
                ),
            );
        }
        let tuples = c
            .list
            .iter()
            .map(|item| {
                // A `[units, magnitude]` (optionally `+ precision`) row always
                // has at least two members, so the `1..*` bound of
                // `C_PRIMITIVE_TUPLE.members` holds by construction.
                let mut members =
                    openehr_base::containers::NonEmptyVec::of(tuple_string(&item.units));
                members.push(tuple_real(item.magnitude.as_ref().map(real_interval)));
                if all_precision {
                    members.push(tuple_integer(
                        None,
                        item.precision.as_ref().map(int_interval),
                    ));
                }
                CPrimitiveTuple { members }
            })
            .collect::<Vec<_>>();
        let mut members = vec![tuple_member("units"), tuple_member("magnitude")];
        if all_precision {
            members.push(tuple_member("precision"));
        }
        attribute_tuples.push(CAttributeTuple {
            members: openehr_base::containers::present(members),
            tuples: openehr_base::containers::present(tuples),
        });
    } else if !c.list.is_empty() {
        attributes.push(widened_units_attribute(c, some_dropped, cx));
    }
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name: c.rm_type_name.clone(),
        occurrences: Some(map_mult(&c.occurrences)),
        node_id: c.node_id.clone(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        attributes: openehr_base::containers::present(attributes),
        attribute_tuples: openehr_base::containers::present(attribute_tuples),
    }))
}

/// The plain `units` list attribute a magnitude-less (or mixed) quantity
/// widens to — the reference-corpus form for units-only constraints; a mixed
/// set's dropped per-unit ranges are reported.
pub(super) fn widened_units_attribute(
    c: &opt14::types::CDvQuantity,
    some_dropped: bool,
    cx: &mut RootCx,
) -> CAttribute {
    if some_dropped {
        cx.note(
            format!("quantity_magnitude.{}", c.node_id),
            format!(
                "C_DV_QUANTITY {}: magnitude/precision constrained on only some units; widened \
                 to the plain units list (the dropped per-unit ranges are unrepresentable \
                 without uniform tuple arity)",
                c.node_id
            ),
        );
    }
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: "units".to_owned(),
        existence: None,
        children: Some(vec![c_string(
            "String",
            "",
            &unbounded_occurrences(),
            c.list.iter().map(|i| i.units.clone()).collect(),
            None,
        )]),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// The child of a single-valued attribute with its occurrences capped at one,
/// or `None` when they already are.
///
/// ADL 1.4 `master05-cadl.adoc` §Occurrences: the value of a single-valued
/// attribute "can only be `0..1` or `1..1`", and a wider occurrences constraint
/// on it "is not illegal" (deployed ROLE templates state `0..*` under
/// `capabilities`). The ADL2 output states the cap, and the widening is
/// recorded in `conversion_details`.
pub(super) fn single_valued(
    child: &opt14::types::CObject,
    attr_path: &str,
    cx: &mut RootCx,
) -> Option<opt14::types::CObject> {
    let mut capped = child.clone();
    let occurrences = occurrences_mut(&mut capped);
    if !occurrences.upper_unbounded && occurrences.upper.is_none_or(|u| u <= 1) {
        return None;
    }
    occurrences.upper = Some(1);
    occurrences.upper_unbounded = false;
    occurrences.upper_included = Some(true);
    if occurrences.lower.is_some_and(|l| l > 1) {
        occurrences.lower = Some(1);
    }
    cx.note(
        format!("single_attribute_occurrences.{attr_path}"),
        format!(
            "{attr_path} is single-valued; its child's occurrences wider than 1 were capped \
             at 1 (ADL 1.4 cADL §Occurrences)"
        ),
    );
    Some(capped)
}

/// The `occurrences` of any OPT 1.4 object constraint.
pub(super) fn occurrences_mut(
    obj: &mut opt14::types::CObject,
) -> &mut opt14::types::Intervalofinteger {
    match obj {
        opt14::types::CObject::ArchetypeInternalRef(o) => &mut o.occurrences,
        opt14::types::CObject::ArchetypeSlot(o) => &mut o.occurrences,
        opt14::types::CObject::ConstraintRef(o) => &mut o.occurrences,
        opt14::types::CObject::CArchetypeRoot(o) => &mut o.occurrences,
        opt14::types::CObject::CCodePhrase(o) => &mut o.occurrences,
        opt14::types::CObject::CCodeReference(o) => &mut o.occurrences,
        opt14::types::CObject::CComplexObject(o) => &mut o.occurrences,
        opt14::types::CObject::CDefinedObject(o) => &mut o.occurrences,
        opt14::types::CObject::CDvOrdinal(o) => &mut o.occurrences,
        opt14::types::CObject::CDvQuantity(o) => &mut o.occurrences,
        opt14::types::CObject::CDvState(o) => &mut o.occurrences,
        opt14::types::CObject::CPrimitiveObject(o) => &mut o.occurrences,
        opt14::types::CObject::TComplexObject(o) => &mut o.occurrences,
    }
}

/// The `0..*` occurrences a synthesized single-child attribute carries (the
/// converter core elides RM-default multiplicities).
pub(super) fn unbounded_occurrences() -> opt14::types::Intervalofinteger {
    opt14::types::Intervalofinteger {
        lower_included: Some(true),
        upper_included: None,
        lower_unbounded: false,
        upper_unbounded: true,
        lower: Some(0),
        upper: None,
    }
}

/// An AOM2 term-binding target must be a `Uri` (`AOM2/master07`
/// §`term_bindings`)
/// and the ADL2 ODIN target a URI token; a 1.4 binding value that is a bare
/// code is wrapped in the converter core's fabricated URN form
/// (`urn:adl14:<terminology>:<code>` — `adl14::convert::external_at_code`'s
/// fallback; no openEHR spec governs 1.4→2 conversion, our own design).
pub(super) fn binding_uri(terminology: &str, value: &str) -> String {
    if value.contains("://") || value.starts_with("urn:") {
        value.to_owned()
    } else {
        format!("urn:adl14:{terminology}:{value}")
    }
}

/// The first numeric segment of an ac-code (`ac0007` → 7), for allocator
/// seeding.
pub(super) fn first_ac_num(code: &str) -> Option<i64> {
    code.strip_prefix("ac")?
        .split('.')
        .next()?
        .parse::<i64>()
        .ok()
}
