// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The language and binding filters of a profiled OPT
//! (`docs/specs/openehr/AM/docs/OPT2/master04-opt_profiled.adoc`).

use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

use super::{BindingFilter, OptError};

/// Keep only the listed languages across the OPT's language / terminology /
/// description sections (master04 §Language Filtering). At least one language
/// must remain.
pub(super) fn filter_languages(
    opt: &mut OperationalTemplate,
    keep: &[String],
) -> Result<(), OptError> {
    let keep_set: std::collections::BTreeSet<&str> = keep.iter().map(String::as_str).collect();

    // The resulting language set must be non-empty.
    let root_lang = opt.original_language.code_string.clone();
    let surviving: Vec<&String> = opt
        .terminology
        .term_definitions
        .keys()
        .filter(|l| keep_set.contains(l.as_str()))
        .collect();
    if surviving.is_empty() && !keep_set.contains(root_lang.as_str()) {
        return Err(OptError::NoLanguagesLeft);
    }

    retain_languages_terminology(&mut opt.terminology, &keep_set);
    if let Some(components) = opt.component_terminologies.as_mut() {
        for term in components.values_mut() {
            retain_languages_terminology(term, &keep_set);
        }
    }
    if let Some(translations) = opt.translations.as_mut() {
        translations.retain(|lang, _| keep_set.contains(lang.as_str()));
    }
    if let Some(description) = opt.description.as_mut()
        && let Some(details) = description.details.as_mut()
    {
        details.retain(|lang, _| keep_set.contains(lang.as_str()));
    }
    Ok(())
}

fn retain_languages_terminology(
    term: &mut ArchetypeTerminology,
    keep: &std::collections::BTreeSet<&str>,
) {
    term.term_definitions
        .retain(|lang, _| keep.contains(lang.as_str()));
}

/// Remove terminology bindings per `filter` (master04 §Terminology Binding
/// Filtering).
pub(super) fn filter_bindings(term: &mut ArchetypeTerminology, filter: &BindingFilter) {
    match filter {
        BindingFilter::KeepAll => {}
        BindingFilter::RemoveAll => term.term_bindings = None,
        BindingFilter::Remove(terminologies) => {
            if let Some(bindings) = term.term_bindings.as_mut() {
                for t in terminologies {
                    bindings.remove(t);
                }
                if bindings.is_empty() {
                    term.term_bindings = None;
                }
            }
        }
    }
}
