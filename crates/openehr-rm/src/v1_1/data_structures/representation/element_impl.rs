// @generated-from-template templates/openehr-rm/data_structures/representation/element_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `ELEMENT`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`
//! §Functions + §Invariants. Enforced here:
//! - `Inv_null_flavour_indicated` (`is_null() xor null_flavour = Void`):
//!   exactly one of `value` / `null_flavour` is present.
//! - `Inv_null_reason_valid` (`null_reason /= Void implies is_null()`): a
//!   `null_reason` implies `value` is absent.
//! - the inherited LOCATABLE `Archetype_node_id_valid`
//!   (`…org.openehr.rm.common.locatable.adoc` §Invariants).
//!
//! `Inv_is_null_valid` (`is_null() = (value = Void)`) is unfalsifiable here:
//! that clause IS the definition of [`Element::is_null`], which this crate
//! computes from `value` rather than storing. `Inv_null_flavour_valid` is
//! group-bound and is enforced in the terminology-aware path
//! (`validate::terminology`, the `ELEMENT` slot) against the `openehr-term`
//! bundle, which this crate does not depend on.

use crate::v1_1::data_structures::representation::element::Element;
use openehr_base::validate::{InvariantViolation, Validate};

/// The ELEMENT invariant core over the projected presence flags — one source
/// for the typed impl and the value-level fast path (`validate::fast`).
pub(crate) fn push_element_invariants(
    has_value: bool,
    has_null_flavour: bool,
    has_null_reason: bool,
    archetype_node_id: &str,
    out: &mut Vec<InvariantViolation>,
) {
    // Inv_null_flavour_indicated: `is_null() xor null_flavour = Void`, i.e.
    // exactly one of value / null_flavour is present.
    if has_value == has_null_flavour {
        out.push(InvariantViolation::here(
            "Invariant Inv_null_flavour_indicated failed on type ELEMENT",
        ));
    }
    // Inv_null_reason_valid: a null reason only applies when value is absent.
    if has_null_reason && has_value {
        out.push(InvariantViolation::here(
            "Invariant Inv_null_reason_valid failed on type ELEMENT",
        ));
    }
    crate::v1_1::validate::generated::archetype_node_id_core("ELEMENT", archetype_node_id, out);
}

impl Element {
    /// Returns `true` when this element's value is logically not known.
    ///
    /// RM `ELEMENT.is_null()` (`element.adoc` §Functions), computed from
    /// `value` as its `Inv_is_null_valid` invariant requires
    /// (`is_null() = (value = Void)`).
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

impl Validate for Element {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        push_element_invariants(
            self.value.is_some(),
            self.null_flavour.is_some(),
            self.null_reason.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::basic::data_value::DataValue;
    use crate::v1_1::data_types::basic::dv_boolean::DvBoolean;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::TerminologyId;

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
        })
    }

    fn null_flavour() -> DvCodedText {
        DvCodedText {
            value: "unknown".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: "253".to_owned(),
                preferred_term: None,
            },
        }
    }

    fn element() -> Element {
        Element {
            name: text("element"),
            archetype_node_id: "at0001".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(DataValue::DvBoolean(DvBoolean { value: true })),
            null_reason: None,
        }
    }

    #[test]
    fn value_only_valid() {
        assert!(element().invariants().is_empty());
    }

    #[test]
    fn null_flavour_only_valid() {
        let mut e = element();
        e.value = None;
        e.null_flavour = Some(null_flavour());
        assert!(e.invariants().is_empty());
    }

    #[test]
    fn both_value_and_null_flavour_invalid() {
        let mut e = element();
        e.null_flavour = Some(null_flavour());
        let v = e.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Inv_null_flavour_indicated failed on type ELEMENT"),
            "got {v:?}"
        );
    }

    #[test]
    fn null_reason_with_value_invalid() {
        let mut e = element();
        e.null_reason = Some(text("some reason"));
        let v = e.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Inv_null_reason_valid failed on type ELEMENT"),
            "got {v:?}"
        );
    }

    /// `element.adoc` §Invariants `Inv_is_null_valid`: `is_null()` is exactly
    /// value-absence, so a valued element is not null.
    #[test]
    fn is_null_is_false_when_a_value_is_present() {
        assert!(!element().is_null());
    }

    /// `element.adoc` §Invariants `Inv_is_null_valid`, refusing twin: an element
    /// with no value IS null, whether or not a null flavour accompanies it.
    #[test]
    fn is_null_is_true_when_the_value_is_absent() {
        let mut e = element();
        e.value = None;
        assert!(e.is_null());
        e.null_flavour = Some(null_flavour());
        assert!(e.is_null());
    }

    #[test]
    fn empty_node_id_invalid() {
        let mut e = element();
        e.archetype_node_id = String::new();
        let v = e.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Archetype_node_id_valid failed on type ELEMENT")
        );
    }
}
