// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_COMPLEX_OBJECT` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_complex_object.adoc` §Functions
//! and `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance Semantics: C_OBJECT, whose bodies `C_COMPLEX_OBJECT` inherits.

use crate::v2_4::aom2::constraint_model::archetype_constraint::ArchetypeConstraint;
use crate::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use crate::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use crate::v2_4::aom2::constraint_model::c_object_impl::{NodeFacts, conforms, congruent};
use crate::v2_4::aom2::constraint_model::c_second_order::CSecondOrder;
use crate::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_base::v1_3::foundation_types::interval::multiplicity_interval::MultiplicityInterval;

impl CComplexObject {
    /// Returns the `node_id` of this object node.
    #[must_use]
    pub fn node_id(&self) -> &str {
        match self {
            Self::CArchetypeRoot(root) => &root.node_id,
            Self::CComplexObject(data) => &data.node_id,
        }
    }

    /// Returns the `occurrences` of this object node.
    #[must_use]
    pub fn occurrences(&self) -> Option<&MultiplicityInterval> {
        match self {
            Self::CArchetypeRoot(root) => root.occurrences.as_ref(),
            Self::CComplexObject(data) => data.occurrences.as_ref(),
        }
    }

    /// Returns this object node's parent, if it has one.
    #[must_use]
    pub fn parent(&self) -> Option<&ArchetypeConstraint> {
        match self {
            Self::CArchetypeRoot(root) => root.parent.as_ref(),
            Self::CComplexObject(data) => data.parent.as_deref(),
        }
    }

    /// Returns this object node's second-order constraint parent, if it has one.
    #[must_use]
    pub fn soc_parent(&self) -> Option<&CSecondOrder> {
        match self {
            Self::CArchetypeRoot(root) => root.soc_parent.as_ref(),
            Self::CComplexObject(data) => data.soc_parent.as_ref(),
        }
    }

    /// Returns the attribute constraints of this object node.
    #[must_use]
    pub fn attributes(&self) -> Option<&[CAttribute]> {
        match self {
            Self::CArchetypeRoot(root) => root.attributes.as_deref(),
            Self::CComplexObject(data) => data.attributes.as_deref(),
        }
    }

    /// Returns true if this object node carries a default value.
    ///
    /// `has_default_value` (`org.openehr.am.aom2.c_defined_object.adoc`
    /// §Functions), read against the `0..1` `default_value` attribute.
    #[must_use]
    pub fn has_default_value(&self) -> bool {
        match self {
            Self::CArchetypeRoot(root) => root.default_value.is_some(),
            Self::CComplexObject(data) => data.default_value.is_some(),
        }
    }

    /// Returns true if any instance of the constrained reference-model type
    /// would be allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_complex_object.adoc` §Functions),
    /// post-condition `Result = attributes.is_empty and not is_prohibited` —
    /// prohibition being `occurrences /= Void and then occurrences.is_prohibited`
    /// (`org.openehr.am.aom2.c_object.adoc` §Functions).
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        let attributes = self.attributes();
        let occurrences = self.occurrences();
        attributes.is_none_or(<[CAttribute]>::is_empty)
            && !occurrences.is_some_and(MultiplicityInterval::is_prohibited)
    }

    /// Returns the `rm_type_name` of this object node.
    #[must_use]
    pub fn rm_type_name(&self) -> &str {
        match self {
            Self::CArchetypeRoot(root) => &root.rm_type_name,
            Self::CComplexObject(data) => &data.rm_type_name,
        }
    }

    /// Returns the `sibling_order` of this object node.
    #[must_use]
    pub fn sibling_order(&self) -> Option<&SiblingOrder> {
        match self {
            Self::CArchetypeRoot(root) => root.sibling_order.as_ref(),
            Self::CComplexObject(data) => data.sibling_order.as_ref(),
        }
    }

    /// The node facts the `master04.5` conformance functions read.
    fn facts(&self) -> NodeFacts<'_> {
        NodeFacts {
            node_id: self.node_id(),
            rm_type_name: self.rm_type_name(),
            occurrences: self.occurrences(),
            sibling_order: self.sibling_order(),
        }
    }

    /// Returns true if this node on its own expresses the same or narrower
    /// constraints than `other`.
    ///
    /// `C_COMPLEX_OBJECT` declares `c_conforms_to` (BMM) but states no body of
    /// its own, so the `C_OBJECT` body applies (`master04.5` §Conformance
    /// Semantics: C_OBJECT). `rmcc` is the spec's reference-model conformance
    /// lambda and `owning_attribute` carries the Eiffel `parent` — `None` means
    /// `is_root`, which is the usual case for a definition root.
    #[must_use]
    pub fn c_conforms_to(
        &self,
        other: &CComplexObject,
        rmcc: &dyn Fn(&str, &str) -> bool,
        owning_attribute: Option<&CAttribute>,
    ) -> bool {
        conforms(&self.facts(), &other.facts(), rmcc, owning_attribute)
    }

    /// Returns true if this node expresses no constraints beyond `other`'s,
    /// node-id redefinition aside.
    ///
    /// As with [`CComplexObject::c_conforms_to`], the body is `C_OBJECT`'s
    /// `c_congruent_to` (`master04.5` §Conformance Semantics: C_OBJECT).
    #[must_use]
    pub fn c_congruent_to(
        &self,
        other: &CComplexObject,
        owning_attribute: Option<&CAttribute>,
    ) -> bool {
        congruent(&self.facts(), &other.facts(), owning_attribute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData;

    fn interval(lower: i32, upper: i32) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    fn object(
        attributes: Option<Vec<CAttribute>>,
        occurrences: Option<MultiplicityInterval>,
    ) -> CComplexObject {
        CComplexObject::CComplexObject(CComplexObjectData {
            parent: None,
            soc_parent: None,
            rm_type_name: "OBSERVATION".to_owned(),
            occurrences,
            node_id: "id1".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            attributes,
            attribute_tuples: None,
        })
    }

    fn an_attribute() -> CAttribute {
        CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "data".to_owned(),
            existence: None,
            children: None,
            differential_path: None,
            cardinality: None,
            is_multiple: false,
        }
    }

    #[test]
    fn no_attribute_constraints_and_no_prohibition_allows_anything() {
        assert!(object(None, None).any_allowed());
        assert!(object(Some(Vec::new()), Some(interval(0, 1))).any_allowed());
    }

    #[test]
    fn a_prohibited_node_allows_nothing_even_with_no_attributes() {
        assert!(!object(None, Some(interval(0, 0))).any_allowed());
    }

    #[test]
    fn one_attribute_constraint_is_already_a_constraint() {
        assert!(!object(Some(vec![an_attribute()]), None).any_allowed());
    }
}
