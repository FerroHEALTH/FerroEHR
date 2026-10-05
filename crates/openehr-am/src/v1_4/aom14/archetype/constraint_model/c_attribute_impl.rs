// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `C_ATTRIBUTE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_attribute.adoc` §Functions,
//! plus the `c_multiple_attribute`/`c_single_attribute` pages for the two
//! concrete forms this abstract type dispatches over.

use crate::v1_4::aom14::archetype::constraint_model::c_attribute::CAttribute;
use crate::v1_4::aom14::archetype::constraint_model::c_multiple_attribute::CMultipleAttribute;
use crate::v1_4::aom14::archetype::constraint_model::c_object::CObject;
use crate::v1_4::aom14::archetype::constraint_model::c_single_attribute::CSingleAttribute;

impl CAttribute {
    /// Returns true if any value of the constrained reference-model attribute
    /// is allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom14.c_attribute.adoc` §Functions),
    /// post-condition `Result := children = Void or else children.is_empty`.
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        self.children().is_none_or(<[CObject]>::is_empty)
    }

    /// Returns the reference-model attribute name this node constrains.
    #[must_use]
    pub fn rm_attribute_name(&self) -> &str {
        match self {
            Self::CMultipleAttribute(a) => &a.rm_attribute_name,
            Self::CSingleAttribute(a) => &a.rm_attribute_name,
        }
    }

    /// Returns this attribute's child constraints, whichever concrete form it
    /// takes.
    #[must_use]
    pub fn children(&self) -> Option<&[CObject]> {
        match self {
            Self::CMultipleAttribute(a) => a.children.as_deref(),
            Self::CSingleAttribute(a) => a.children.as_deref(),
        }
    }
}

impl CMultipleAttribute {
    /// Returns the constraints on the members of this container attribute.
    ///
    /// `members` (`org.openehr.am.aom14.c_multiple_attribute.adoc` §Functions):
    /// "List of constraints representing members of the container value of this
    /// attribute within the data." The declared result multiplicity is `0..1`,
    /// matching the `0..1` `children` attribute it reads.
    #[must_use]
    pub fn members(&self) -> Option<&[CObject]> {
        self.children.as_deref()
    }
}

impl CSingleAttribute {
    /// Returns the alternative constraints for this attribute's single child.
    ///
    /// `alternatives` (`org.openehr.am.aom14.c_single_attribute.adoc`
    /// §Functions): "List of alternative constraints for the single child of
    /// this attribute within the data."
    #[must_use]
    pub fn alternatives(&self) -> Option<&[CObject]> {
        self.children.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_4::aom14::archetype::constraint_model::c_complex_object::CComplexObject;
    use crate::v1_4::aom14::archetype::constraint_model::cardinality::Cardinality;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::proper_interval::{
        ProperInterval, ProperIntervalData,
    };

    fn zero_to_many() -> Interval<i32> {
        Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(0),
            upper: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower_included: true,
            upper_included: false,
        }))
    }

    fn a_child() -> CObject {
        CObject::CComplexObject(CComplexObject {
            rm_type_name: "ELEMENT".to_owned(),
            occurrences: zero_to_many(),
            node_id: "at0001".to_owned(),
            assumed_value: None,
            attributes: None,
        })
    }

    fn single(children: Option<Vec<CObject>>) -> CSingleAttribute {
        CSingleAttribute {
            rm_attribute_name: "value".to_owned(),
            existence: zero_to_many(),
            children,
        }
    }

    fn multiple(children: Option<Vec<CObject>>) -> CMultipleAttribute {
        CMultipleAttribute {
            rm_attribute_name: "items".to_owned(),
            existence: zero_to_many(),
            children,
            cardinality: Cardinality {
                interval: zero_to_many(),
                is_ordered: true,
                is_unique: false,
            },
        }
    }

    #[test]
    fn an_attribute_with_no_children_allows_anything() {
        assert!(CAttribute::CSingleAttribute(single(None)).any_allowed());
        assert!(CAttribute::CSingleAttribute(single(Some(Vec::new()))).any_allowed());
        assert!(CAttribute::CMultipleAttribute(multiple(None)).any_allowed());
    }

    #[test]
    fn one_child_is_already_a_constraint() {
        assert!(!CAttribute::CSingleAttribute(single(Some(vec![a_child()]))).any_allowed());
        assert!(!CAttribute::CMultipleAttribute(multiple(Some(vec![a_child()]))).any_allowed());
    }

    #[test]
    fn members_and_alternatives_are_the_children_of_their_own_form() {
        assert_eq!(multiple(None).members(), None);
        assert_eq!(
            multiple(Some(vec![a_child()])).members().map(<[_]>::len),
            Some(1)
        );
        assert_eq!(single(None).alternatives(), None);
        assert_eq!(
            single(Some(vec![a_child()])).alternatives().map(<[_]>::len),
            Some(1)
        );
    }
}
