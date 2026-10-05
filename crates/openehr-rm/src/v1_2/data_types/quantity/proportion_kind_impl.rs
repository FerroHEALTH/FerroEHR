// @generated-from-template templates/openehr-rm/data_types/quantity/proportion_kind_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `PROPORTION_KIND`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.proportion_kind.adoc`
//! §Constants + §Functions.

use crate::v1_2::data_types::quantity::proportion_kind::ProportionKind;

impl ProportionKind {
    /// Returns `true` when `nq` is one of the defined proportion kinds.
    ///
    /// Spec: `org.openehr.rm.data_types.proportion_kind.adoc` §Functions
    /// `valid_proportion_kind` — "True if n is one of the defined types", over
    /// the §Constants `pk_ratio` = 0 … `pk_integer_fraction` = 4.
    ///
    /// This forwards to the definition the `DV_PROPORTION` invariants already
    /// evaluate (`crate::v1_2::validate::valid_proportion_kind`), so the class
    /// function and the invariant cannot answer differently for the same
    /// value.
    #[must_use]
    pub fn valid_proportion_kind(nq: i32) -> bool {
        crate::v1_2::validate::valid_proportion_kind(nq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every §Constants value is valid, at both ends of the defined range.
    #[test]
    fn the_five_defined_kinds_are_valid() {
        for nq in 0..=4 {
            assert!(
                ProportionKind::valid_proportion_kind(nq),
                "{nq} is a defined PROPORTION_KIND"
            );
        }
    }

    /// The boundaries either side of the constant set, and a negative value,
    /// are not defined types.
    #[test]
    fn anything_outside_the_constant_set_is_invalid() {
        assert!(!ProportionKind::valid_proportion_kind(-1));
        assert!(!ProportionKind::valid_proportion_kind(5));
        assert!(!ProportionKind::valid_proportion_kind(i32::MAX));
    }

    /// The validity predicate and the strict decode partition the same
    /// values: one cannot admit a kind the other refuses.
    #[test]
    fn validity_agrees_with_the_strict_decode() {
        for nq in -3..8 {
            assert_eq!(
                ProportionKind::valid_proportion_kind(nq),
                ProportionKind::try_from(i64::from(nq)).is_ok()
            );
        }
    }
}
