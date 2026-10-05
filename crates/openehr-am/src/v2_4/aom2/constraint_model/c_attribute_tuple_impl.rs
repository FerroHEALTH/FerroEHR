// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_ATTRIBUTE_TUPLE` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_SECOND_ORDER.

use crate::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;

impl CAttributeTuple {
    /// Returns true if this tuple constraint is a subset of, or the same as,
    /// `other`.
    ///
    /// `c_conforms_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER):
    /// `for_all t: tuples | there_exists ot: other.tuples | t.c_conforms_to (ot,
    /// rmcc) or else tuples.count < other.tuples.count and for_all t: tuples |
    /// there_exists ot: other.tuples | t.c_congruent_to (ot)`.
    #[must_use]
    pub fn c_conforms_to(
        &self,
        other: &CAttributeTuple,
        rmcc: &dyn Fn(&str, &str) -> bool,
    ) -> bool {
        let own_rows = self.tuples.as_deref().unwrap_or_default();
        let other_rows = other.tuples.as_deref().unwrap_or_default();
        let every_row_conforms = own_rows.iter().all(|row| {
            other_rows
                .iter()
                .any(|theirs| row.c_conforms_to(theirs, rmcc))
        });
        every_row_conforms
            || (own_rows.len() < other_rows.len()
                && own_rows
                    .iter()
                    .all(|row| other_rows.iter().any(|theirs| row.c_congruent_to(theirs))))
    }

    /// Returns true if this tuple constraint adds nothing to `other`.
    ///
    /// `c_congruent_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER):
    /// `for_all t: tuples | there_exists ot: other.tuples | t.c_congruent_to
    /// (ot)`.
    ///
    /// NOTE: the BMM declares the parameter as `C_SECOND_ORDER` (the inherited
    /// signature) while `master04.5` states the body over `C_ATTRIBUTE_TUPLE`,
    /// and the docs text is the oracle.
    #[must_use]
    pub fn c_congruent_to(&self, other: &CAttributeTuple) -> bool {
        let other_rows = other.tuples.as_deref().unwrap_or_default();
        self.tuples
            .iter()
            .flatten()
            .all(|row| other_rows.iter().any(|theirs| row.c_congruent_to(theirs)))
    }
}
