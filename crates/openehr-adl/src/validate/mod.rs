// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM2 validation engine: the five public entry points and the phase
//! drivers over the topic modules ([`catalogue`], [`bindings`], [`slots`],
//! [`rm`], [`conformance`] and the private ones).
//!
//! The catalogue and phase schedule are `AOM2/master08-validation.adoc`: phase 2
//! runs only after phase 1 passes, and phase 3 on the flattened form. A level-0
//! parent is its own flat form (`ADL2/master09.02` §Differential and Flat
//! Forms); a specialised one goes through [`crate::flatten::flat_form`].
//! Parent-dependent checks take an optional [`ArchetypeRepository`] and keep
//! the half they can compute without one.

mod annotations;
pub mod bindings;
pub mod catalogue;
pub mod conformance;
mod flat;
mod identification;
pub mod resource_meta;
pub mod rm;
pub mod rm_extended;
mod scope;
pub mod slots;
mod source_level;
mod specialisation;
mod structure;
mod terminology;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;

use crate::artefact::{ArchetypeRepository, ArchetypeView, FlatParent, resolve_flat_parent, view};
use crate::error::SyntaxError;
use crate::parse::Dialect;
use crate::source::{SourceArtefact, parse_source};
use crate::validate::catalogue::{Severity, ValidationCode};

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// The catalogue code.
    pub code: ValidationCode,
    /// The severity (mirrors [`ValidationCode::severity`]).
    pub severity: Severity,
    /// A concrete, human-readable description of the specific violation.
    pub message: String,
    /// The archetype path the issue is anchored at, where derivable.
    pub path: Option<String>,
    /// A source byte span, for source-level checks (e.g. VOKU) where derivable.
    pub span: Option<std::ops::Range<usize>>,
}

impl ValidationIssue {
    /// Build an issue for `code` with `message`, deriving the severity.
    pub(crate) fn new(code: ValidationCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.severity(),
            message: message.into(),
            path: None,
            span: None,
        }
    }

    /// Attach an archetype path.
    #[must_use]
    pub(crate) fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// Append a path-anchored issue to `issues` — the one construction site every
/// walker in this module tree uses, so message + path shaping is identical
/// across the phase-1, specialisation, reference-model and flat-form walks.
pub(super) fn push_issue(
    issues: &mut Vec<ValidationIssue>,
    code: ValidationCode,
    msg: impl Into<String>,
    path: &str,
) {
    issues.push(ValidationIssue::new(code, msg).at_path(path.to_owned()));
}

/// Run the basic-integrity catalogue over `v` in the order of
/// `master08-validation.adoc` §Phase 1: identification, structure, then
/// terminology, which needs a clean terminology structure.
///
/// `dialect` selects the catalogue; see [`validate_source_integrity`] for the
/// ADL 1.4 subset.
fn run_integrity_checks(
    v: &ArchetypeView<'_>,
    repo: Option<&ArchetypeRepository>,
    source: Option<(&SourceArtefact, &str)>,
    dialect: Dialect,
    issues: &mut Vec<ValidationIssue>,
) {
    // ── basic identification / meta-data checks (master08 §Basic checks +
    //    §AUTHORED_ARCHETYPE meta-data checks) ──────────────────────────────
    let mut basic = Vec::new();
    identification::check_identification(v, repo, dialect, &mut basic);

    // ── terminology structure (STCNT / VOLT) — gates the code checks ───────
    let term_status = identification::terminology_structure(v);
    match term_status {
        identification::TermStructure::Empty => {
            // STCNT: any missing mandatory part, e.g. the `terminology` section
            // (master08 §Basic checks; no full vendored text — NOTE-flagged).
            if v.kind != crate::source::ArtefactKind::TemplateOverlay {
                basic.push(ValidationIssue::new(
                    ValidationCode::Stcnt,
                    "the terminology section defines no term_definitions",
                ));
            }
        }
        identification::TermStructure::MissingOriginalLanguage => {
            // VOLT: original language available in the terminology section
            // (master08 §AUTHORED_ARCHETYPE meta-data checks; NOTE-flagged).
            issues.push(ValidationIssue::new(
                ValidationCode::Volt,
                format!(
                    "the original language {:?} has no term_definitions bucket",
                    identification::original_language(v)
                ),
            ));
        }
        identification::TermStructure::Ok => {}
    }

    // ── structural definition walk (always runs; independent rules) ────────
    structure::check_structure(v, dialect, issues);
    annotations::check_annotations(v, issues);
    annotations::check_rm_overlay(v, issues);
    identification::check_resource_description_languages(v, issues); // VRDLA
    if let Some((src, text)) = source {
        source_level::check_object_key_unique(src, issues); // VOKU (source-level)
        source_level::check_rule_paths(v, src, text, issues); // VRRLP (raw rules text)
        if dialect == Dialect::Adl14 {
            identification::check_concept_term_adl14(v, src, issues); // VARCN (terminology half)
            source_level::check_deprecated_domain_spelling_adl14(text, issues); // W14DEP
        }
    }

    let basic_clean = basic.is_empty();
    issues.append(&mut basic);

    // ── terminology + code checks (gated: basic clean + terminology Ok) ────
    if basic_clean && term_status == identification::TermStructure::Ok {
        terminology::check_terminology(v, dialect, issues);
    }
}

/// Validate an assembled [`Archetype`] against the ADL2 basic-integrity
/// catalogue (`master08` §Phase 1).
///
/// With `repo` the parent-dependent checks (VACSD, VASID, VALC, VTPL) run in
/// full. The source-level checks (VOKU) need [`validate_source_integrity`].
#[must_use]
pub fn validate_integrity(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    run_integrity_checks(&view(archetype), repo, None, Dialect::Adl2, &mut issues);
    issues
}

/// Parses `src` in `dialect` and runs that dialect's basic-integrity catalogue,
/// including the source-level checks (VOKU, VRRLP).
///
/// A 1.4 upload is judged as 1.4. Four AOM2-only rules are suppressed for it at
/// their check sites (VARAV/VARRV, VCOID, VATCV, VCOSU), and three run only for
/// it: VCOC (`ADL1.4/master05-cadl.adoc` §Occurrences), definedness over the
/// qualified term-constraint form (`master09-customising_adl.adoc` §Custom
/// Syntax), and VDFPT (`master08-adl.adoc` §Definition Section).
///
/// # Errors
/// Returns the parse [`SyntaxError`]s if `src` does not parse in `dialect`;
/// validation runs only on a successful parse.
pub fn validate_source_integrity(
    src: &str,
    dialect: Dialect,
    repo: Option<&ArchetypeRepository>,
) -> Result<Vec<ValidationIssue>, Vec<SyntaxError>> {
    let source = parse_source(src, dialect)?;
    let archetype = crate::assemble::assemble(&source, src, dialect)?;
    let mut issues = Vec::new();
    run_integrity_checks(
        &view(&archetype),
        repo,
        Some((&source, src)),
        dialect,
        &mut issues,
    );
    if dialect == Dialect::Adl14 {
        issues.extend(flat::validate_definition_paths_adl14(&archetype));
    }
    Ok(issues)
}

/// Parses an ADL 1.4 source and validates it against the full 1.4 catalogue:
/// the 1.4 basic-integrity pass plus VUNT (`ADL1.4/master05-cadl.adoc` §Internal
/// References), which runs only after an error-free integrity pass.
///
/// # Errors
/// Returns the parse [`SyntaxError`]s if `src` does not parse as ADL 1.4;
/// validation runs only on a successful parse.
pub fn validate_adl14_source(
    src: &str,
    rm: &dyn rm::RmModel,
) -> Result<Vec<ValidationIssue>, Vec<SyntaxError>> {
    let source = parse_source(src, Dialect::Adl14)?;
    let archetype = crate::assemble::assemble(&source, src, Dialect::Adl14)?;
    let mut issues = Vec::new();
    run_integrity_checks(
        &view(&archetype),
        None,
        Some((&source, src)),
        Dialect::Adl14,
        &mut issues,
    );
    issues.extend(flat::validate_definition_paths_adl14(&archetype));
    // The RM resource package governs AOM 1.4 meta-data (RM common
    // `master08-resource_package.adoc` front-matter NOTE) — 1.4 sources only.
    resource_meta::check(&view(&archetype), &mut issues);
    if issues.iter().all(|i| i.severity != Severity::Error) {
        issues.extend(rm::validate_rm_conformance(&archetype, rm, Dialect::Adl14));
    }
    Ok(issues)
}

/// Validates an assembled [`Archetype`] against the full schedule: basic
/// integrity, then, only if that raised no [`Severity::Error`], the RM,
/// parent-conformance and flat-form checks (`master08` §Overview).
///
/// With [`bindings::NoTerminologyResolver`] VETDF is not raised (`master03`
/// §Validity Rules: "subject to tool accessibility").
#[must_use]
pub fn validate(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
    rm: &dyn rm::RmModel,
    resolver: &dyn bindings::TerminologyResolver,
) -> Vec<ValidationIssue> {
    let mut issues = validate_integrity(archetype, repo);
    if issues.iter().all(|i| i.severity != Severity::Error) {
        issues.extend(rm::validate_rm_conformance(archetype, rm, Dialect::Adl2));
    }
    run_parent_conformance(archetype, repo, rm, &mut issues);
    run_flat_form_checks(archetype, repo, &mut issues);
    run_filler_checks(archetype, repo, &mut issues);
    bindings::check_external_term_bindings(&view(archetype), resolver, &mut issues);
    issues
}

/// Run the specialised-child-against-flat-parent checks, gated on a repository
/// and a clean issue list; an unresolvable or unflattened parent skips them.
fn run_parent_conformance(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
    rm: &dyn rm::RmModel,
    issues: &mut Vec<ValidationIssue>,
) {
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return;
    }
    let Some(repo) = repo else {
        return;
    };
    match resolve_flat_parent(archetype, repo) {
        FlatParent::Available(parent) => {
            issues.extend(specialisation::validate_against_flat_parent(
                archetype, parent, rm, repo,
            ));
        }
        FlatParent::NeedsFlattener => {
            // The declared parent is itself specialised: flatten it to its deep
            // flat form, then validate against that (`master08` §Flattening:
            // process each parent in order from the top).
            if let Some(parent_id) = view(archetype).parent_archetype_id
                && let Some(parent) =
                    repo.resolve(parent_id, view(archetype).archetype_id.namespace.as_deref())
                && let Ok(flat_parent) = crate::flatten::flat_form(parent, repo)
            {
                issues.extend(specialisation::validate_against_flat_parent(
                    archetype,
                    &flat_parent,
                    rm,
                    repo,
                ));
            }
        }
        // NOTE: a stated parent absent from the repository is VASID, raised by
        // the identification pass (`specialisation::check_specialisation_depth`).
        FlatParent::NotSpecialised | FlatParent::NotFound => {}
    }
}

/// Run the flat-form checks on the flattened archetype, gated on a clean issue
/// list (`master08` §Phase 3); skipped when the archetype cannot be flattened.
fn run_flat_form_checks(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
    issues: &mut Vec<ValidationIssue>,
) {
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return;
    }
    let flat = repo.and_then(|r| crate::flatten::flat_form(archetype, r).ok());
    let flat_ref = match &flat {
        Some(f) => f,
        None if view(archetype).parent_archetype_id.is_none() => archetype,
        None => return,
    };
    issues.extend(flat::validate_flat_form_structure(flat_ref));
    // The deferred flat-form terminology/structure checks (VATDF / VTVSMD /
    // VACMCU / WACMCL / VCOSU) run only for a *specialised* archetype: a
    // non-specialised archetype is its own flat form, so the integrity topic
    // modules already ran their equivalents (never double-firing here).
    if view(archetype).is_specialised() {
        issues.extend(flat::validate_flat_form(flat_ref));
    }
}

/// Run the repository-dependent filler checks (VARXR, VTPL), gated on a
/// repository and a clean issue list (`master04.5` §Validity Rules:
/// `C_ARCHETYPE_ROOT`).
fn run_filler_checks(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
    issues: &mut Vec<ValidationIssue>,
) {
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return;
    }
    // NOTE: OPT2 `master03-opt_raw` gathers each constituent, flattened, into the
    // operational template, so it carries no external reference left to resolve.
    if view(archetype).kind == crate::source::ArtefactKind::OperationalTemplate {
        return;
    }
    if let Some(repo) = repo {
        issues.extend(slots::validate_fillers(archetype, repo));
    }
}

/// Parses and validates ADL2 `src` against the schedule of [`validate`], plus
/// the source-level integrity checks.
///
/// NOTE: the flat-form checks run for a top-level archetype too, whose flat form
/// is its differential (`master03-archetype_package.adoc` §Validity Rules;
/// `ADL2/master09.02` §Differential and Flat Forms).
///
/// # Errors
/// Returns the parse [`SyntaxError`]s if `src` does not parse into an
/// [`Archetype`]; validation runs only on a successful parse.
pub fn validate_source(
    src: &str,
    repo: Option<&ArchetypeRepository>,
    rm: &dyn rm::RmModel,
    resolver: &dyn bindings::TerminologyResolver,
) -> Result<Vec<ValidationIssue>, Vec<SyntaxError>> {
    let source = parse_source(src, Dialect::Adl2)?;
    let archetype = crate::assemble::assemble(&source, src, Dialect::Adl2)?;
    let mut issues = Vec::new();
    run_integrity_checks(
        &view(&archetype),
        repo,
        Some((&source, src)),
        Dialect::Adl2,
        &mut issues,
    );
    if issues.iter().all(|i| i.severity != Severity::Error) {
        issues.extend(rm::validate_rm_conformance(&archetype, rm, Dialect::Adl2));
    }
    run_parent_conformance(&archetype, repo, rm, &mut issues);
    run_flat_form_checks(&archetype, repo, &mut issues);
    run_filler_checks(&archetype, repo, &mut issues);
    bindings::check_external_term_bindings(&view(&archetype), resolver, &mut issues);
    Ok(issues)
}
