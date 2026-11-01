// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM attributes a Web Template node carries where the operational
//! template leaves them implicit, as constraint attributes both builders walk
//! like the template's own: the alternatives of an unconstrained
//! `ELEMENT.value` (OPT 1.4) and the `lower` and `upper` parts of a
//! `DV_INTERVAL<T>` (OPT 1.4 and OPT2).

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData;
use openehr_am::v2_4::aom2::constraint_model::{c_attribute, c_complex_object, c_object};
use openehr_its::opt14::types::{
    CAttribute, CComplexObject, CObject, CSingleAttribute, Intervalofinteger,
};

use super::inputs;

/// The parts of a `DV_INTERVAL<T>`, each of type `T` (RM `data_types`
/// §`DV_INTERVAL`; ITS-REST `simplified_formats/master05-rm_mapping.adoc`
/// §`DV_INTERVAL`: the FLAT paths `/lower` and `/upper`).
const INTERVAL_PARTS: [&str; 2] = ["lower", "upper"];

/// The parameter `T` of a `DV_INTERVAL<T>` RM type, `None` for another type.
fn interval_parameter(rm_type: &str) -> Option<&str> {
    rm_type.strip_prefix("DV_INTERVAL<")?.strip_suffix('>')
}

/// The `C_ATTRIBUTE.existence` interval of `attr` (AOM 1.4
/// `master04-constraint_model_package.adoc` §existence).
pub(super) fn attribute_existence(attr: &CAttribute) -> &Intervalofinteger {
    match attr {
        CAttribute::CSingleAttribute(s) => &s.existence,
        CAttribute::CMultipleAttribute(m) => &m.existence,
    }
}

/// The interval `0..1`, the RM existence of an optional single attribute.
fn optional() -> Intervalofinteger {
    Intervalofinteger {
        lower_included: Some(true),
        upper_included: Some(true),
        lower_unbounded: false,
        upper_unbounded: false,
        lower: Some(0),
        upper: Some(1),
    }
}

/// The OPT 1.4 constraint object an implicit attribute holds: an RM type with
/// no constraint on its own attributes.
fn open_object(rm_type: &str, occurrences: Intervalofinteger) -> CObject {
    CObject::CComplexObject(CComplexObject {
        rm_type_name: rm_type.to_owned(),
        occurrences,
        node_id: String::new(),
        attributes: Vec::new(),
    })
}

/// The concrete `DATA_VALUE` types an unconstrained `ELEMENT.value` admits, in
/// the order they become choice alternatives: every non-generic, non-deprecated
/// subtype of RM `data_types` (`DV_INTERVAL<T>` needs a parameter the OPT does
/// not give, `DV_PARAGRAPH` is deprecated) except `DV_STATE`.
// NOTE: simplified_formats master05-rm_mapping has no DV_STATE table, so no FLAT key
// can name one (#3936).
const ANY_ELEMENT_VALUE_TYPES: [&str; 17] = [
    "DV_TEXT",
    "DV_CODED_TEXT",
    "DV_MULTIMEDIA",
    "DV_PARSABLE",
    "DV_BOOLEAN",
    "DV_IDENTIFIER",
    "DV_URI",
    "DV_EHR_URI",
    "DV_DURATION",
    "DV_QUANTITY",
    "DV_COUNT",
    "DV_PROPORTION",
    "DV_DATE_TIME",
    "DV_TIME",
    "DV_ORDINAL",
    "DV_SCALE",
    "DV_DATE",
];

/// The `value` attribute an `ELEMENT` that leaves its value unconstrained
/// stands for: one open alternative per concrete `DATA_VALUE` type.
///
/// An `ELEMENT` with no `value` constraint, or with `value matches {*}`, admits
/// any value the RM permits (AOM 1.4 `C_COMPLEX_OBJECT.any_allowed`, ADL 1.4
/// `master05-cadl.adoc` §"Any" Constraints), so it keeps a node in the web
/// template and its FLAT keys name the chosen type (`…/test_method/text_value`,
/// ITS-REST `simplified_formats/master05-rm_mapping.adoc` §CLUSTER). Returns
/// `None` for every other node, and for a `value` the template prohibits.
pub(super) fn any_element_value(co: &CObject, rm_type: &str) -> Option<CAttribute> {
    if rm_type != "ELEMENT" {
        return None;
    }
    let declared = inputs::attributes(co)
        .iter()
        .find(|a| inputs::attribute_name(a) == "value");
    let existence = match declared {
        None => optional(),
        Some(attr) if inputs::attribute_children(attr).is_empty() => {
            let existence = attribute_existence(attr);
            if !existence.upper_unbounded && existence.upper == Some(0) {
                return None;
            }
            existence.clone()
        }
        Some(_) => return None,
    };
    let children = ANY_ELEMENT_VALUE_TYPES
        .iter()
        .map(|rm| {
            let occurrences = Intervalofinteger {
                lower: Some(0),
                ..existence.clone()
            };
            open_object(rm, occurrences)
        })
        .collect();
    Some(CAttribute::CSingleAttribute(CSingleAttribute {
        rm_attribute_name: "value".to_owned(),
        existence,
        children,
    }))
}

/// The `lower` and `upper` attributes of an OPT 1.4 `DV_INTERVAL<T>` node that
/// its template leaves unconstrained, each holding an open `T`.
///
/// A `DV_INTERVAL` has both parts whatever the template constrains (RM
/// `data_types` §`DV_INTERVAL`), and its FLAT form reaches them as `/lower` and
/// `/upper` (ITS-REST `simplified_formats/master05-rm_mapping.adoc`
/// §`DV_INTERVAL`), so the Web Template names both. A part the template
/// constrains, or prohibits with existence `0..0`, yields none.
pub(super) fn interval_parts_v1_4(co: &CObject, rm_type: &str) -> Vec<CAttribute> {
    let Some(parameter) = interval_parameter(rm_type) else {
        return Vec::new();
    };
    INTERVAL_PARTS
        .into_iter()
        .filter(|part| {
            inputs::attributes(co)
                .iter()
                .find(|a| inputs::attribute_name(a) == *part)
                .is_none_or(|attr| {
                    let existence = attribute_existence(attr);
                    inputs::attribute_children(attr).is_empty()
                        && (existence.upper_unbounded || existence.upper != Some(0))
                })
        })
        .map(|part| {
            CAttribute::CSingleAttribute(CSingleAttribute {
                rm_attribute_name: part.to_owned(),
                existence: optional(),
                children: vec![open_object(parameter, optional())],
            })
        })
        .collect()
}

/// The `lower` and `upper` attributes of an OPT2 `DV_INTERVAL<T>` node that its
/// template leaves unconstrained, each holding an open `T`; the OPT2 twin of
/// [`interval_parts_v1_4`] over the node's `declared` attributes.
pub(super) fn interval_parts_v2_4(
    rm_type: &str,
    declared: &[c_attribute::CAttribute],
) -> Vec<c_attribute::CAttribute> {
    let Some(parameter) = interval_parameter(rm_type) else {
        return Vec::new();
    };
    INTERVAL_PARTS
        .into_iter()
        .filter(|part| {
            declared
                .iter()
                .find(|a| a.rm_attribute_name == *part)
                .is_none_or(|attr| {
                    attr.children.as_deref().unwrap_or_default().is_empty()
                        && attr
                            .existence
                            .as_ref()
                            .is_none_or(|e| e.upper_unbounded || e.upper != Some(0))
                })
        })
        .map(|part| c_attribute::CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: part.to_owned(),
            existence: None,
            children: Some(vec![c_object::CObject::CComplexObject(
                c_complex_object::CComplexObject::CComplexObject(CComplexObjectData {
                    parent: None,
                    soc_parent: None,
                    rm_type_name: parameter.to_owned(),
                    occurrences: None,
                    node_id: String::new(),
                    alternative_ids: None,
                    is_deprecated: None,
                    sibling_order: None,
                    default_value: None,
                    attributes: None,
                    attribute_tuples: None,
                }),
            )]),
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        })
        .collect()
}
