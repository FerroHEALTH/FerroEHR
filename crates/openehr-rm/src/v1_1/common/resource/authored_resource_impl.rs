// @generated-from-template templates/openehr-rm/overrides/v1_1/common/resource/authored_resource_impl.rs (per-generation override) — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `AUTHORED_RESOURCE` invariants (RM 1.2.0
//! `org.openehr.rm.common.authored_resource.adoc` §Invariants):
//! `Revision_history_valid` in the generated core, only when `is_controlled`
//! is present. The other rows live in the generated complex-invariant
//! register.
//!
//! NOTE: the `v1_1` override; RM 1.1.0 spells `TRANSLATION_DETAILS.accreditaton`
//! (`translation_details.adoc` §Attributes), so the fixtures differ.

use crate::v1_1::common::resource::authored_resource::AuthoredResource;
use openehr_base::validate::{InvariantViolation, Validate};

/// The `current_revision` of an uncontrolled resource
/// (`Current_revision_valid`).
pub const UNCONTROLLED_REVISION: &str = "(uncontrolled)";

impl AuthoredResource {
    /// Returns the most recent revision of this resource (`current_revision`).
    ///
    /// `None` for a controlled resource without a revision history, which
    /// `Revision_history_valid` forbids.
    #[must_use]
    pub fn current_revision(&self) -> Option<&str> {
        if self.is_controlled == Some(true) {
            return self
                .revision_history
                .as_ref()
                .and_then(crate::v1_1::common::generic::revision_history::RevisionHistory::most_recent_version);
        }
        Some(UNCONTROLLED_REVISION)
    }

    /// Returns every language of this resource, the original first, then the
    /// `translations` keys without repeats (`languages_available`).
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
        crate::v1_1::validate::generated::authored_resource_core(
            self.is_controlled,
            self.revision_history.is_some(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::common::generic::audit_details::{AuditDetails, AuditDetailsData};
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::common::generic::revision_history::RevisionHistory;
    use crate::v1_1::common::generic::revision_history_item::RevisionHistoryItem;
    use crate::v1_1::common::resource::translation_details::TranslationDetails;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_2::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_base::v1_2::base_types::identification::terminology_id::TerminologyId;
    use openehr_term::codes::AuditChangeType;

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
        assert_eq!(
            resource(Some(false)).invariants(),
            Vec::<InvariantViolation>::new()
        );
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
        assert_eq!(
            resource(None).invariants(),
            Vec::<InvariantViolation>::new()
        );
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
                            value: AuditChangeType::Creation.rubric().to_owned(),
                            hyperlink: None,
                            formatting: None,
                            mappings: None,
                            language: None,
                            encoding: None,
                            defining_code: CodePhrase {
                                terminology_id: TerminologyId {
                                    value: "openehr".to_owned(),
                                },
                                code_string: AuditChangeType::Creation.code().to_owned(),
                                preferred_term: None,
                            },
                        },
                        description: None,
                        committer: Box::new(PartyProxy::PartySelf(PartySelf {
                            external_ref: None,
                        })),
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
