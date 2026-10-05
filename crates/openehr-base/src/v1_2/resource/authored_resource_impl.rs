// @generated-from-template templates/openehr-base/resource/authored_resource_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written spec functions + invariants of `AUTHORED_RESOURCE`.
//!
//! Spec: `BASE/docs/UML/classes/org.openehr.base.resource.authored_resource.adoc`
//! §Functions and §Invariants, and
//! `BASE/docs/resource/master02-resource_package.adoc` §Natural Languages and
//! Translation ("The languages_available function provides a complete list of
//! languages in the resource").
//!
//! Invariants — the FOUR entries the class table declares:
//! - `Translations_valid` and `Description_valid` — checked, each under its
//!   own name.
//! - `Languages_available_valid` (`languages_available.has
//!   (original_language)`) — structurally satisfied: `languages_available`
//!   opens with the original language.
//! - `Original_language_valid` — a terminology boundary, see the NOTE below.
//!
//! NOTE: `Original_language_valid`
//! (`code_set (Code_set_id_languages).has_code (original_language.as_string)`)
//! is not enforced here: the languages code set lives in the terminology
//! service, which `openehr-base` has no dependency on.
//!
//! NOTE: `Description_valid`'s literal form would reject the resource's own
//! ORIGINAL-language detail, which `master02-resource_package.adoc` §Meta-data
//! requires each translated resource to keep, so it is read as "every detail
//! language is the original language or a declared translation".
//!
//! NOTE: `Translations_valid` spells its second clause `translations.has
//! (original_language.code_string)` over a list "keyed by language code" whose
//! §Attributes text says the original language "does not appear in this list",
//! so it is read as the KEY test its sibling clause spells `has_key`.

use crate::v1_2::resource::authored_resource::AuthoredResource;
use crate::v1_2::resource::resource_description_item::ResourceDescriptionItem;
use crate::v1_2::resource::translation_details::TranslationDetails;
use crate::validate::{InvariantViolation, Validate};
use std::collections::BTreeMap;

impl AuthoredResource {
    /// `AUTHORED_RESOURCE.languages_available`: the total list of languages
    /// available in this resource, derived from `original_language` and
    /// `translations` (class doc §Functions). The original language is always
    /// a member (invariant `Languages_available_valid`:
    /// `languages_available.has (original_language)`), followed by the
    /// translation language codes in their stored (key) order.
    #[must_use]
    pub fn languages_available(&self) -> Vec<&str> {
        let mut out = vec![self.original_language.code_string.as_str()];
        if let Some(translations) = &self.translations {
            out.extend(translations.keys().map(String::as_str));
        }
        out
    }
}

/// The uniform violation for one named invariant of this class.
fn failed(invariant: &str) -> InvariantViolation {
    InvariantViolation::here(format!(
        "Invariant {invariant} failed on type AUTHORED_RESOURCE"
    ))
}

/// Appends `Description_valid` when any description detail is written in a
/// language the resource declares neither as its original nor as a translation.
fn push_description_violation(
    details: &BTreeMap<String, ResourceDescriptionItem>,
    original_language: &str,
    translations: &BTreeMap<String, TranslationDetails>,
    out: &mut Vec<InvariantViolation>,
) {
    let undeclared = details.values().any(|item| {
        let language = item.language.code_string.as_str();
        language != original_language && !translations.contains_key(language)
    });
    if undeclared {
        out.push(failed("Description_valid"));
    }
}

impl Validate for AuthoredResource {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Both declared rules are `translations /= Void implies …`, so a
        // resource with no translation list satisfies each vacuously.
        let Some(translations) = &self.translations else {
            return;
        };
        let original_language = self.original_language.code_string.as_str();
        if translations.is_empty() || translations.contains_key(original_language) {
            out.push(failed("Translations_valid"));
        }
        if let Some(details) = self.description.as_ref().and_then(|d| d.details.as_ref()) {
            push_description_violation(details, original_language, translations, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::foundation_types::terminology::terminology_code::TerminologyCode;

    /// An ISO 639-1 language code as a `TERMINOLOGY_CODE`.
    fn language_code(code: &str) -> TerminologyCode {
        TerminologyCode {
            terminology_id: "ISO_639-1".to_owned(),
            terminology_version: None,
            code_string: code.to_owned(),
            uri: None,
        }
    }

    /// A `translations` list keyed by the given language codes.
    fn translation_map(languages: &[&str]) -> BTreeMap<String, TranslationDetails> {
        languages
            .iter()
            .map(|lang| {
                (
                    (*lang).to_owned(),
                    TranslationDetails {
                        language: language_code(lang),
                        author: BTreeMap::new(),
                        accreditation: None,
                        other_details: None,
                        version_last_translated: None,
                        other_contributors: None,
                    },
                )
            })
            .collect()
    }

    /// A `description.details` list keyed by the given language codes.
    fn details_map(languages: &[&str]) -> BTreeMap<String, ResourceDescriptionItem> {
        languages
            .iter()
            .map(|lang| {
                (
                    (*lang).to_owned(),
                    ResourceDescriptionItem {
                        language: language_code(lang),
                        purpose: "purpose".to_owned(),
                        keywords: None,
                        use_: None,
                        misuse: None,
                        original_resource_uri: None,
                        other_details: None,
                    },
                )
            })
            .collect()
    }

    fn resource(translations: &[&str]) -> AuthoredResource {
        AuthoredResource {
            uid: None,
            original_language: language_code("en"),
            description: None,
            is_controlled: None,
            annotations: None,
            translations: if translations.is_empty() {
                None
            } else {
                Some(translation_map(translations))
            },
        }
    }

    #[test]
    fn original_language_only() {
        // Languages_available_valid: the original language is always a member.
        assert_eq!(resource(&[]).languages_available(), ["en"]);
    }

    #[test]
    fn original_plus_translations() {
        assert_eq!(
            resource(&["de", "pt"]).languages_available(),
            ["en", "de", "pt"]
        );
    }

    // ── invariants ───────────────────────────────────────────────────────────

    /// `Translations_valid`: a present list is neither empty nor a re-statement
    /// of the original language.
    #[test]
    fn translations_valid_refuses_an_empty_or_self_naming_list() {
        for (languages, case) in [
            (Vec::new(), "an empty list"),
            (vec!["en"], "the original language alone"),
            (vec!["de", "en"], "the original language among others"),
        ] {
            let mut authored = resource(&[]);
            authored.translations = Some(translation_map(&languages));
            let violations = authored.invariants();
            assert_eq!(violations.len(), 1, "{case}");
            assert_eq!(
                violations[0].message,
                "Invariant Translations_valid failed on type AUTHORED_RESOURCE",
                "{case}"
            );
        }
    }

    /// The state `Translations_valid` exists to prevent: `languages_available`
    /// reports the original language twice.
    #[test]
    fn a_self_naming_translation_list_duplicates_a_language() {
        let mut authored = resource(&[]);
        authored.translations = Some(translation_map(&["en"]));
        assert_eq!(authored.languages_available(), ["en", "en"]);
        assert!(!authored.invariants().is_empty());
    }

    #[test]
    fn a_valid_translation_list_reports_nothing() {
        // No list at all: both rules are vacuous.
        assert!(resource(&[]).invariants().is_empty());
        assert!(resource(&["de", "pt"]).invariants().is_empty());
    }

    /// `Description_valid`: a detail written in a language the resource
    /// declares nowhere.
    #[test]
    fn description_valid_refuses_a_detail_in_an_undeclared_language() {
        // NOTE: BASE 1.2.0's RESOURCE_DESCRIPTION declares a different field set
        // from 1.3.0's, so a whole-value fixture is not writable in a generation
        // twin — the rule is asserted on the core every generation shares.
        let mut violations = Vec::new();
        push_description_violation(
            &details_map(&["en", "fr"]),
            "en",
            &translation_map(&["de"]),
            &mut violations,
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].message,
            "Invariant Description_valid failed on type AUTHORED_RESOURCE"
        );
    }

    /// The accepting twin, including the ORIGINAL-language detail the literal
    /// invariant text would have rejected (module NOTE).
    #[test]
    fn description_valid_accepts_the_original_language_and_every_translation() {
        for details in [
            details_map(&["en"]),
            details_map(&["en", "de"]),
            details_map(&["en", "de", "pt"]),
            details_map(&[]),
        ] {
            let mut violations = Vec::new();
            push_description_violation(
                &details,
                "en",
                &translation_map(&["de", "pt"]),
                &mut violations,
            );
            assert!(
                violations.is_empty(),
                "{details:?} declares only known languages"
            );
        }
    }
}
