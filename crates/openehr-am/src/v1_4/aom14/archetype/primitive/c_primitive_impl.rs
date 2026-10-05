// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM 1.4 `C_PRIMITIVE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom14.c_primitive.adoc` §Functions.

use crate::v1_4::aom14::archetype::primitive::c_primitive::CPrimitive;

impl CPrimitive {
    /// Returns true if this primitive constraint carries an assumed value.
    ///
    /// `has_assumed_value` (`org.openehr.am.aom14.c_primitive.adoc`
    /// §Functions): "True if there is an assumed value", read against the
    /// `0..1` `assumed_value` attribute every concrete descendant carries.
    #[must_use]
    pub fn has_assumed_value(&self) -> bool {
        match self {
            Self::CBoolean(c) => c.assumed_value.is_some(),
            Self::CDate(c) => c.assumed_value.is_some(),
            Self::CDateTime(c) => c.assumed_value.is_some(),
            Self::CDuration(c) => c.assumed_value.is_some(),
            Self::CInteger(c) => c.assumed_value.is_some(),
            Self::CReal(c) => c.assumed_value.is_some(),
            Self::CString(c) => c.assumed_value.is_some(),
            Self::CTime(c) => c.assumed_value.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_4::aom14::archetype::primitive::c_boolean::CBoolean;
    use crate::v1_4::aom14::archetype::primitive::c_string::CString;

    #[test]
    fn an_absent_assumed_value_is_no_assumed_value() {
        let c = CPrimitive::CBoolean(CBoolean {
            assumed_value: None,
            true_valid: true,
            false_valid: true,
        });
        assert!(!c.has_assumed_value());
    }

    #[test]
    fn a_present_assumed_value_is_reported_per_descendant() {
        let boolean = CPrimitive::CBoolean(CBoolean {
            assumed_value: Some(false),
            true_valid: true,
            false_valid: true,
        });
        assert!(boolean.has_assumed_value());
        let string = CPrimitive::CString(CString {
            assumed_value: Some(String::new()),
            pattern: None,
            list: None,
            list_open: false,
        });
        assert!(string.has_assumed_value());
    }
}
