// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `C_COMPLEX_OBJECT` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_complex_object.adoc` §Functions.

use crate::v1_4::aom14::archetype::constraint_model::c_attribute::CAttribute;
use crate::v1_4::aom14::archetype::constraint_model::c_complex_object::CComplexObject;

impl CComplexObject {
    /// Returns true if any value of the constrained reference-model type is
    /// allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom14.c_complex_object.adoc` §Functions),
    /// post-condition `Result = attributes.is_empty`. The attribute is `0..1`,
    /// so absent and present-but-empty both read as "no constraint stated".
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        self.attributes
            .as_deref()
            .is_none_or(<[CAttribute]>::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_4::aom14::archetype::constraint_model::c_single_attribute::CSingleAttribute;
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

    fn object(attributes: Option<Vec<CAttribute>>) -> CComplexObject {
        CComplexObject {
            rm_type_name: "OBSERVATION".to_owned(),
            occurrences: zero_to_many(),
            node_id: "at0000".to_owned(),
            assumed_value: None,
            attributes,
        }
    }

    #[test]
    fn an_object_with_no_attribute_constraints_allows_anything() {
        assert!(object(None).any_allowed());
        assert!(object(Some(Vec::new())).any_allowed());
    }

    #[test]
    fn one_attribute_constraint_is_already_a_constraint() {
        let attribute = CAttribute::CSingleAttribute(CSingleAttribute {
            rm_attribute_name: "data".to_owned(),
            existence: zero_to_many(),
            children: None,
        });
        assert!(!object(Some(vec![attribute])).any_allowed());
    }
}
