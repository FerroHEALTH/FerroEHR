// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM resource-package invariants over the uploaded OPT's own meta-data.
//!
//! The OPT root is `AUTHORED_RESOURCE`-shaped — `Template.xsd` declares
//! `language`/`is_controlled`/`description`/`revision_history` and says the
//! operational template "will inherit" from `AUTHORED_RESOURCE` — and an
//! embedded `ARCHETYPE` reached through `description.parent_resource` carries
//! the full header including `translations`. The rules are the RM class
//! invariants (`docs/specs/openehr/RM/docs/UML/classes/
//! org.openehr.rm.common.authored_resource.adoc`, `…translation_details.adoc`,
//! `…resource_description.adoc`, `…resource_description_item.adoc`
//! §Invariants), applied to the values the OPT 1.4 wire can express: the root
//! declares no `translations` element, so `Translations_valid` and
//! `Description_valid` (antecedent `translations /= Void`) bind only on an
//! embedded `ARCHETYPE`.

use openehr_its::opt14::types::{
    Archetype, AuthoredResource, OperationalTemplate, ResourceDescription, TranslationDetails,
};
use openehr_term::bundle::{OpenehrTerminology, openehr};

use super::RuleViolation;

/// Validates the RM resource-package invariants of the OPT's meta-data
/// header, reporting the first violation found (the module's contract).
pub(super) fn check_resource_meta(opt: &OperationalTemplate) -> Vec<RuleViolation> {
    let terminology = openehr();
    let mut violations = Vec::new();
    if let Err(v) = check_language(
        terminology,
        &opt.language.code_string,
        "AUTHORED_RESOURCE.Original_language_valid",
        "the operational template's language",
    ) {
        violations.push(v);
    }
    if let Err(v) = check_revision_history_flag(
        opt.is_controlled,
        opt.revision_history.is_some(),
        "the operational template",
    ) {
        violations.push(v);
    }
    if let Some(description) = opt.description.as_ref() {
        violations.extend(check_description(
            terminology,
            description,
            "the operational template",
        ));
    }
    violations
}

/// One embedded `ARCHETYPE` (an `AUTHORED_RESOURCE` subtype): the full
/// header, including the translation rules the OPT root cannot express.
fn check_archetype(
    terminology: &OpenehrTerminology,
    archetype: &Archetype,
) -> Result<(), RuleViolation> {
    let original = archetype.original_language.code_string.as_str();
    check_language(
        terminology,
        original,
        "AUTHORED_RESOURCE.Original_language_valid",
        &format!("embedded archetype '{}'", archetype.archetype_id.value),
    )?;
    check_revision_history_flag(
        archetype.is_controlled,
        archetype.revision_history.is_some(),
        &format!("embedded archetype '{}'", archetype.archetype_id.value),
    )?;
    check_translations(terminology, &archetype.translations, original)?;
    if let Some(description) = archetype.description.as_ref() {
        if let Some(v) = check_description(
            terminology,
            description,
            &format!("embedded archetype '{}'", archetype.archetype_id.value),
        )
        .into_iter()
        .next()
        {
            return Err(v);
        }
        // Description_valid: `translations /= Void implies (description.details
        // .for_all (d | translations.has_key (d.language.code_string)))`.
        // NOTE: authored_resource.adoc §Invariants Description_valid — the
        // literal would refuse the original language's own description item,
        // so membership is checked against the original plus the translations.
        if !archetype.translations.is_empty() {
            for item in &description.details {
                let language = item.language.code_string.as_str();
                let translated = archetype
                    .translations
                    .iter()
                    .any(|t| t.language.code_string == language);
                if language != original && !translated {
                    return Err(RuleViolation::new(
                        "AUTHORED_RESOURCE.Description_valid",
                        format!(
                            "description detail language '{language}' of embedded archetype \
                             '{}' has no matching translation (original language '{original}')",
                            archetype.archetype_id.value
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The `translations` rules of an embedded `ARCHETYPE`.
///
/// `Translations_valid`: a translation never re-states the original language
/// (`not translations.has (orginal_language.code_string)` — the spec's own
/// spelling); each entry's `language` is bound to the `languages` code set
/// (`translation_details.adoc` §Invariants `Language_valid`); and the wire's
/// repeated elements realize a `Hash<String, TRANSLATION_DETAILS>` keyed by
/// language, so a duplicate key is unrepresentable in the model and refused.
fn check_translations(
    terminology: &OpenehrTerminology,
    translations: &[TranslationDetails],
    original: &str,
) -> Result<(), RuleViolation> {
    let mut seen: Vec<&str> = Vec::with_capacity(translations.len());
    for translation in translations {
        let language = translation.language.code_string.as_str();
        check_language(
            terminology,
            language,
            "TRANSLATION_DETAILS.Language_valid",
            "a translation",
        )?;
        if language == original {
            return Err(RuleViolation::new(
                "AUTHORED_RESOURCE.Translations_valid",
                format!("a translation re-states the original language '{original}'"),
            ));
        }
        if seen.contains(&language) {
            return Err(RuleViolation::new(
                "AUTHORED_RESOURCE.Translations_valid",
                format!(
                    "two translations carry the same language '{language}' — translations \
                     are keyed by language"
                ),
            ));
        }
        seen.push(language);
    }
    Ok(())
}

/// One `RESOURCE_DESCRIPTION` (and, recursively, its `parent_resource`
/// chain): the class's own invariants plus each detail item's.
fn check_description(
    terminology: &OpenehrTerminology,
    description: &ResourceDescription,
    owner: &str,
) -> Vec<RuleViolation> {
    // The three header rules below decide whether there IS a description to
    // examine, so each one returns alone: with no author, no lifecycle state or
    // no details at all, the per-detail rules have nothing to say.
    if description.original_author.is_empty() {
        return vec![RuleViolation::new(
            "RESOURCE_DESCRIPTION.Original_author_valid",
            format!("the description of {owner} has an empty original_author"),
        )];
    }
    if description.lifecycle_state.is_empty() {
        return vec![RuleViolation::new(
            "RESOURCE_DESCRIPTION.Lifecycle_state_valid",
            format!("the description of {owner} has an empty lifecycle_state"),
        )];
    }
    if description.details.is_empty() {
        return vec![RuleViolation::new(
            "RESOURCE_DESCRIPTION.Details_valid",
            format!("the description of {owner} has no details"),
        )];
    }
    // Every detail's own rules are independent properties of that detail, so
    // they accumulate: a hand-written template with an empty `use` AND an empty
    // `misuse` reports both at once rather than one per upload (#3129).
    let mut violations = Vec::new();
    let mut seen: Vec<&str> = Vec::with_capacity(description.details.len());
    for item in &description.details {
        let language = item.language.code_string.as_str();
        if let Err(v) = check_language(
            terminology,
            language,
            "RESOURCE_DESCRIPTION_ITEM.Language_valid",
            "a description detail",
        ) {
            violations.push(v);
        }
        if seen.contains(&language) {
            violations.push(RuleViolation::new(
                "RESOURCE_DESCRIPTION.Details_valid",
                format!(
                    "two description details carry the same language '{language}' — details \
                     are keyed by language"
                ),
            ));
        }
        seen.push(language);
        if item.purpose.is_empty() {
            violations.push(RuleViolation::new(
                "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
                format!("the '{language}' description detail of {owner} has an empty purpose"),
            ));
        }
        for (value, code, field) in [
            (
                item.use_.as_deref(),
                "RESOURCE_DESCRIPTION_ITEM.Use_valid",
                "use",
            ),
            (
                item.misuse.as_deref(),
                "RESOURCE_DESCRIPTION_ITEM.misuse_valid",
                "misuse",
            ),
            (
                item.copyright.as_deref(),
                "RESOURCE_DESCRIPTION_ITEM.copyright_valid",
                "copyright",
            ),
        ] {
            if let Err(v) = check_present_non_empty(value, code, language, field, owner) {
                violations.push(v);
            }
        }
    }
    if let Some(parent) = description.parent_resource.as_deref() {
        let AuthoredResource::Archetype(archetype) = parent;
        if let Err(v) = check_archetype(terminology, archetype) {
            violations.push(v);
        }
    }
    violations
}

/// The `x /= Void implies not x.is_empty` rows of a description item.
fn check_present_non_empty(
    value: Option<&str>,
    code: &'static str,
    language: &str,
    field: &str,
    owner: &str,
) -> Result<(), RuleViolation> {
    if value == Some("") {
        return Err(RuleViolation::new(
            code,
            format!("the '{language}' description detail of {owner} has an empty {field}"),
        ));
    }
    Ok(())
}

/// `Revision_history_valid`: `is_controlled xor revision_history = Void`.
///
/// NOTE: `authored_resource.adoc` §Invariants — evaluated only when
/// `is_controlled` (0..1) is present; an xor against a Void operand is not
/// evaluable, so an absent flag asserts nothing.
fn check_revision_history_flag(
    is_controlled: Option<bool>,
    has_history: bool,
    owner: &str,
) -> Result<(), RuleViolation> {
    match is_controlled {
        Some(true) if !has_history => Err(RuleViolation::new(
            "AUTHORED_RESOURCE.Revision_history_valid",
            format!("{owner} declares is_controlled but carries no revision_history"),
        )),
        Some(false) if has_history => Err(RuleViolation::new(
            "AUTHORED_RESOURCE.Revision_history_valid",
            format!("{owner} carries a revision_history while declaring is_controlled false"),
        )),
        _ => Ok(()),
    }
}

/// One language code against the openEHR `languages` code set.
fn check_language(
    terminology: &OpenehrTerminology,
    code: &str,
    invariant: &'static str,
    what: &str,
) -> Result<(), RuleViolation> {
    if terminology.is_valid_language(code) {
        Ok(())
    } else {
        Err(RuleViolation::new(
            invariant,
            format!("{what} names '{code}', which is not in the openEHR languages code set"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;
    use openehr_its::opt14::types::ResourceDescriptionItem;

    fn code_phrase(code: &str) -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: "ISO_639-1".to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    fn item(language: &str, purpose: &str) -> ResourceDescriptionItem {
        ResourceDescriptionItem {
            language: code_phrase(language),
            purpose: purpose.to_owned(),
            keywords: Vec::new(),
            use_: None,
            misuse: None,
            copyright: None,
            original_resource_uri: None,
            other_details: None,
        }
    }

    fn description(items: Vec<ResourceDescriptionItem>) -> ResourceDescription {
        ResourceDescription {
            original_author: std::iter::once(("name".to_owned(), "tester".to_owned())).collect(),
            other_contributors: Vec::new(),
            lifecycle_state: "Initial".to_owned(),
            resource_package_uri: None,
            other_details: None,
            details: items,
            parent_resource: None,
        }
    }

    fn translation(language: &str) -> TranslationDetails {
        TranslationDetails {
            language: code_phrase(language),
            author: indexmap::IndexMap::new(),
            accreditation: None,
            other_details: None,
        }
    }

    /// The reported case (#3129): a hand-written OPT that left `use` and
    /// `misuse` empty took two uploads of a 1.7 MB file to find both, because
    /// the check returned at the first. Both come back together now.
    #[test]
    fn an_empty_use_and_an_empty_misuse_are_reported_together() {
        let mut detail = item("en", "testing");
        detail.use_ = Some(String::new());
        detail.misuse = Some(String::new());

        let codes: Vec<&str> = check_description(openehr(), &description(vec![detail]), "t")
            .iter()
            .map(|v| v.code)
            .collect();

        assert!(
            codes.contains(&"RESOURCE_DESCRIPTION_ITEM.Use_valid")
                && codes.contains(&"RESOURCE_DESCRIPTION_ITEM.misuse_valid"),
            "both empty fields must be reported in one response, got: {codes:?}"
        );
    }

    /// Every defect of a detail comes back at once, not just the first: an
    /// upload cycle per rule is what #3129 removed.
    #[test]
    fn every_defect_of_one_description_detail_is_reported_at_once() {
        let mut detail = item("en", "");
        detail.use_ = Some(String::new());
        detail.misuse = Some(String::new());
        detail.copyright = Some(String::new());

        let codes: Vec<&str> = check_description(openehr(), &description(vec![detail]), "t")
            .iter()
            .map(|v| v.code)
            .collect();

        for expected in [
            "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
            "RESOURCE_DESCRIPTION_ITEM.Use_valid",
            "RESOURCE_DESCRIPTION_ITEM.misuse_valid",
            "RESOURCE_DESCRIPTION_ITEM.copyright_valid",
        ] {
            assert!(codes.contains(&expected), "missing {expected} in {codes:?}");
        }
    }

    /// The one violation a single-defect fixture is expected to produce.
    ///
    /// `check_description` reports every violation it finds (#3129), so these
    /// tests assert the code of a fixture built to carry exactly one; a fixture
    /// that grew a second defect fails here rather than passing quietly on the
    /// first.
    fn only(violations: Vec<RuleViolation>) -> RuleViolation {
        assert_eq!(
            violations.len(),
            1,
            "fixture must carry exactly one defect, got: {:?}",
            violations.iter().map(|v| v.code).collect::<Vec<_>>()
        );
        violations.into_iter().next().expect("one violation")
    }

    #[test]
    fn a_valid_description_passes() {
        assert!(
            check_description(openehr(), &description(vec![item("en", "testing")]), "t").is_empty()
        );
    }

    #[test]
    fn empty_original_author_is_refused() {
        let mut d = description(vec![item("en", "testing")]);
        d.original_author.clear();
        let v = only(check_description(openehr(), &d, "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION.Original_author_valid");
    }

    #[test]
    fn empty_lifecycle_state_is_refused() {
        let mut d = description(vec![item("en", "testing")]);
        d.lifecycle_state.clear();
        let v = only(check_description(openehr(), &d, "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION.Lifecycle_state_valid");
    }

    #[test]
    fn detail_less_description_is_refused() {
        let v = only(check_description(openehr(), &description(Vec::new()), "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION.Details_valid");
    }

    #[test]
    fn duplicate_detail_language_is_refused() {
        let d = description(vec![item("en", "testing"), item("en", "again")]);
        let v = only(check_description(openehr(), &d, "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION.Details_valid");
    }

    #[test]
    fn empty_purpose_and_empty_optionals_are_refused() {
        let d = description(vec![item("en", "")]);
        let v = only(check_description(openehr(), &d, "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION_ITEM.Purpose_valid");

        let mut with_use = item("en", "testing");
        with_use.use_ = Some(String::new());
        let v = only(check_description(
            openehr(),
            &description(vec![with_use]),
            "t",
        ));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION_ITEM.Use_valid");
    }

    #[test]
    fn a_detail_language_outside_the_code_set_is_refused() {
        let d = description(vec![item("xx-not-a-language", "testing")]);
        let v = only(check_description(openehr(), &d, "t"));
        assert_eq!(v.code, "RESOURCE_DESCRIPTION_ITEM.Language_valid");
    }

    #[test]
    fn a_translation_restating_the_original_is_refused() {
        let v = check_translations(openehr(), &[translation("en")], "en").unwrap_err();
        assert_eq!(v.code, "AUTHORED_RESOURCE.Translations_valid");
    }

    #[test]
    fn duplicate_translation_languages_are_refused() {
        let v = check_translations(openehr(), &[translation("de"), translation("de")], "en")
            .unwrap_err();
        assert_eq!(v.code, "AUTHORED_RESOURCE.Translations_valid");
    }

    #[test]
    fn the_revision_history_xor_binds_only_when_declared() {
        assert!(check_revision_history_flag(None, false, "t").is_ok());
        assert!(check_revision_history_flag(None, true, "t").is_ok());
        assert!(check_revision_history_flag(Some(true), false, "t").is_err());
        assert!(check_revision_history_flag(Some(false), true, "t").is_err());
        assert!(check_revision_history_flag(Some(false), false, "t").is_ok());
    }
}
