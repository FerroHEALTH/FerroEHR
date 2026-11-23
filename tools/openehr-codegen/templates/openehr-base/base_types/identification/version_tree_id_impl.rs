// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Invariants and accessors for `VERSION_TREE_ID` (BASE 1.3.0
//! `org.openehr.base.base_types.version_tree_id.adoc`):
//! `trunk_version [ '.' branch_number '.' branch_version ]`.
//!
//! `Value_valid`, `Trunk_version_valid`, `Branch_number_valid` and
//! `Branch_version_valid` are checked; `Branch_validity`,
//! `Is_branch_validity` and `Is_first_validity` hold by construction, since the
//! accessors derive from one decomposition. `Value_lexical_form_valid` is our
//! own name for the master05 §Syntaxes production.

use super::lexical::{IdError, is_positive_int};
use super::version_tree_id::VersionTreeId;
use crate::validate::{InvariantViolation, Validate};
use std::str::FromStr;

/// Trunk version: `[1-9][0-9]*` (numbering starts at 1).
fn is_trunk(s: &str) -> bool {
    is_positive_int(s)
}

/// Branch version: `[1-9][0-9]*.[1-9][0-9]*.[1-9][0-9]*`.
fn is_branch_form(s: &str) -> bool {
    matches!(s.split('.').collect::<Vec<_>>().as_slice(),
        [trunk, branch_no, branch_ver]
        if is_trunk(trunk) && is_positive_int(branch_no) && is_positive_int(branch_ver))
}

/// `true` if `value` is a well-formed trunk or branch identifier.
#[must_use]
pub(crate) fn is_valid_version_tree(value: &str) -> bool {
    is_trunk(value) || is_branch_form(value)
}

impl VersionTreeId {
    /// The trunk version number (BASE `VERSION_TREE_ID.trunk_version`): the part
    /// before the first `.`, or the whole value if there is no `.`.
    #[must_use]
    pub fn trunk_version(&self) -> &str {
        self.value
            .split_once('.')
            .map_or(self.value.as_str(), |(t, _)| t)
    }

    /// `true` if this identifier represents a branch, i.e. has three
    /// `.`-separated parts (BASE `VERSION_TREE_ID.is_branch`).
    #[must_use]
    pub fn is_branch(&self) -> bool {
        self.value.split('.').count() == 3
    }

    /// `true` if this is the first version, i.e. `trunk_version` is `1` (BASE
    /// `VERSION_TREE_ID.is_first`).
    #[must_use]
    pub fn is_first(&self) -> bool {
        self.trunk_version() == "1"
    }

    /// The branch number (the second `.`-separated part), or `None` for a trunk
    /// identifier (BASE `VERSION_TREE_ID.branch_number`).
    #[must_use]
    pub fn branch_number(&self) -> Option<&str> {
        match self.value.split('.').collect::<Vec<_>>().as_slice() {
            [_, b, _] => Some(b),
            _ => None,
        }
    }

    /// The branch version (the third `.`-separated part), or `None` for a trunk
    /// identifier (BASE `VERSION_TREE_ID.branch_version`).
    #[must_use]
    pub fn branch_version(&self) -> Option<&str> {
        match self.value.split('.').collect::<Vec<_>>().as_slice() {
            [_, _, v] => Some(v),
            _ => None,
        }
    }
}

impl VersionTreeId {
    /// Builds a `VERSION_TREE_ID`, each part at least 1 (master05 §Syntaxes; RM
    /// `master06-change_control_package.adoc` §"The 'Virtual Version Tree'").
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value; [`IdError::VersionTree`] for
    /// anything that is neither a bare trunk (`N`) nor a full `N.N.N` branch
    /// with each segment `>= 1`.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        if !is_valid_version_tree(&value) {
            return Err(IdError::VersionTree(value));
        }
        Ok(Self { value })
    }
}

impl FromStr for VersionTreeId {
    type Err = IdError;

    /// Parse a `VERSION_TREE_ID` — see [`VersionTreeId::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for VersionTreeId {
    type Error = IdError;

    /// Parse a `VERSION_TREE_ID` — see [`VersionTreeId::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for VersionTreeId {
    type Error = IdError;

    /// Parse a `VERSION_TREE_ID` — see [`VersionTreeId::new`].
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

/// The uniform violation for one named invariant of this class.
fn failed(invariant: &str) -> InvariantViolation {
    InvariantViolation::here(format!(
        "Invariant {invariant} failed on type VERSION_TREE_ID"
    ))
}

impl Validate for VersionTreeId {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.value.is_empty() {
            out.push(failed("Value_valid"));
            // Every other invariant reads a part of the value; with none to
            // read, `Value_valid` is the whole report.
            return;
        }
        let parts: Vec<&str> = self.value.split('.').collect();
        // Outside the production every declared invariant holds vacuously.
        if !matches!(parts.len(), 1 | 3) {
            out.push(failed("Value_lexical_form_valid"));
            return;
        }
        // `Trunk_version_valid` — trunk_version is an integer >= 1.
        if !is_positive_int(self.trunk_version()) {
            out.push(failed("Trunk_version_valid"));
        }
        // Each, when present, is an integer >= 1.
        if self.branch_number().is_some_and(|b| !is_positive_int(b)) {
            out.push(failed("Branch_number_valid"));
        }
        if self.branch_version().is_some_and(|v| !is_positive_int(v)) {
            out.push(failed("Branch_version_valid"));
        }
        // The other three hold by construction.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vtid(value: &str) -> Vec<InvariantViolation> {
        VersionTreeId {
            value: value.to_owned(),
        }
        .invariants()
    }

    #[test]
    fn valid_forms() {
        assert_eq!(vtid("1"), Vec::<InvariantViolation>::new());
        assert_eq!(vtid("42"), Vec::<InvariantViolation>::new());
        assert_eq!(vtid("1.2.3"), Vec::<InvariantViolation>::new());
    }

    #[test]
    fn empty_fails_value_valid_only() {
        let v = vtid("");
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Value_valid failed on type VERSION_TREE_ID"
        );
    }

    /// Every malformed value names the invariant it breaks.
    #[test]
    fn malformed_values_name_the_invariant_they_break() {
        for (bad, invariant) in [
            // Trunk_version_valid: `trunk_version.as_integer >= 1`.
            ("a", "Trunk_version_valid"),
            ("-1", "Trunk_version_valid"),
            ("0", "Trunk_version_valid"),
            ("0.1.1", "Trunk_version_valid"),
            // Branch_number_valid: the same rule on the second part.
            ("1.0.1", "Branch_number_valid"),
            ("1.-1.1", "Branch_number_valid"),
            ("1.a.1", "Branch_number_valid"),
            // Branch_version_valid: the same rule on the third part.
            ("1.1.-1", "Branch_version_valid"),
            ("1.2.a", "Branch_version_valid"),
            ("1.1.0", "Branch_version_valid"),
            // Our own lexical rule: not the `version_tree_id` production at
            // all, so every declared invariant would hold vacuously.
            ("1.2", "Value_lexical_form_valid"),
            ("1.2.3.4", "Value_lexical_form_valid"),
        ] {
            let v = vtid(bad);
            let expected = format!("Invariant {invariant} failed on type VERSION_TREE_ID");
            assert!(
                v.iter().any(|m| m.message == expected),
                "{bad:?} should report {invariant}, got {v:?}"
            );
        }
        // `1/2/2` has no `.` at all, so it is read as a one-part trunk that is
        // not an integer.
        assert!(
            vtid("1/2/2").iter().any(
                |m| m.message == "Invariant Trunk_version_valid failed on type VERSION_TREE_ID"
            ),
            "a non-numeric single part breaks Trunk_version_valid"
        );
        // `01` is legal: `Trunk_version_valid` bounds the value, not the spelling.
        for good in ["1", "42", "1.2.3", "10.9.8", "01", "1.02.3", "007"] {
            assert!(vtid(good).is_empty(), "{good:?} is a legal VERSION_TREE_ID");
        }
    }

    /// The three invariants that hold by construction are never reported.
    #[test]
    fn the_derived_invariants_hold_by_construction() {
        for value in ["1", "7", "1.1.1", "2.3.3"] {
            let id = VersionTreeId {
                value: value.to_owned(),
            };
            // Branch_validity: both Void or both present.
            assert_eq!(
                id.branch_number().is_some(),
                id.branch_version().is_some(),
                "Branch_validity on {value:?}"
            );
            // Is_branch_validity: `is_branch xor branch_number = Void`.
            assert_eq!(
                id.is_branch(),
                id.branch_number().is_some(),
                "Is_branch_validity on {value:?}"
            );
            // Is_first_validity: `not is_first xor trunk_version = "1"`.
            assert_eq!(
                id.is_first(),
                id.trunk_version() == "1",
                "Is_first_validity on {value:?}"
            );
        }
    }

    #[test]
    fn accessors_trunk() {
        let t = VersionTreeId {
            value: "1".to_owned(),
        };
        assert_eq!(t.trunk_version(), "1");
        assert!(!t.is_branch());
        assert!(t.is_first());
        assert_eq!(t.branch_number(), None);
        assert_eq!(t.branch_version(), None);
    }

    #[test]
    fn accessors_branch() {
        let b = VersionTreeId {
            value: "2.1.4".to_owned(),
        };
        assert_eq!(b.trunk_version(), "2");
        assert!(b.is_branch());
        assert!(!b.is_first());
        assert_eq!(b.branch_number(), Some("1"));
        assert_eq!(b.branch_version(), Some("4"));
    }

    #[test]
    fn from_str_strict() {
        assert_eq!("1".parse::<VersionTreeId>().unwrap().value, "1".to_owned());
        assert!("1.2.3".parse::<VersionTreeId>().is_ok());
        assert_eq!("".parse::<VersionTreeId>(), Err(IdError::Empty));
        assert!(matches!(
            "1.2".parse::<VersionTreeId>(),
            Err(IdError::VersionTree(_))
        ));
    }
}
