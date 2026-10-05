// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Terminology-binding topic: the local binding KEYS (VTTBK / VTCBK) and the
//! external terminology-service seam that answers VETDF.
//!
//! Key validity is
//! `docs/specs/openehr/AM/docs/AOM2/master07-terminology_package.adoc`
//! §Validity Rules (a binding key is a defined at/ac-code, or a path that
//! resolves in the archetype). External term validity is
//! `master03-archetype_package.adoc` §Validity Rules, VETDF: "external term
//! validity. Each external term used within the archetype definition must exist
//! in the relevant terminology (subject to tool accessibility; codes for
//! inaccessible terminologies should be flagged with a warning indicating that
//! no verification was possible)."
//!
//! `openehr-adl` is a network-free spec engine, so it holds no live
//! terminology-service client. VETDF is threaded through the
//! [`TerminologyResolver`] seam, like the [`RmModel`](super::rm::RmModel)
//! reference-model seam: the application injects a resolver backed by its
//! terminology service, and the full-validation entry points
//! ([`super::validate`] / [`super::validate_source`]) consult it. Every entry
//! point that takes no resolver behaves as if a [`NoTerminologyResolver`]
//! were supplied (VETDF is silently not raised — no verification was possible),
//! matching the spec's "subject to tool accessibility" carve-out.
//!
//! The `_type` of a binding target and the terminology-id → resolver mapping are
//! deliberately kept out of this crate: the resolver receives the binding
//! target reference exactly as authored and owns all terminology-specific
//! interpretation.

use std::collections::BTreeSet;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use crate::artefact::{ArchetypeView, view};
use crate::paths::{Resolution, has_node_id_predicate, resolve};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

// ── binding keys (VTTBK / VTCBK) ──────────────────────────────────────────

/// VTTBK / VTCBK: every `term_bindings` key must be a defined at/ac-code, or a
/// path that resolves within the archetype (`master07` §Validity Rules).
pub(super) fn check_bindings(
    v: &ArchetypeView<'_>,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(bindings) = v.terminology.term_bindings.as_ref() else {
        return;
    };
    for terms in bindings.values() {
        for key in terms.keys() {
            if let Some(issue) = binding_key_issue(v, defined, key) {
                issues.push(issue);
            }
        }
    }
}

/// The VTTBK/VTCBK violation one binding key raises, or `None` when it is
/// valid.
///
/// A constraint (ac) key must be a defined ac-code (VTCBK); a term key must be
/// a defined at-code (VTTBK); a non-code key that is not even a path (a bare
/// word) is never a valid binding target; and a node-id-predicated path must
/// resolve within the archetype. A pure-RM path (no predicate) is a
/// reference-model concern ([`super::rm`]).
fn binding_key_issue(
    v: &ArchetypeView<'_>,
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
    if has_node_id_predicate(key) && resolve(v.definition, key) != Resolution::Found {
        return Some(ValidationIssue::new(
            ValidationCode::Vttbk,
            format!("term binding key path {key:?} is not valid in the archetype"),
        ));
    }
    None
}

// ── the external terminology-service seam (VETDF) ──────────────────────────

/// The seam by which the application answers "does this code exist in this
/// terminology?" for the ADL2 VETDF check, without `openehr-adl` holding a
/// network client.
///
/// `code_exists` returns:
/// - `Some(true)` — the term is known to exist in the terminology (no VETDF);
/// - `Some(false)` — the term is definitely absent from the terminology
///   (**VETDF is raised**);
/// - `None` — the resolver cannot answer (the terminology is unknown or the
///   service is unavailable). No VETDF is raised: a validator must not reject an
///   archetype on an infrastructure gap, per the VETDF "subject to tool
///   accessibility; … no verification was possible" carve-out
///   (`master03-archetype_package.adoc` §Validity Rules).
pub trait TerminologyResolver {
    /// Whether `code` (the external term reference exactly as authored in the
    /// binding — typically a URI for an external terminology) exists in the
    /// terminology named by `terminology_id` (the outer binding key). `None` =
    /// could not be determined (see the trait docs).
    fn code_exists(&self, terminology_id: &str, code: &str) -> Option<bool>;
}

/// The default resolver: it can never answer, so VETDF is never raised.
///
/// Supplied implicitly by every validation entry point that takes no resolver,
/// so those paths are byte-for-byte unchanged (the VETDF "no verification was
/// possible" carve-out — `master03-archetype_package.adoc` §Validity Rules).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTerminologyResolver;

impl TerminologyResolver for NoTerminologyResolver {
    fn code_exists(&self, _terminology_id: &str, _code: &str) -> Option<bool> {
        None
    }
}

/// One external term binding extracted from an archetype's `term_bindings`.
///
/// Carries the external `terminology_id` (the outer binding key), the local
/// `key` bound (an at/ac-code or a path), and the `target` external term
/// reference (a URI).
///
/// Yielded by [`external_term_bindings`] so the application can pre-resolve the
/// same set the validator will consult (the resolver seam is synchronous; a
/// terminology lookup is async, so the app resolves ahead of time and hands the
/// validator a memoised resolver).
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

/// Whether `terminology_id` names a *genuinely external* terminology — one whose
/// term references VETDF must verify against a terminology service.
///
/// Excluded (returns `false`):
/// - `"local"` — the archetype's own term definitions
///   (`docs/specs/openehr/AM/docs/ADL2/master07.13-adl_terminology.adoc`
///   §Terminology: archetype-local terms), never an external terminology;
/// - `"openehr"` — the openEHR Terminology, treated here as archetype-internal:
///   its binding *keys* are validated by VTTBK/VTCBK (`master07` §Validity
///   Rules) and VETDF is scoped to third-party terminologies.
///
/// NOTE: openEHR publishes no enumeration of "external" terminology ids, so the
/// `local`/`openehr` exclusion set is our own scoping decision (the two
/// archetype-/openEHR-internal ids), not a spec-pinned list. The comparison is
/// ASCII-case-insensitive, matching openEHR's case-insensitive identifier rules
/// (`master03` §Lexical Conventions).
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

/// The external term bindings of `archetype` (`term_bindings` entries under a
/// genuinely-external terminology id — see `is_external_terminology`).
///
/// The application iterates these to pre-resolve each `(terminology_id, target)`
/// against its terminology service and build the [`TerminologyResolver`] the
/// validator then consults for VETDF.
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

/// VETDF: raise an issue for every external term binding whose target the
/// `resolver` reports as definitely absent (`Some(false)`).
///
/// `Some(true)` (exists) and `None` (could not verify) raise nothing —
/// `master03-archetype_package.adoc` §Validity Rules, VETDF: an inaccessible
/// terminology is flagged as unverifiable, not as invalid.
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
