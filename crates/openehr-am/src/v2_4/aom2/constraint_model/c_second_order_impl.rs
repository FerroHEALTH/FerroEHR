// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_SECOND_ORDER` spec functions.
//!
//! Spec source (vendored):
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance semantics: C_SECOND_ORDER.

use crate::v2_4::aom2::constraint_model::c_second_order::CSecondOrder;

impl CSecondOrder {
    /// Returns true if this node expresses the same or narrower constraints
    /// than `other`.
    ///
    /// `c_conforms_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER) is
    /// deferred there and effected on `C_ATTRIBUTE_TUPLE` and
    /// `C_PRIMITIVE_TUPLE`, so this dispatches to the effecting subtype; two
    /// different second-order kinds are not comparable.
    #[must_use]
    pub fn c_conforms_to(&self, other: &CSecondOrder, rmcc: &dyn Fn(&str, &str) -> bool) -> bool {
        match (self, other) {
            (Self::CAttributeTuple(own), Self::CAttributeTuple(theirs)) => {
                own.c_conforms_to(theirs, rmcc)
            }
            (Self::CPrimitiveTuple(own), Self::CPrimitiveTuple(theirs)) => {
                own.c_conforms_to(theirs, rmcc)
            }
            _ => false,
        }
    }

    /// Returns true if this node expresses no constraints beyond `other`'s.
    ///
    /// `c_congruent_to` (`master04.5` §Conformance semantics: C_SECOND_ORDER),
    /// deferred and dispatched exactly as
    /// [`CSecondOrder::c_conforms_to`].
    #[must_use]
    pub fn c_congruent_to(&self, other: &CSecondOrder) -> bool {
        match (self, other) {
            (Self::CAttributeTuple(own), Self::CAttributeTuple(theirs)) => {
                own.c_congruent_to(theirs)
            }
            (Self::CPrimitiveTuple(own), Self::CPrimitiveTuple(theirs)) => {
                own.c_congruent_to(theirs)
            }
            _ => false,
        }
    }
}
