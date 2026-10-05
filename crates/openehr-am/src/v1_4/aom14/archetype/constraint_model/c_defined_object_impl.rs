// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `C_DEFINED_OBJECT` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_defined_object.adoc` §Functions.

use crate::v1_4::aom14::archetype::constraint_model::c_defined_object::CDefinedObject;

impl CDefinedObject {
    /// Returns true if this constraint carries an assumed value.
    ///
    /// `has_assumed_value` (`org.openehr.am.aom14.c_defined_object.adoc`
    /// §Functions): "True if there is an assumed value", read against the
    /// `0..1` `assumed_value` attribute every concrete descendant carries.
    #[must_use]
    pub fn has_assumed_value(&self) -> bool {
        match self {
            Self::CCodedText(o) => o.assumed_value.is_some(),
            Self::CComplexObject(o) => o.assumed_value.is_some(),
            Self::COrdinal(o) => o.assumed_value.is_some(),
            Self::CPrimitiveObject(o) => o.assumed_value.is_some(),
            Self::CQuantity(o) => o.assumed_value.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_4::aom14::archetype::constraint_model::c_complex_object::CComplexObject;
    use openehr_base::v1_3::foundation_types::interval::interval::Interval;
    use openehr_base::v1_3::foundation_types::interval::proper_interval::{
        ProperInterval, ProperIntervalData,
    };

    #[expect(
        clippy::disallowed_types,
        reason = "the AOM 1.4 assumed_value slot is one of the adjudicated free-form JSON seams the generator emits as serde_json::Value, so a fixture for it has to name that type"
    )]
    fn complex(assumed_value: Option<serde_json::Value>) -> CDefinedObject {
        CDefinedObject::CComplexObject(CComplexObject {
            rm_type_name: "ELEMENT".to_owned(),
            occurrences: Interval::ProperInterval(ProperInterval::ProperInterval(
                ProperIntervalData {
                    lower: Some(0),
                    upper: None,
                    lower_unbounded: false,
                    upper_unbounded: true,
                    lower_included: true,
                    upper_included: false,
                },
            )),
            node_id: "at0001".to_owned(),
            assumed_value,
            attributes: None,
        })
    }

    #[test]
    fn an_absent_assumed_value_is_no_assumed_value() {
        assert!(!complex(None).has_assumed_value());
    }

    #[test]
    fn a_present_assumed_value_counts_even_when_it_is_json_null() {
        assert!(complex(Some(serde_json::Value::Null)).has_assumed_value());
    }
}
