// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_COMPLEX_OBJECT_PROXY` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_complex_object_proxy.adoc`
//! §Functions.

use crate::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use crate::v2_4::aom2::constraint_model::c_object::CObject;
use crate::v2_4::aom2::constraint_model::c_object_impl::{NodeFacts, occurrences_conform};

impl CComplexObjectProxy {
    /// Returns true if the target node's occurrences apply to this proxy.
    ///
    /// `use_target_occurrences`
    /// (`org.openehr.am.aom2.c_complex_object_proxy.adoc` §Functions),
    /// post-condition `Result = (occurrences = Void)`.
    #[must_use]
    pub fn use_target_occurrences(&self) -> bool {
        self.occurrences.is_none()
    }

    /// Returns true if this proxy's occurrences conform to `other`'s.
    ///
    /// `occurrences_conforms_to`
    /// (`org.openehr.am.aom2.c_complex_object_proxy.adoc` §Functions,
    /// redefined): "If `other` is a `C_COMPLEX_OBJECT`, then always `True`,
    /// since if occurrences defined on proxy node, it is an override of the
    /// occurrences on the target"; against another proxy "normal occurrences
    /// apply", i.e. the `C_OBJECT` rule.
    ///
    /// NOTE: that page spells the second case `C_COMPLEX_OBJECT` too, which
    /// would make both branches the same test; the proxy reading is the one
    /// its own sentence ("the override is of another use_node") requires.
    #[must_use]
    pub fn occurrences_conforms_to(&self, other: &CObject) -> bool {
        let CObject::CComplexObjectProxy(target) = other else {
            return true;
        };
        occurrences_conform(&self.facts(), &target.facts())
    }

    /// The node facts the `master04.5` conformance functions read.
    fn facts(&self) -> NodeFacts<'_> {
        NodeFacts {
            node_id: &self.node_id,
            rm_type_name: &self.rm_type_name,
            occurrences: self.occurrences.as_ref(),
            sibling_order: self.sibling_order.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::foundation_types::interval::multiplicity_interval::MultiplicityInterval;

    fn proxy(occurrences: Option<MultiplicityInterval>) -> CComplexObjectProxy {
        CComplexObjectProxy {
            parent: None,
            soc_parent: None,
            rm_type_name: "CLUSTER".to_owned(),
            occurrences,
            node_id: "id3".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            target_path: "/data[id2]".to_owned(),
        }
    }

    #[test]
    fn an_unset_occurrences_defers_to_the_target() {
        assert!(proxy(None).use_target_occurrences());
    }

    #[test]
    fn a_local_occurrences_overrides_the_target() {
        let local = MultiplicityInterval {
            lower: Some(0),
            upper: Some(1),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        };
        assert!(!proxy(Some(local)).use_target_occurrences());
    }
}
