// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Constructors for the generated AOM2 constraint model.
//!
//! The generated `openehr_am::v2_4::aom2` structs are plain records with every
//! field spelled out (including the `parent`/`soc_parent` back-references the
//! emitter leaves unset; back-references are hand-wired, never emitted owning
//! fields). Building one by
//! hand at each call site is noise, so the shapes the parser, the flattener, the
//! ADL 1.4 lowering, and the OPT generator all need live here once.
//!
//! Spec oracle for the shapes themselves:
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! (`C_OBJECT` / `C_ATTRIBUTE` / `C_PRIMITIVE_OBJECT` class definitions) and
//! `docs/specs/openehr/BASE/docs/foundation_types/master05-interval.adoc`
//! (`INTERVAL` / `MULTIPLICITY_INTERVAL`).

#![expect(
    clippy::disallowed_types,
    reason = "ODIN-to-JSON conversion targets the JSON data model by specification (LANG odin \
              spec) (#1694)"
)]

use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_real::CReal;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_string::CString;
use openehr_base::prelude::{
    Interval, MultiplicityInterval, PointInterval, ProperInterval, ProperIntervalData,
    TerminologyCode,
};
use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;

/// Build a [`MultiplicityInterval`].
pub(crate) fn mult(
    lower: Option<i32>,
    upper: Option<i32>,
    lower_unbounded: bool,
    upper_unbounded: bool,
) -> MultiplicityInterval {
    MultiplicityInterval {
        lower,
        upper,
        lower_unbounded,
        upper_unbounded,
        lower_included: !lower_unbounded,
        upper_included: !upper_unbounded,
    }
}

/// A closed point interval `{v}`.
pub(crate) fn point_interval<T: Clone>(v: T) -> Interval<T> {
    Interval::PointInterval(PointInterval {
        lower: Some(v.clone()),
        upper: Some(v),
        lower_unbounded: false,
        upper_unbounded: false,
        lower_included: true,
        upper_included: true,
    })
}

/// A proper interval with explicit bounds/inclusivity.
// The four flags mirror `ProperIntervalData`'s own boolean fields 1:1.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the four flags mirror `ProperIntervalData`'s own boolean fields 1:1 — collapsing them into a struct would just restate that type"
)]
pub(crate) fn proper_interval<T>(
    lower: Option<T>,
    upper: Option<T>,
    lower_included: bool,
    upper_included: bool,
    lower_unbounded: bool,
    upper_unbounded: bool,
) -> Interval<T> {
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower,
        upper,
        lower_unbounded,
        upper_unbounded,
        lower_included,
        upper_included,
    }))
}

/// A closed real point interval `{v}`.
pub(crate) fn point_real(v: f64) -> Interval<f64> {
    Interval::PointInterval(PointInterval {
        lower: Some(v),
        upper: Some(v),
        lower_unbounded: false,
        upper_unbounded: false,
        lower_included: true,
        upper_included: true,
    })
}

/// A closed integer point interval `{v}`, saturating `v` into the AOM2
/// `Integer` range.
pub(crate) fn point_int(v: i64) -> Interval<i32> {
    // Domain-list integer constraints (precision, counts) are small clinical
    // values; saturate defensively into `i32` (AOM2 uses `Integer` = `i32`).
    let v = i32::try_from(v).unwrap_or(if v.is_negative() { i32::MIN } else { i32::MAX });
    Interval::PointInterval(PointInterval {
        lower: Some(v),
        upper: Some(v),
        lower_unbounded: false,
        upper_unbounded: false,
        lower_included: true,
        upper_included: true,
    })
}

/// Build a [`CComplexObjectData`] wrapped as a [`CObject`].
pub(crate) fn complex_object(
    rm_type_name: String,
    node_id: String,
    attributes: Vec<CAttribute>,
    attribute_tuples: Vec<CAttributeTuple>,
    default_value: Option<serde_json::Value>,
) -> CObject {
    CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
        parent: None,
        soc_parent: None,
        rm_type_name,
        occurrences: None,
        node_id,
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value,
        attributes: openehr_base::containers::present(attributes),
        attribute_tuples: openehr_base::containers::present(attribute_tuples),
    }))
}

/// Convert a parsed complex object into a [`CArchetypeRoot`] carrying
/// `archetype_ref` (the OPT-inlined slot-filler / external-reference form,
/// OPT2 master03). A non-complex `obj` (a primitive) cannot bear an archetype
/// ref; it is returned unchanged (validation flags the misuse).
pub(crate) fn into_archetype_root(obj: CObject, archetype_ref: String) -> CObject {
    let CObject::CComplexObject(CComplexObject::CComplexObject(d)) = obj else {
        return obj;
    };
    CObject::CComplexObject(CComplexObject::CArchetypeRoot(Box::new(CArchetypeRoot {
        parent: None,
        soc_parent: None,
        rm_type_name: d.rm_type_name,
        occurrences: d.occurrences,
        node_id: d.node_id,
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: d.default_value,
        attributes: d.attributes,
        attribute_tuples: d.attribute_tuples,
        archetype_ref,
    })))
}

/// A tuple member `C_ATTRIBUTE` (name only; the values live in the tuples).
pub(crate) fn tuple_member(rm_attribute_name: String) -> CAttribute {
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name,
        existence: None,
        children: openehr_base::containers::present(Vec::new()),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// A single-child `C_ATTRIBUTE` named `name`.
pub(crate) fn cattr_single(name: &str, child: CObject) -> CAttribute {
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: name.to_owned(),
        existence: None,
        children: Some(vec![child]),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// A childless `C_ATTRIBUTE` named `name`.
pub(crate) fn cattr_empty(name: &str) -> CAttribute {
    CAttribute {
        parent: None,
        soc_parent: None,
        rm_attribute_name: name.to_owned(),
        existence: None,
        children: openehr_base::containers::present(Vec::new()),
        differential_path: None,
        cardinality: None,
        is_multiple: false,
    }
}

/// A `C_STRING` carrying a single regex constraint (`/re/`, delimiters kept).
pub(crate) fn cstring_regex(regex: String, assumed: Option<String>) -> CString {
    CString {
        parent: None,
        soc_parent: None,
        rm_type_name: "String".to_owned(),
        occurrences: None,
        node_id: "Primitive_node_id".to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: assumed,
        is_enumerated_type_constraint: None,
        constraint: Some(vec![regex]),
    }
}

/// A `C_STRING` carrying a literal value list.
pub(crate) fn cstring_values(values: &[String]) -> CString {
    CString {
        parent: None,
        soc_parent: None,
        rm_type_name: "String".to_owned(),
        occurrences: None,
        node_id: "Primitive_node_id".to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(values.to_vec()),
    }
}

/// A `C_REAL` carrying an interval-list constraint.
pub(crate) fn creal_values(constraint: Vec<Interval<f64>>) -> CReal {
    CReal {
        parent: None,
        soc_parent: None,
        rm_type_name: "Real".to_owned(),
        occurrences: None,
        node_id: "Primitive_node_id".to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(constraint),
    }
}

/// A `C_INTEGER` carrying an interval-list constraint.
pub(crate) fn cinteger_values(constraint: Vec<Interval<i32>>) -> CInteger {
    CInteger {
        parent: None,
        soc_parent: None,
        rm_type_name: "Integer".to_owned(),
        occurrences: None,
        node_id: "Primitive_node_id".to_owned(),
        alternative_ids: openehr_base::containers::present(Vec::new()),
        is_deprecated: None,
        sibling_order: None,
        default_value: None,
        assumed_value: None,
        is_enumerated_type_constraint: None,
        constraint: openehr_base::containers::present(constraint),
    }
}

/// A local (archetype-internal) at-code terminology value.
pub(crate) fn local_term_code(code: &str) -> TerminologyCode {
    TerminologyCode {
        terminology_id: LOCAL_TERMINOLOGY_ID.to_owned(),
        terminology_version: None,
        code_string: code.to_owned(),
        uri: None,
    }
}

/// Widen a [`CPrimitiveObject`] to the corresponding [`CObject`] variant.
pub(crate) fn primitive_to_cobject(p: CPrimitiveObject) -> CObject {
    match p {
        CPrimitiveObject::CString(c) => CObject::CString(c),
        CPrimitiveObject::CReal(c) => CObject::CReal(c),
        CPrimitiveObject::CInteger(c) => CObject::CInteger(c),
        CPrimitiveObject::CBoolean(c) => CObject::CBoolean(c),
        CPrimitiveObject::CDate(c) => CObject::CDate(c),
        CPrimitiveObject::CDateTime(c) => CObject::CDateTime(c),
        CPrimitiveObject::CDuration(c) => CObject::CDuration(c),
        CPrimitiveObject::CTerminologyCode(c) => CObject::CTerminologyCode(c),
        CPrimitiveObject::CTime(c) => CObject::CTime(c),
    }
}

/// Narrow a [`CObject`] to the corresponding [`CPrimitiveObject`], or `None` if
/// it is not a `C_PRIMITIVE_OBJECT` descendant.
pub(crate) fn cobject_to_primitive(o: &CObject) -> Option<CPrimitiveObject> {
    Some(match o {
        CObject::CBoolean(c) => CPrimitiveObject::CBoolean(c.clone()),
        CObject::CDate(c) => CPrimitiveObject::CDate(c.clone()),
        CObject::CDateTime(c) => CPrimitiveObject::CDateTime(c.clone()),
        CObject::CDuration(c) => CPrimitiveObject::CDuration(c.clone()),
        CObject::CInteger(c) => CPrimitiveObject::CInteger(c.clone()),
        CObject::CReal(c) => CPrimitiveObject::CReal(c.clone()),
        CObject::CString(c) => CPrimitiveObject::CString(c.clone()),
        CObject::CTerminologyCode(c) => CPrimitiveObject::CTerminologyCode(c.clone()),
        CObject::CTime(c) => CPrimitiveObject::CTime(c.clone()),
        _ => return None,
    })
}
