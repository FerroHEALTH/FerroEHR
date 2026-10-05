// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! RM/BASE class-invariant contract (hand-written spec behaviour).
//!
//! Hand-written; preserved across `openehr-codegen` regeneration (it is not a
//! `// @generated` file, so `write_crate` keeps it and `lib.rs` auto-declares
//! `pub mod validate;`). Concrete invariant bodies live in sibling `*_impl.rs`
//! files next to each generated type and `impl Validate for <Type>`.
//!
//! The invariant SET is the specification's own: every check corresponds to a
//! named invariant in an RM/BASE class page's §Invariants section
//! (`docs/specs/openehr/{RM,BASE}/docs/UML/classes/`), and each impl cites the
//! page and invariant name it enforces. What the specifications leave
//! underdetermined is the *algorithm* — openEHR's AOM2 `validation` spec covers
//! ARCHETYPE validation, not RM-instance validation — so the traversal,
//! violation shape and sub-path reporting below are our own design; a NOTE
//! comment in the impls flags each such choice.

/// An RM class-invariant violation: a human-readable message plus the RM
/// sub-path (relative to the value being checked) it applies to.
///
/// An empty path means the value itself; the composition validator prefixes
/// the absolute RM path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantViolation {
    /// The RM sub-path the violation applies to, relative to the value being
    /// checked (empty = the value itself).
    pub path: String,
    /// The human-readable violation message (the uniform
    /// `Invariant <Name> failed on type <RM_TYPE>` form for invariant cores —
    /// `<Name>` is the released class table's own invariant name).
    pub message: String,
}

impl InvariantViolation {
    /// A violation on the value itself (no sub-path).
    #[must_use]
    pub fn here(message: impl Into<String>) -> Self {
        Self {
            path: String::new(),
            message: message.into(),
        }
    }

    /// A violation on a named sub-attribute of the value.
    #[must_use]
    pub fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// The RM class invariants of a spec type. Implemented in `*_impl.rs` siblings.
pub trait Validate {
    /// Append this value's class-invariant violations to `out`. A value with no
    /// violations appends nothing. Implementations check only their own class
    /// invariants (not archetype constraints); the caller recurses into children.
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>);

    /// Convenience: collect this value's own invariant violations.
    #[must_use]
    fn invariants(&self) -> Vec<InvariantViolation> {
        let mut out = Vec::new();
        self.validate_invariants(&mut out);
        out
    }
}
