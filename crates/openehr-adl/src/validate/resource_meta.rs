// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM resource-package invariants over an ADL 1.4 artefact's meta-data.
//!
//! The RM resource package governs AOM 1.4 archetype meta-data
//! (`docs/specs/openehr/RM/docs/common/master08-resource_package.adoc` — the
//! front-matter NOTE scopes it to ADL 1.4 artefacts), so a 1.4 SOURCE is
//! checked against the RM class invariants
//! (`docs/specs/openehr/RM/docs/UML/classes/
//! org.openehr.rm.common.authored_resource.adoc`,
//! `…resource_description.adoc`, `…resource_description_item.adoc`
//! §Invariants), the terminology-backed language rows included (the openEHR
//! `languages` code set, via the `openehr-term` bundle).

use openehr_am::v2_4::resource::resource_description::ResourceDescription;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use crate::artefact::ArchetypeView;

/// Appends the RM resource-meta violations of a 1.4 artefact to `issues`.
pub(super) fn check(v: &ArchetypeView<'_>, issues: &mut Vec<ValidationIssue>) {
    let original = v.original_language.map(|l| l.code_string.as_str());
    if let Some(original) = original
        && !openehr_term::bundle::openehr().is_valid_language(original)
    {
        issues.push(ValidationIssue::new(
            ValidationCode::RmArOriginalLanguage,
            format!("original language '{original}' is not in the openEHR languages code set"),
        ));
    }
    if let Some(translations) = v.translations {
        if translations.is_empty() {
            issues.push(ValidationIssue::new(
                ValidationCode::RmArTranslations,
                "the translations list is present but empty".to_owned(),
            ));
        }
        if let Some(original) = original
            && translations.keys().any(|language| language == original)
        {
            issues.push(ValidationIssue::new(
                ValidationCode::RmArTranslations,
                format!("a translation re-states the original language '{original}'"),
            ));
        }
        for translation in translations.values() {
            let language = translation.language.code_string.as_str();
            if !openehr_term::bundle::openehr().is_valid_language(language) {
                issues.push(ValidationIssue::new(
                    ValidationCode::RmTdLanguage,
                    format!(
                        "translation language '{language}' is not in the openEHR languages \
                         code set"
                    ),
                ));
            }
        }
    }
    if let Some(description) = v.description {
        check_description(v, description, original, issues);
    }
}

/// The description's own rows plus each detail item's.
fn check_description(
    v: &ArchetypeView<'_>,
    description: &ResourceDescription,
    original: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if description.original_author.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::RmRdOriginalAuthor,
            "the description's original_author is empty".to_owned(),
        ));
    }
    if description.lifecycle_state.is_empty() {
        issues.push(ValidationIssue::new(
            ValidationCode::RmRdLifecycleState,
            "the description's lifecycle_state is empty".to_owned(),
        ));
    }
    let details = description.details.as_ref();
    if details.is_none_or(std::collections::BTreeMap::is_empty) {
        issues.push(ValidationIssue::new(
            ValidationCode::RmRdDetails,
            "the description carries no details".to_owned(),
        ));
    }
    for (language, item) in details.into_iter().flatten() {
        let item_language = item.language.code_string.as_str();
        if !openehr_term::bundle::openehr().is_valid_language(item_language) {
            issues.push(ValidationIssue::new(
                ValidationCode::RmRdiLanguage,
                format!(
                    "description detail language '{item_language}' is not in the openEHR \
                     languages code set"
                ),
            ));
        }
        if item.purpose.is_empty() {
            issues.push(ValidationIssue::new(
                ValidationCode::RmRdiPurpose,
                format!("the '{language}' description detail has an empty purpose"),
            ));
        }
        if item.use_.as_deref() == Some("") {
            issues.push(ValidationIssue::new(
                ValidationCode::RmRdiUse,
                format!("the '{language}' description detail has an empty use"),
            ));
        }
        if item.misuse.as_deref() == Some("") {
            issues.push(ValidationIssue::new(
                ValidationCode::RmRdiMisuse,
                format!("the '{language}' description detail has an empty misuse"),
            ));
        }
        // Description_valid, checked against the original language plus the
        // translations.
        // NOTE: authored_resource.adoc §Invariants Description_valid — the
        // literal would refuse the original language's own description item,
        // so membership includes the original.
        if let (Some(original), Some(translations)) = (original, v.translations)
            && !translations.is_empty()
            && language != original
            && !translations.contains_key(language)
        {
            issues.push(ValidationIssue::new(
                ValidationCode::RmArDescription,
                format!(
                    "the '{language}' description detail has no matching translation \
                     (original language '{original}')"
                ),
            ));
        }
    }
}
