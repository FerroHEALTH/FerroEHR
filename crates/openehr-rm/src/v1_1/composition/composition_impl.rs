// @generated-from-template templates/openehr-rm/composition/composition_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `COMPOSITION` invariants.
//!
//! `org.openehr.rm.composition.composition.adoc` §Invariants: `Is_archetype_root`
//! and `Archetype_node_id_valid` in `composition_core`; `Content_valid` by the
//! `NonEmptyVec` type; `Category_validity`, `Territory_valid` and
//! `Language_valid` in `validate::terminology`.

use crate::v1_1::composition::composition::Composition;
use openehr_base::validate::{InvariantViolation, Validate};

impl Composition {
    /// The `composition_category` code for persistent content, "`431|persistent|`"
    /// (`composition.adoc` §Functions).
    const PERSISTENT_CATEGORY: &str = "431";

    /// Whether this composition's category is `431|persistent|`.
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.category.defining_code.code_string == Self::PERSISTENT_CATEGORY
    }
}

impl Validate for Composition {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::composition_core(
            self.archetype_details.is_some(),
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::archetyped::archetyped::Archetyped;
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::{ArchetypeId, TerminologyId};

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
            archetype_details: Some(Box::new(Archetyped {
                archetype_id: ArchetypeId {
                    value: "openEHR-EHR-COMPOSITION.encounter.v1".to_owned(),
                },
                template_id: None,
                rm_version: "1.1.0".to_owned(),
            })),
            feeder_audit: None,
            language: code("ISO_639-1", "en"),
            territory: code("ISO_3166-1", "GB"),
            category: category(),
            context: None,
            composer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            content: openehr_base::containers::present_nonempty(Vec::new()),
        }
    }

    /// `is_persistent` is false for `433|event|`.
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
        assert_eq!(composition().invariants(), Vec::<InvariantViolation>::new());
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
