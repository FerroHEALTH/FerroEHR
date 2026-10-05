// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_PRIMITIVE_TUPLE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_SECOND_ORDER.

use crate::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;

impl CPrimitiveTuple {
    /// Returns true if this tuple row is a subset of, or the same as, `other`.
    ///
    /// `c_conforms_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER):
    /// equal member counts and, position-wise, `same_type` plus the member's own
    /// `c_conforms_to`. Members are `C_PRIMITIVE_OBJECT`s under a tuple rather
    /// than under a `C_ATTRIBUTE`, so the precursor's occurrences clause is
    /// evaluated as for a root node.
    #[must_use]
    pub fn c_conforms_to(
        &self,
        other: &CPrimitiveTuple,
        rmcc: &dyn Fn(&str, &str) -> bool,
    ) -> bool {
        self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(other.members.iter())
                .all(|(own, theirs)| {
                    own.constrained_typename()
                        .eq_ignore_ascii_case(theirs.constrained_typename())
                        && own.c_conforms_to(theirs, rmcc, None)
                })
    }

    /// Returns true if this tuple row and `other` are semantically the same.
    ///
    /// `c_congruent_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER):
    /// equal member counts and, position-wise, `same_type` plus the member's own
    /// `c_congruent_to`.
    ///
    /// NOTE: the BMM declares the parameter as `C_SECOND_ORDER` (the inherited
    /// signature) while `master04.5` states the body over `C_PRIMITIVE_TUPLE`,
    /// and the docs text is the oracle.
    #[must_use]
    pub fn c_congruent_to(&self, other: &CPrimitiveTuple) -> bool {
        self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(other.members.iter())
                .all(|(own, theirs)| own.c_congruent_to(theirs))
    }
}
