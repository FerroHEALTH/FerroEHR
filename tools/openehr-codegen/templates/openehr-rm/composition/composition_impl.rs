// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `COMPOSITION`.
//!
//! The class page
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.composition.composition.adoc`
//! §Invariants) declares five invariants; every one is enforced, each at the
//! layer its inputs live at:
//!
//! - `Is_archetype_root` + inherited `Archetype_node_id_valid` — here, via
//!   the generated `composition_core` (the machine-classified mechanical
//!   invariants).
//! - `Content_valid` (`content /= Void implies not content.is_empty`) — by
//!   construction: the field emits `Option<NonEmptyVec<CONTENT_ITEM>>`, so a
//!   present-but-empty list is unrepresentable and the strict readers refuse
//!   `[]` at parse.
//! - `Category_validity`, `Territory_valid`, `Language_valid` — terminology-
//!   bound, enforced in `validate::terminology` against the `openehr-term`
//!   bundle (they need the code sets, which this layer does not hold).
//!
//! Beyond the invariant table: `composer` presence is structural (a `1..1`
//! attribute emits a mandatory field), and content typing is structural (the
//! `ContentItem` enum admits only the CONTENT_ITEM family).
//!
//! NOTE: the released text declares NO persistent-category context rule —
//! SPECRM-52 removed the old "no context on persistent Compositions"
//! invariant (`RM/docs/ehr/master05-composition_package.adoc` §The
//! Composition Package NOTE: relaxed after release 1.0.3).

use crate::v1_2::composition::composition::Composition;
use openehr_base::validate::{InvariantViolation, Validate};

impl Composition {
    /// The `composition_category` code for persistent content.
    ///
    /// Spec: `composition.adoc` §Functions gives the code NUMERICALLY —
    /// "True if category is `431|persistent|`" — so `431` is normative here,
    /// not a local convention. The rubric is a rendering of it and is resolved
    /// from the terminology bundle, never compared against.
    const PERSISTENT_CATEGORY: &str = "431";

    /// Returns `true` when this composition's category is `431|persistent|`.
    ///
    /// Spec: `composition.adoc` §Functions — "True if category is
    /// `431|persistent|`, False otherwise. Useful for finding Compositions in
    /// an EHR which are guaranteed to be of interest to most users."
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.category.defining_code.code_string == Self::PERSISTENT_CATEGORY
    }
}

impl Validate for Composition {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::composition_core(
            self.archetype_details.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::archetyped::archetyped::Archetyped;
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::{ArchetypeId, TerminologyId};

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

    fn code(terminology: &str, code: &str) -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: terminology.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    fn category() -> DvCodedText {
        DvCodedText {
            value: "event".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: code("openehr", "433"),
        }
    }

    fn composition() -> Composition {
        Composition {
            name: text("Encounter"),
            archetype_node_id: "openEHR-EHR-COMPOSITION.encounter.v1".to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: Some(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-COMPOSITION.encounter.v1".to_owned(),
                },
                template_id: None,
                rm_version: "1.1.0".to_owned(),
            }),
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            territory: code("ISO_3166-1", "GB"),
            category: category(),
            context: None,
            composer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
            content: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }

    /// `composition.adoc` §Functions: `is_persistent` is true for `431` and
    /// FALSE OTHERWISE — asserted against a real other member of the same
    /// group (`433|event|`, the fixture's own category) rather than against an
    /// absent or nonsense code, since "not persistent" has to hold for the
    /// categories that actually occur.
    #[test]
    fn is_persistent_is_the_431_category_and_nothing_else() {
        let mut c = composition();
        assert!(!c.is_persistent(), "433|event| is not persistent");
        c.category = DvCodedText {
            defining_code: code("openehr", "431"),
            ..category()
        };
        assert!(c.is_persistent());
    }

    #[test]
    fn valid_composition() {
        assert!(composition().invariants().is_empty());
    }

    #[test]
    fn missing_archetype_details_invalid() {
        let mut c = composition();
        c.archetype_details = None;
        let v = c.invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Is_archetype_root failed on type COMPOSITION"),
            "got {v:?}"
        );
    }
}
