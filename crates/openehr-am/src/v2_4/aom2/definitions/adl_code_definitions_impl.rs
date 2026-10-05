// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `ADL_CODE_DEFINITIONS` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.adl_code_definitions.adoc`
//! §Constants + §Functions and
//! `AM/docs/AOM2/master02-model_overview.adoc` §Utility Algorithms, which
//! carries the `codes_conformant` Eiffel body.

use crate::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

impl AdlCodeDefinitionsData {
    /// Returns true if `a_code` is an 'at' code.
    ///
    /// `is_at_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions), post-condition `Result = a_code.starts_with
    /// (At_code_leader)`.
    #[must_use]
    pub fn is_at_code(a_code: &str) -> bool {
        a_code.starts_with(Self::AT_CODE_LEADER)
    }

    /// Returns true if `a_code` is an 'at' code, i.e. a code representing a
    /// single terminology item.
    ///
    /// `is_value_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions), post-condition `Result = a_code.starts_with
    /// (Value_code_leader)`. `Value_code_leader` and `At_code_leader` are both
    /// `"at"`, so this and [`AdlCodeDefinitionsData::is_at_code`] answer alike.
    #[must_use]
    pub fn is_value_code(a_code: &str) -> bool {
        a_code.starts_with(Self::VALUE_CODE_LEADER)
    }

    /// Returns true if `a_code` is an 'id' code.
    ///
    /// `is_id_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions), post-condition `Result = a_code.starts_with
    /// (Id_code_leader)`.
    #[must_use]
    pub fn is_id_code(a_code: &str) -> bool {
        a_code.starts_with(Self::ID_CODE_LEADER)
    }

    /// Returns true if `a_code` is an 'ac' code, i.e. a code referring to a
    /// terminology value set.
    ///
    /// `is_value_set_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions), post-condition `Result = a_code.starts_with
    /// (Value_set_code_leader)`.
    #[must_use]
    pub fn is_value_set_code(a_code: &str) -> bool {
        a_code.starts_with(Self::VALUE_SET_CODE_LEADER)
    }

    /// Returns true if `a_code` is any kind of ADL archetype local code.
    ///
    /// `is_adl_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions), post-condition `Result = is_at_code (a_code) or else
    /// is_id_code (a_code) or else is_value_code (a_code) or else
    /// is_value_set_code (a_code)`.
    #[must_use]
    pub fn is_adl_code(a_code: &str) -> bool {
        Self::is_at_code(a_code)
            || Self::is_id_code(a_code)
            || Self::is_value_code(a_code)
            || Self::is_value_set_code(a_code)
    }

    /// Returns true if `a_child_code` conforms to `a_parent_code` in the sense
    /// of specialisation.
    ///
    /// `codes_conformant` (`master02-model_overview.adoc` §Utility Algorithms),
    /// verbatim: `is_valid_code (a_child_code) and then
    /// a_child_code.starts_with (a_parent_code) and then (a_child_code.count =
    /// a_parent_code.count or else a_child_code.item (a_parent_code.count + 1)
    /// = Specialisation_separator)`. The separator test is what stops `at00040`
    /// conforming to `at0004`.
    #[must_use]
    pub fn codes_conformant(a_child_code: &str, a_parent_code: &str) -> bool {
        Self::is_valid_code(a_child_code)
            && a_child_code.starts_with(a_parent_code)
            && (a_child_code.len() == a_parent_code.len()
                || a_child_code
                    .get(a_parent_code.len()..)
                    .and_then(|rest| rest.chars().next())
                    == Some(Self::SPECIALISATION_SEPARATOR))
    }

    /// Returns true if `a_code` has been specialised from a parent code.
    ///
    /// `is_redefined_code` (`org.openehr.am.aom2.adl_code_definitions.adoc`
    /// §Functions): "A code has been specialised if there is a non-zero code
    /// index anywhere above the last index", with the page's own examples
    /// `at0.0.1` → False and `at1.0.1` → True.
    #[must_use]
    pub fn is_redefined_code(a_code: &str) -> bool {
        let Some(numeric) = Self::numeric_part(a_code) else {
            return false;
        };
        let mut segments: Vec<&str> = numeric.split(Self::SPECIALISATION_SEPARATOR).collect();
        if segments.pop().is_none() || segments.is_empty() {
            return false;
        }
        segments
            .iter()
            .any(|segment| segment.bytes().any(|byte| byte != b'0'))
    }

    /// Returns true if `a_code` is a well-formed archetype local code.
    ///
    /// Not declared in the class table, but referenced by the
    /// `codes_conformant` body (`master02-model_overview.adoc` §Utility
    /// Algorithms); the value space is `Code_regex_pattern` — dot-separated
    /// numeric segments — behind one of the declared leaders.
    ///
    /// NOTE: `Code_regex_pattern` spells each segment `(0|[1-9][0-9]*)`, which
    /// rejects the zero-padded at-codes the same class page's
    /// `Root_code_regex_pattern` (`^(id1|at0000)(\.1)*$`) requires, so segments
    /// are read as `[0-9]+` here.
    #[must_use]
    pub fn is_valid_code(a_code: &str) -> bool {
        let Some(numeric) = Self::numeric_part(a_code) else {
            return false;
        };
        numeric
            .split(Self::SPECIALISATION_SEPARATOR)
            .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
    }

    /// The numeric part of a valid-leader code, or `None` when the string
    /// carries no declared leader or nothing behind it.
    fn numeric_part(a_code: &str) -> Option<&str> {
        let leader = if Self::is_id_code(a_code) {
            Self::ID_CODE_LEADER
        } else if Self::is_value_set_code(a_code) {
            Self::VALUE_SET_CODE_LEADER
        } else if Self::is_at_code(a_code) {
            Self::AT_CODE_LEADER
        } else {
            return None;
        };
        a_code.strip_prefix(leader).filter(|rest| !rest.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_leader_selects_its_own_code_kind() {
        assert!(AdlCodeDefinitionsData::is_at_code("at0004"));
        assert!(AdlCodeDefinitionsData::is_value_code("at0004"));
        assert!(AdlCodeDefinitionsData::is_id_code("id1"));
        assert!(AdlCodeDefinitionsData::is_value_set_code("ac1"));
        assert!(!AdlCodeDefinitionsData::is_at_code("id1"));
        assert!(!AdlCodeDefinitionsData::is_id_code("ac1"));
        assert!(AdlCodeDefinitionsData::is_adl_code("ac1"));
        assert!(!AdlCodeDefinitionsData::is_adl_code("XYZ"));
    }

    #[test]
    fn a_valid_code_is_a_leader_over_dotted_numeric_segments() {
        assert!(AdlCodeDefinitionsData::is_valid_code("at0000"));
        assert!(AdlCodeDefinitionsData::is_valid_code("at0004.0.1"));
        assert!(AdlCodeDefinitionsData::is_valid_code("id1.1.1"));
        assert!(AdlCodeDefinitionsData::is_valid_code("ac1"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("at"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("at.1"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("at0004."));
        assert!(!AdlCodeDefinitionsData::is_valid_code("foo"));
    }

    /// The `master02` §Utility Algorithms body, including the separator
    /// boundary that stops `at00040` conforming to `at0004`.
    #[test]
    fn conformance_requires_a_separator_boundary() {
        assert!(AdlCodeDefinitionsData::codes_conformant("at0004", "at0004"));
        assert!(AdlCodeDefinitionsData::codes_conformant(
            "at0004.1", "at0004"
        ));
        assert!(AdlCodeDefinitionsData::codes_conformant(
            "at0004.1.2",
            "at0004.1"
        ));
        assert!(!AdlCodeDefinitionsData::codes_conformant(
            "at00040", "at0004"
        ));
        assert!(!AdlCodeDefinitionsData::codes_conformant(
            "at0005", "at0004"
        ));
        assert!(!AdlCodeDefinitionsData::codes_conformant("nope", "nope"));
    }

    /// The class page's own examples for `is_redefined_code`.
    #[test]
    fn redefinition_is_a_non_zero_index_above_the_last() {
        assert!(!AdlCodeDefinitionsData::is_redefined_code("at0.0.1"));
        assert!(AdlCodeDefinitionsData::is_redefined_code("at1.0.1"));
        assert!(AdlCodeDefinitionsData::is_redefined_code("at0004.1"));
        assert!(!AdlCodeDefinitionsData::is_redefined_code("at0004"));
        assert!(!AdlCodeDefinitionsData::is_redefined_code("nope"));
    }
}
