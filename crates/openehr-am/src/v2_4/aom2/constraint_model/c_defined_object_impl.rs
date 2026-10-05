// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_DEFINED_OBJECT` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_defined_object.adoc` §Functions,
//! with `any_allowed` effected on the concrete descendants
//! (`c_complex_object.adoc`, `c_boolean.adoc`, `c_string.adoc`,
//! `c_ordered.adoc`/`c_temporal.adoc`, `c_terminology_code.adoc`).

use crate::v2_4::aom2::constraint_model::c_defined_object::CDefinedObject;

impl CDefinedObject {
    /// Returns true if this constraint carries a default value.
    ///
    /// `has_default_value` (`org.openehr.am.aom2.c_defined_object.adoc`
    /// §Functions), read against the `0..1` `default_value` attribute every
    /// concrete descendant carries.
    #[must_use]
    pub fn has_default_value(&self) -> bool {
        match self {
            Self::CBoolean(c) => c.default_value.is_some(),
            Self::CComplexObject(c) => c.has_default_value(),
            Self::CDate(c) => c.default_value.is_some(),
            Self::CDateTime(c) => c.default_value.is_some(),
            Self::CDuration(c) => c.default_value.is_some(),
            Self::CInteger(c) => c.default_value.is_some(),
            Self::CReal(c) => c.default_value.is_some(),
            Self::CString(c) => c.default_value.is_some(),
            Self::CTerminologyCode(c) => c.default_value.is_some(),
            Self::CTime(c) => c.default_value.is_some(),
        }
    }

    /// Returns true if any value of the constrained type would be allowed.
    ///
    /// `any_allowed` (`org.openehr.am.aom2.c_defined_object.adoc` §Functions) is
    /// abstract — "Redefined in descendants" — so this dispatches to each
    /// concrete descendant's own effecting.
    #[must_use]
    pub fn any_allowed(&self) -> bool {
        match self {
            Self::CBoolean(c) => c.any_allowed(),
            Self::CComplexObject(c) => c.any_allowed(),
            Self::CString(c) => c.any_allowed(),
            Self::CTerminologyCode(c) => c.any_allowed(),
            Self::CDate(c) => empty_range_and_pattern(
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CDateTime(c) => empty_range_and_pattern(
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CDuration(c) => empty_range_and_pattern(
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CTime(c) => empty_range_and_pattern(
                c.constraint.as_ref().is_none_or(Vec::is_empty),
                c.pattern_constraint.as_deref(),
            ),
            Self::CInteger(c) => c.constraint.as_ref().is_none_or(Vec::is_empty),
            Self::CReal(c) => c.constraint.as_ref().is_none_or(Vec::is_empty),
        }
    }
}

/// The `C_TEMPORAL.any_allowed` post-condition: an empty range constraint AND an
/// empty pattern constraint (`org.openehr.am.aom2.c_temporal.adoc` §Functions).
fn empty_range_and_pattern(range_empty: bool, pattern: Option<&str>) -> bool {
    range_empty && pattern.is_none_or(str::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::primitive::c_boolean::CBoolean;

    fn boolean(default_value: Option<bool>, constraint: Option<Vec<bool>>) -> CDefinedObject {
        CDefinedObject::CBoolean(CBoolean {
            parent: None,
            soc_parent: None,
            rm_type_name: "Boolean".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value,
            assumed_value: None,
            is_enumerated_type_constraint: None,
            constraint,
        })
    }

    #[test]
    fn a_default_value_is_reported_per_descendant() {
        assert!(!boolean(None, None).has_default_value());
        assert!(boolean(Some(true), None).has_default_value());
    }

    #[test]
    fn any_allowed_dispatches_to_the_descendant_effecting() {
        assert!(boolean(None, None).any_allowed());
        assert!(!boolean(None, Some(vec![true])).any_allowed());
    }
}
