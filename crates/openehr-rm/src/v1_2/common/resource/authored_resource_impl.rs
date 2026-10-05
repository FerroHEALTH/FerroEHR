// @generated-from-template templates/openehr-rm/common/resource/authored_resource_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `AUTHORED_RESOURCE`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.authored_resource.adoc`
//! §Invariants — `Revision_history_valid`
//! (`is_controlled xor revision_history = Void`), evaluated by the generated
//! core only when `is_controlled` is PRESENT: the attribute is `0..1` and an
//! `xor` against a Void operand is not evaluable, so an absent flag asserts
//! nothing (refusing there would invent a prohibition the released text does
//! not contain). `Current_revision_valid` constrains the DERIVED
//! `current_revision()` function (§Functions), not stored data — adjudicated
//! in the generated register. The terminology-backed
//! `Original_language_valid` stays with the terminology binding table;
//! `Translations_valid`/`Description_valid` are cross-member map rules over
//! `translations`, realized where a whole authored resource is ingested —
//! the OPT 1.4 template upload's resource-meta pass
//! (`app/ferroehr/src/validation/opt/resource.rs`) and the ADL 1.4 source
//! catalogue (`openehr-adl` `validate/resource_meta.rs`); their rows live in
//! the generated complex-invariant register. `Languages_available_valid`
//! (`languages_available.has (original_language)`) constrains the derived
//! `languages_available()` function, which builds its result from
//! `original_language` — so it holds by that function's own definition, the
//! same venue as `Current_revision_valid` (register-adjudicated).

use crate::v1_2::common::resource::authored_resource::AuthoredResource;
use openehr_base::validate::{InvariantViolation, Validate};

/// The `current_revision` of a resource that is not under change control.
///
/// Spec: `org.openehr.rm.common.authored_resource.adoc` §Functions
/// `current_revision` ("… if `is_controlled` else `(uncontrolled)`") and
/// §Invariants `Current_revision_valid`, which pins the literal:
/// `(current_revision /= Void and not is_controlled) implies
/// current_revision.is_equal ("(uncontrolled)")`.
pub const UNCONTROLLED_REVISION: &str = "(uncontrolled)";

impl AuthoredResource {
    /// Returns the most recent revision of this resource.
    ///
    /// Spec: `org.openehr.rm.common.authored_resource.adoc` §Functions
    /// `current_revision` — "Most recent revision in `revision_history` if
    /// `is_controlled` else `(uncontrolled)`", with
    /// `Post: Result = revision_history.most_recent_version`.
    ///
    /// `None` is the one state neither branch answers: a resource that
    /// declares itself controlled while carrying no revision history, which
    /// §Invariants `Revision_history_valid` (`is_controlled xor
    /// revision_history = Void`) forbids. Reporting it is how a caller sees
    /// that violation instead of receiving a revision the resource does not
    /// have.
    #[must_use]
    pub fn current_revision(&self) -> Option<&str> {
        if self.is_controlled == Some(true) {
            return self
                .revision_history
                .as_ref()
                .and_then(crate::v1_2::common::generic::revision_history::RevisionHistory::most_recent_version);
        }
        Some(UNCONTROLLED_REVISION)
    }

    /// Returns every language this resource is available in, original language
    /// first.
    ///
    /// Spec: `org.openehr.rm.common.authored_resource.adoc` §Functions
    /// `languages_available` — "Total list of languages available in this
    /// resource, derived from `original_language` and `translations`" — with
    /// §Invariants `Languages_available_valid` (`languages_available.has
    /// (original_language)`) making the original language's membership
    /// unconditional, which is why it leads the list.
    ///
    /// The remaining languages are the `translations` keys, which the same
    /// section says are keyed by language and never include the original
    /// (§Invariants `Translations_valid`: `not translations.has
    /// (orginal_language.code_string)`). A translation keyed by the original
    /// language anyway is not listed twice.
    #[must_use]
    pub fn languages_available(&self) -> Vec<String> {
        let original = self.original_language.code_string.as_str();
        let mut languages = vec![original.to_owned()];
        if let Some(translations) = self.translations.as_ref() {
            languages.extend(
                translations
                    .keys()
                    .filter(|language| language.as_str() != original)
                    .cloned(),
            );
        }
        languages
    }
}

impl Validate for AuthoredResource {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::authored_resource_core(
            self.is_controlled,
            self.revision_history.is_some(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::common::generic::revision_history::RevisionHistory;
    use crate::v1_2::common::generic::revision_history_item::RevisionHistoryItem;
    use crate::v1_2::common::resource::translation_details::TranslationDetails;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;

    fn resource(is_controlled: Option<bool>) -> AuthoredResource {
        AuthoredResource {
            original_language: CodePhrase {
                terminology_id: TerminologyId {
                    value: "ISO_639-1".to_owned(),
                },
                code_string: "en".to_owned(),
                preferred_term: None,
            },
            is_controlled,
            translations: None,
            description: None,
            revision_history: None,
        }
    }

    #[test]
    fn uncontrolled_without_history_passes() {
        assert!(resource(Some(false)).invariants().is_empty());
    }

    #[test]
    fn controlled_without_history_is_a_violation() {
        let v = resource(Some(true)).invariants();
        assert!(
            v.iter()
                .any(|m| m.message.contains("Revision_history_valid")),
            "got {v:?}"
        );
    }

    #[test]
    fn absent_is_controlled_asserts_nothing() {
        assert!(resource(None).invariants().is_empty());
    }

    /// A history whose most recent item carries `version_id`.
    fn history(version_id: &str) -> Option<RevisionHistory> {
        Some(RevisionHistory {
            items: openehr_base::containers::NonEmptyVec::of(RevisionHistoryItem {
                version_id: ObjectVersionId::new(version_id.to_owned()).ok()?,
                audits: openehr_base::containers::NonEmptyVec::of(AuditDetails::AuditDetails(
                    AuditDetailsData {
                        system_id: "ferroehr.local".to_owned(),
                        time_committed: DvDateTime {
                            normal_status: None,
                            normal_range: None,
                            other_reference_ranges: None,
                            magnitude_status: None,
                            accuracy: None,
                            value: "2026-07-07T10:11:12Z".to_owned(),
                        },
                        change_type: DvCodedText {
                            value: "creation".to_owned(),
                            hyperlink: None,
                            formatting: None,
                            mappings: None,
                            language: None,
                            encoding: None,
                            defining_code: CodePhrase {
                                terminology_id: TerminologyId {
                                    value: "openehr".to_owned(),
                                },
                                code_string: "249".to_owned(),
                                preferred_term: None,
                            },
                        },
                        description: None,
                        committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
                    },
                )),
            }),
        })
    }

    /// A translation entry keyed by `language`.
    fn translation(language: &str) -> TranslationDetails {
        TranslationDetails {
            language: CodePhrase {
                terminology_id: TerminologyId {
                    value: "ISO_639-1".to_owned(),
                },
                code_string: language.to_owned(),
                preferred_term: None,
            },
            author: std::collections::BTreeMap::new(),
            accreditaton: None,
            other_details: None,
        }
    }

    /// "… else `(uncontrolled)`" — an uncontrolled resource reports the
    /// literal `Current_revision_valid` pins, whatever else it carries.
    #[test]
    fn an_uncontrolled_resource_reports_the_uncontrolled_literal() {
        assert_eq!(
            resource(Some(false)).current_revision(),
            Some("(uncontrolled)")
        );
        assert_eq!(resource(None).current_revision(), Some("(uncontrolled)"));
    }

    /// `Post: Result = revision_history.most_recent_version` — a controlled
    /// resource reports its history's most recent version id.
    #[test]
    fn a_controlled_resource_reports_its_most_recent_revision() {
        let mut controlled = resource(Some(true));
        controlled.revision_history =
            history("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::3");
        assert_eq!(
            controlled.current_revision(),
            Some("8849182c-82ad-4088-a07f-48ead4180515::ferroehr.local::3")
        );
    }

    /// The state `Revision_history_valid` forbids — controlled with no
    /// history — has no revision to report, and says so.
    #[test]
    fn a_controlled_resource_without_history_reports_no_revision() {
        assert!(resource(Some(true)).current_revision().is_none());
    }

    /// `languages_available.has (original_language)` holds unconditionally,
    /// and the translations follow it.
    #[test]
    fn the_languages_are_the_original_plus_the_translations() {
        let plain = resource(Some(false));
        assert_eq!(plain.languages_available(), vec!["en".to_owned()]);

        let mut translated = resource(Some(false));
        let mut translations = std::collections::BTreeMap::new();
        translations.insert("de".to_owned(), translation("de"));
        translations.insert("nl".to_owned(), translation("nl"));
        translated.translations = Some(translations);
        assert_eq!(
            translated.languages_available(),
            vec!["en".to_owned(), "de".to_owned(), "nl".to_owned()]
        );
    }

    /// `Translations_valid` forbids a translation keyed by the original
    /// language; one present anyway is not reported twice.
    #[test]
    fn the_original_language_is_listed_once() {
        let mut resource = resource(Some(false));
        let mut translations = std::collections::BTreeMap::new();
        translations.insert("en".to_owned(), translation("en"));
        resource.translations = Some(translations);
        assert_eq!(resource.languages_available(), vec!["en".to_owned()]);
    }
}
