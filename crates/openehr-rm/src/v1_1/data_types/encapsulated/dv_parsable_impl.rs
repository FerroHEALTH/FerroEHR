// @generated-from-template templates/openehr-rm/data_types/encapsulated/dv_parsable_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants + functions for `DV_PARSABLE`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_parsable.adoc`:
//! - `size()`: size in bytes of `value`.
//! - `Formalism_valid`: `not formalism.is_empty`.
//! - `Size_valid`: `size >= 0` — holds by construction for a Rust byte length
//!   (usize), so it is not a runnable check.

use crate::v1_1::data_types::encapsulated::dv_parsable::DvParsable;
use openehr_base::validate::{InvariantViolation, Validate};

impl DvParsable {
    /// RM `DV_PARSABLE.size()`: size in bytes of `value`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.value.len()
    }
}

impl Validate for DvParsable {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::dv_parsable_core(&self.formalism, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsable(value: &str, formalism: &str) -> DvParsable {
        DvParsable {
            charset: None,
            language: None,
            value: value.to_owned(),
            formalism: formalism.to_owned(),
        }
    }

    #[test]
    fn valid_parsable() {
        let p = parsable("<gdl/>", "GLIF 1.0");
        assert!(p.invariants().is_empty());
        assert_eq!(p.size(), 6);
    }

    #[test]
    fn empty_value_is_allowed_but_empty_formalism_is_not() {
        // "The string, which may validly be empty in some syntaxes."
        assert!(parsable("", "proforma").invariants().is_empty());
        let v = parsable("x", "").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Formalism_valid failed on type DV_PARSABLE"),
            "got {v:?}"
        );
    }
}
