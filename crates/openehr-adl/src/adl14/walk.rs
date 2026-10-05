// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Read-only definition traversals shared by the 1.4→2 conversion stages.
//!
//! The collectors here feed [`crate::adl14::convert`]'s code planning; the
//! mutable rewrites that consume their results stay in that module (they need
//! the converter state), and the multiplicity rewrites live in
//! `crate::adl14::multiplicity`. The one mutable helper here is
//! [`cco_data_mut`], the complex-object accessor every stage shares.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` module is
//! our own design (see the [`crate::adl14`] flag).

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::{
    CComplexObject, CComplexObjectData,
};
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// The mutable `C_COMPLEX_OBJECT` data, if this is a plain complex object.
///
/// NOTE: a 1.4 *source archetype* never contains an inline `C_ARCHETYPE_ROOT`
/// (only a flattened OPT does), so the `CArchetypeRoot` arm yields `None` and its
/// walk is a no-op. Feeding a flattened OPT-1.4 through here is a separate
/// capability (OPT-1.4 → ADL2 conversion, not performed by this source-archetype
/// converter). No openEHR spec governs 1.4→2 conversion — our own
/// design/extension.
pub(super) fn cco_data_mut(cco: &mut CComplexObject) -> Option<&mut CComplexObjectData> {
    match cco {
        CComplexObject::CComplexObject(d) => Some(d),
        CComplexObject::CArchetypeRoot(_) => None,
    }
}

/// Visit every node code (`C_COMPLEX_OBJECT` / proxy / slot) in document order.
pub(super) fn collect_node_codes(def: &CComplexObject, f: &mut impl FnMut(&str)) {
    if let CComplexObject::CComplexObject(d) = def {
        f(&d.node_id);
        for attr in d.attributes.iter().flatten() {
            for child in attr.children.iter().flatten() {
                collect_node_codes_obj(child, f);
            }
        }
        for tuple in d.attribute_tuples.iter().flatten() {
            for member in tuple.members.iter().flatten() {
                for child in member.children.iter().flatten() {
                    collect_node_codes_obj(child, f);
                }
            }
        }
    }
}

fn collect_node_codes_obj(obj: &CObject, f: &mut impl FnMut(&str)) {
    match obj {
        CObject::CComplexObject(cco) => collect_node_codes(cco, f),
        CObject::CComplexObjectProxy(p) => f(&p.node_id),
        CObject::ArchetypeSlot(s) => f(&s.node_id),
        _ => {}
    }
}

/// Visit every at-code used as a `local::…` *value* in a terminology constraint.
pub(super) fn collect_local_value_codes(def: &CComplexObject, f: &mut impl FnMut(&str)) {
    walk_constraints(def, &mut |raw, _| {
        if let Some((term, codes)) = raw.split_once("::")
            && term == "local"
        {
            for code in codes.split([',', ';']).map(str::trim) {
                if AdlCodeDefinitionsData::is_at_code(code) {
                    f(code);
                }
            }
        }
    });
}

/// Visit every `C_TERMINOLOGY_CODE.constraint` (with its enclosing element
/// rubric context — unused here, passed empty).
pub(super) fn walk_constraints(def: &CComplexObject, f: &mut impl FnMut(&str, &str)) {
    let CComplexObject::CComplexObject(d) = def else {
        return;
    };
    for attr in d.attributes.iter().flatten() {
        for child in attr.children.iter().flatten() {
            walk_constraints_obj(child, f);
        }
    }
    for tuple in d.attribute_tuples.iter().flatten() {
        walk_tuple_constraints(tuple, f);
    }
}

/// Visits the terminology codes of one attribute tuple: its members' children,
/// and its ROWS.
///
/// Tuple rows carry the actual primitive constraints (e.g. ordinal
/// `[value, symbol]` symbol codes), so their terminology codes are visited too
/// and value at-codes get planned and converted like attribute ones.
fn walk_tuple_constraints(
    tuple: &openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple,
    f: &mut impl FnMut(&str, &str),
) {
    for member in tuple.members.iter().flatten() {
        for child in member.children.iter().flatten() {
            walk_constraints_obj(child, f);
        }
    }
    for row in tuple.tuples.iter().flatten() {
        for m in &row.members {
            if let CPrimitiveObject::CTerminologyCode(tc) = m {
                f(&tc.constraint, "");
            }
        }
    }
}

fn walk_constraints_obj(obj: &CObject, f: &mut impl FnMut(&str, &str)) {
    match obj {
        CObject::CTerminologyCode(tc) => f(&tc.constraint, ""),
        CObject::CComplexObject(cco) => walk_constraints(cco, f),
        _ => {}
    }
}
