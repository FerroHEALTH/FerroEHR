// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `DV_QUANTITY.property`: the AOM 1.4 archetype-profile pseudo-attribute.
//!
//! AOM 1.4 `C_QUANTITY.property` names the physical property of a constrained
//! quantity (`AM/docs/UML/classes/org.openehr.am.aom14.c_quantity.adoc`), but
//! no RM release defines a `DV_QUANTITY.property` attribute
//! (`RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantity.adoc`) and
//! AOM 2 has no `C_DV_QUANTITY` (`AM/docs/AOM2/master04.3-constraint_model-second_order.adoc`
//! §Tuple Constraints). This module is the one owner of that fact: the 1.4→2
//! converter drops the constraint, and the ADL 2 RM pass refuses it (VCARM)
//! with a message naming the 1.4 route.

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use crate::adl14::log::ConversionLog;
use crate::adl14::walk::cco_data_mut;

/// Whether `attribute` on `rm_type` is the AOM 1.4 `DV_QUANTITY.property`
/// pseudo-attribute.
pub(crate) fn is_quantity_property(rm_type: &str, attribute: &str) -> bool {
    rm_type == "DV_QUANTITY" && attribute == "property"
}

/// What an ADL 2 author is told when a `DV_QUANTITY.property` constraint is
/// refused, appended to the VCARM message.
pub(crate) const ADL2_REFUSAL_ADVICE: &str = "`property` is the ADL 1.4 archetype-profile \
     attribute C_DV_QUANTITY.property, which no ADL 2 or RM release defines; upload the \
     archetype's ADL 1.4 source instead, which converts to ADL 2 without it";

/// Removes every `property` constraint from a `DV_QUANTITY` node, recording
/// each one in `log` with its path and constraint.
// NOTE: RM `data_types/master06-quantity_package.adoc` §Basic Semantics ("A valid units
// string always implies a measured property") outranks `ADL2/master04.4` §Tuple Constraints.
pub(super) fn drop_quantity_properties(
    cco: &mut CComplexObject,
    path: &str,
    log: &mut ConversionLog,
) {
    let Some(d) = cco_data_mut(cco) else { return };
    let here = format!("{path}[{}]", d.node_id);
    if let Some(attributes) = d.attributes.as_mut() {
        let rm_type = d.rm_type_name.as_str();
        attributes.retain(|attr| {
            if !is_quantity_property(rm_type, &attr.rm_attribute_name) {
                return true;
            }
            let constraint = attr
                .children
                .iter()
                .flatten()
                .find_map(|child| match child {
                    CObject::CTerminologyCode(tc) => Some(tc.constraint.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "(none)".to_owned());
            log.note(format!(
                "DV_QUANTITY property constraint {constraint:?} at {here}/property dropped: \
                 RM DV_QUANTITY defines no property attribute, and its units imply the property"
            ));
            false
        });
        if attributes.is_empty() {
            d.attributes = None;
        }
    }
    for attr in d.attributes.iter_mut().flatten() {
        let attr_path = format!("{here}/{}", attr.rm_attribute_name);
        for child in attr.children.iter_mut().flatten() {
            if let CObject::CComplexObject(child) = child {
                drop_quantity_properties(child, &attr_path, log);
            }
        }
    }
}
