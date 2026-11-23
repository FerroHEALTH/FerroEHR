// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Terminology bindings and the external terminology seam.
//!
//! The local binding keys (VTTBK, VTCBK; `AOM2/master07` §Validity Rules) and the
//! [`TerminologyResolver`] that answers VETDF (`master03` §Validity Rules:
//! "subject to tool accessibility"). Without a resolver VETDF is not raised.

use std::collections::BTreeSet;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use crate::artefact::{ArchetypeView, view};
use crate::paths::{Resolution, has_node_id_predicate, resolve_in_flat};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

// ── binding keys (VTTBK / VTCBK) ──────────────────────────────────────────

/// VTTBK / VTCBK: every `term_bindings` key must be a defined at/ac-code, or a
/// path that is valid in the flat archetype (`master07` §Validity Rules).
pub(super) fn check_bindings(
    v: &ArchetypeView<'_>,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    check_term_bindings(v.terminology, Some(v.definition), defined, issues);
}

/// VTTBK / VTCBK over the `term_bindings` of `term`, whose path keys resolve
/// against `definition`; with no definition only the key form is judged.
pub(super) fn check_term_bindings(
    term: &ArchetypeTerminology,
    definition: Option<&CComplexObject>,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(bindings) = term.term_bindings.as_ref() else {
        return;
    };
    for terms in bindings.values() {
        for key in terms.keys() {
            if let Some(issue) = binding_key_issue(definition, defined, key) {
                issues.push(issue);
            }
        }
    }
}

/// The VTTBK/VTCBK violation one binding key raises, or `None`: an ac key must
/// be a defined ac-code, a term key a defined at-code, and a node-id path must
/// resolve in the flat archetype ([`resolve_in_flat`]).
fn binding_key_issue(
    definition: Option<&CComplexObject>,
    defined: &BTreeSet<&str>,
    key: &str,
) -> Option<ValidationIssue> {
    if AdlCodeDefinitionsData::is_value_set_code(key) {
        return (!defined.contains(key)).then(|| {
            ValidationIssue::new(
                ValidationCode::Vtcbk,
                format!("constraint binding key {key:?} is not a defined ac-code"),
            )
        });
    }
    if AdlCodeDefinitionsData::is_at_code(key) || AdlCodeDefinitionsData::is_id_code(key) {
        return (!defined.contains(key)).then(|| {
            ValidationIssue::new(
                ValidationCode::Vttbk,
                format!("term binding key {key:?} is not a defined at-code"),
            )
        });
    }
    if !key.starts_with('/') {
        return Some(ValidationIssue::new(
            ValidationCode::Vttbk,
            format!("term binding key {key:?} is neither an at-code nor a path"),
        ));
    }
    if let Some(definition) = definition
        && has_node_id_predicate(key)
        && resolve_in_flat(definition, key) != Resolution::Found
    {
        return Some(ValidationIssue::new(
            ValidationCode::Vttbk,
            format!("term binding key path {key:?} is not valid in the archetype"),
        ));
    }
    None
}

// ── the external terminology-service seam (VETDF) ──────────────────────────

/// The seam by which the application answers "does this code exist in this
/// terminology?" for VETDF.
///
/// `Some(false)` raises VETDF; `None` (unknown or unavailable) raises nothing
/// (`master03-archetype_package.adoc` §Validity Rules).
pub trait TerminologyResolver {
    /// Whether `code` (the external term reference exactly as authored in the
    /// binding — typically a URI for an external terminology) exists in the
    /// terminology named by `terminology_id` (the outer binding key). `None` =
    /// could not be determined (see the trait docs).
    fn code_exists(&self, terminology_id: &str, code: &str) -> Option<bool>;
}

/// The default resolver: it never answers, so VETDF is never raised.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTerminologyResolver;

impl TerminologyResolver for NoTerminologyResolver {
    fn code_exists(&self, _terminology_id: &str, _code: &str) -> Option<bool> {
        None
    }
}

/// One external term binding of an archetype's `term_bindings`: the
/// terminology, the local `key`, and the `target` URI. The application
/// pre-resolves these for a synchronous resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTermBinding {
    /// The external terminology id (the outer `term_bindings` key), e.g.
    /// `"SNOMED-CT"`, `"LOINC"`, `"ISO_639-1"`.
    pub terminology_id: String,
    /// The local binding key: an at/ac-code or an archetype path.
    pub key: String,
    /// The external term reference the key is bound to (a URI), e.g.
    /// `"http://snomedct.info/id/12394009"`.
    pub target: String,
}

/// Whether `terminology_id` names an external terminology VETDF verifies.
///
/// NOTE: openEHR publishes no list of external terminologies; excluding `local`
/// and `openehr` is our own design, compared ASCII-case-insensitively.
fn is_external_terminology(terminology_id: &str) -> bool {
    let id = terminology_id.to_ascii_lowercase();
    id != "local" && id != "openehr"
}

/// Invoke `f(terminology_id, key, target)` for every external term binding in
/// `v` — the single walk both [`external_term_bindings`] and
/// [`check_external_term_bindings`] share, so the app pre-resolves exactly the
/// set the validator consults.
fn walk_external_bindings(v: &ArchetypeView<'_>, mut f: impl FnMut(&str, &str, &str)) {
    let Some(bindings) = v.terminology.term_bindings.as_ref() else {
        return;
    };
    for (terminology_id, entries) in bindings {
        if !is_external_terminology(terminology_id) {
            continue;
        }
        for (key, target) in entries {
            f(terminology_id, key, target);
        }
    }
}

/// The external term bindings of `archetype`, for the application to
/// pre-resolve into the [`TerminologyResolver`].
#[must_use]
pub fn external_term_bindings(archetype: &Archetype) -> Vec<ExternalTermBinding> {
    let v = view(archetype);
    let mut out = Vec::new();
    walk_external_bindings(&v, |terminology_id, key, target| {
        out.push(ExternalTermBinding {
            terminology_id: terminology_id.to_owned(),
            key: key.to_owned(),
            target: target.to_owned(),
        });
    });
    out
}

/// VETDF: an issue for every external binding the `resolver` reports absent
/// (`master03-archetype_package.adoc` §Validity Rules).
pub(super) fn check_external_term_bindings(
    v: &ArchetypeView<'_>,
    resolver: &dyn TerminologyResolver,
    issues: &mut Vec<ValidationIssue>,
) {
    walk_external_bindings(v, |terminology_id, key, target| {
        if resolver.code_exists(terminology_id, target) == Some(false) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vetdf,
                format!(
                    "external term {target:?} bound to {key:?} does not exist in terminology \
                     {terminology_id:?}"
                ),
            ));
        }
    });
}
