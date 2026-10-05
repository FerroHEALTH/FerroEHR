// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `P_C_OBJECT` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/AOM2/master09-serialisation_model.adoc` (the `P_` classes are the
//! serialisation mirror of the AOM classes),
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_object.adoc` §Functions (the
//! mirrored `specialisation_depth` definition), and
//! `AM/docs/AOM2/master07-terminology_package.adoc` §Specialisation Depth.

use crate::v2_4::aom2::constraint_model::c_object_impl::specialisation_depth_of;
use crate::v2_4::aom2::persistence::constraint_model::p_c_object::PCObject;

impl PCObject {
    /// Returns the `node_id` of this persisted object node.
    #[must_use]
    pub fn node_id(&self) -> &str {
        match self {
            Self::PArchetypeSlot(o) => &o.node_id,
            Self::PCBoolean(o) => &o.node_id,
            Self::PCComplexObject(o) => match o {
                crate::v2_4::aom2::persistence::constraint_model::p_c_complex_object::PCComplexObject::PCArchetypeRoot(r) => &r.node_id,
                crate::v2_4::aom2::persistence::constraint_model::p_c_complex_object::PCComplexObject::PCComplexObject(d) => &d.node_id,
            },
            Self::PCComplexObjectProxy(o) => &o.node_id,
            Self::PCString(o) => &o.node_id,
            Self::PCTerminologyCode(o) => &o.node_id,
        }
    }

    /// Returns the specialisation level of this node, from its `node_id`.
    ///
    /// `specialisation_depth` (`org.openehr.am.aom2.p_c_object.adoc`
    /// §Functions) carries no meaning text of its own; `P_C_OBJECT` is the
    /// serialisation mirror of `C_OBJECT`, whose own page defines the result as
    /// "the number of '.' characters in the `node_id` code", with `-1` when
    /// `node_id` is not set.
    #[must_use]
    pub fn specialisation_depth(&self) -> i32 {
        specialisation_depth_of(self.node_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::persistence::primitive::p_c_string::PCString;
    use openehr_base::containers::NonEmptyVec;

    fn string(node_id: &str) -> PCObject {
        PCObject::PCString(PCString {
            rm_type_name: "String".to_owned(),
            occurrences: None,
            node_id: node_id.to_owned(),
            is_deprecated: None,
            is_frozen: None,
            default_value: String::new(),
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint: NonEmptyVec::of(".*".to_owned()),
        })
    }

    #[test]
    fn the_depth_is_the_separator_count_of_the_node_id() {
        assert_eq!(string("id1").specialisation_depth(), 0);
        assert_eq!(string("at0004.1").specialisation_depth(), 1);
        assert_eq!(string("at0004.0.1").specialisation_depth(), 2);
    }

    #[test]
    fn an_unset_node_id_yields_minus_one() {
        assert_eq!(string("").specialisation_depth(), -1);
    }
}
