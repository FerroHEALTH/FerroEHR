// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The report of a broken class invariant of a draft package.
//!
//! The invariant checks of each generated draft module share it
//! ([`crate::draft::access_log::invariants`],
//! [`crate::draft::access_control::invariants`]).
//!
//! A draft is FerroEHR's change proposal and no openEHR release, so these
//! checks are our own design over our own model (#5124).

/// One broken invariant: the class and the invariant, as the BMM names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Violation {
    /// The BMM class whose invariant broke, such as `ACCESS_LOG_LINK`.
    pub class: &'static str,
    /// The invariant's BMM name, such as `Sequence_valid`.
    pub invariant: &'static str,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.class, self.invariant)
    }
}

/// The invariants an object of a draft package breaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the object breaks the draft invariants {}", list(.0))]
pub struct Violations(pub Vec<Violation>);

/// The violations, comma-separated.
fn list(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Collects the broken invariants.
#[derive(Default)]
pub(crate) struct Check(Vec<Violation>);

impl Check {
    /// Records `class.invariant` as broken unless it `holds`.
    pub(crate) fn hold(&mut self, holds: bool, class: &'static str, invariant: &'static str) {
        if !holds {
            self.0.push(Violation { class, invariant });
        }
    }

    /// Every broken invariant, or `Ok` when none broke.
    pub(crate) fn finish(self) -> Result<(), Violations> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Violations(self.0))
        }
    }
}
