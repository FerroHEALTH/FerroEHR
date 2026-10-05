// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM2 validation engine: orchestration over a catalogue of topic modules.
//!
//! A validator walks an assembled `openehr_am::v2_4::aom2` [`Archetype`]
//! and produces typed [`ValidationIssue`]s, each carrying a
//! [`ValidationCode`] (one variant per AOM2 validity
//! code), a [`Severity`], a message, and — where derivable
//! — the archetype path the issue is anchored at.
//!
//! This module holds ONLY the orchestration: [`ValidationIssue`], the five
//! public entry points, and the three drivers (`run_integrity_checks`,
//! `run_parent_conformance`, `run_flat_form_checks`) that call the topic modules
//! in the order the spec's phase schedule prescribes — master08's "phase 1 /
//! phase 2 / phase 3" in the spec's guide vocabulary. Every rule lives in the
//! topic module that owns its subject matter:
//!
//! | module | topic |
//! |---|---|
//! | [`catalogue`] | the code vocabulary: `Severity` + `ValidationCode` |
//! | `identification` | archetype id, root typename/concept, versions, languages, the terminology-structure gate |
//! | `structure` | the phase-1 definition-tree walk (node identity, cardinality, slots, assumed values) |
//! | `terminology` | term definitions, value sets and the codes the definition references |
//! | [`bindings`] | binding keys (VTTBK/VTCBK) + the external terminology-service seam (VETDF) |
//! | `annotations` | the `annotations` and `rm_overlay` sections |
//! | `source_level` | the checks that read the raw parsed source (VOKU, VRRLP) |
//! | `specialisation` | a differential child against its flat parent |
//! | [`slots`] | slot redefinition/filling + the template-filler pass |
//! | [`rm`] | the reference-model seam and its checks |
//! | [`conformance`] | the `master04.5` conformance functions the above build on |
//! | `flat` | everything decidable only on the flattened form |
//!
//! The phase-1 catalogue and its orchestration are defined in
//! `docs/specs/openehr/AM/docs/AOM2/master08-validation.adoc` §Phase 1 - Basic
//! Integrity, with the full rule texts in `master03-archetype_package.adoc`,
//! `master04.5-constraint_model-class_definitions.adoc`,
//! `master06-rm_overlay.adoc`, and `master07-terminology_package.adoc` (cited
//! per check in the topic modules); the phase-2 RM checks are `master08` §Phase
//! 2 → Validate Against Reference Model (cited per check in [`rm`]).
//!
//! Phase orchestration follows `master08` "multi-pass … more basic kinds of
//! errors being checked first": phase 2 runs only after phase 1 passes, and both
//! full-pipeline entries ([`validate`] / [`validate_source`]) run the SAME phase
//! set, since AOM2 defines no partial-validation profile. Parent-dependent
//! checks (VACSD, VASID, VALC, VTPL) take an optional [`ArchetypeRepository`]
//! and degrade to the standalone half they can compute without one.
//!
//! The specialised-vs-flat-parent checks live in `specialisation` (slot arm in
//! [`slots`]) and run against the parent [`resolve_flat_parent`] returns. Per
//! `ADL2/master09.02` §Differential and Flat Forms ("For a top-level archetype,
//! the flat-form is the same as its differential form") a level-0 parent is used
//! as-is; a specialised parent ([`FlatParent::NeedsFlattener`]) goes through
//! [`crate::flatten::flat_form`] first. Phase 3 (`flat`) then runs on the
//! flattened form.

mod annotations;
pub mod bindings;
pub mod catalogue;
pub mod conformance;
mod flat;
mod identification;
mod resource_meta;
pub mod rm;
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

/// Run the basic-integrity catalogue over `v`, appending issues to `issues`
/// (master08 "phase 1 — basic integrity" in the spec's guide vocabulary).
///
/// The order follows `master08-validation.adoc` §Phase 1 - Basic Integrity:
/// basic identification checks first, then structural, then terminology (the
/// latter gated behind a clean terminology structure and no basic error —
/// master08 "basic errors first", and a code cannot be checked against a
/// missing/inconsistent terminology).
///
/// `dialect` selects the validity catalogue: [`Dialect::Adl2`] runs the full
/// AOM2 integrity catalogue; [`Dialect::Adl14`] runs the subset that corresponds
/// to the ADL 1.4 / AOM 1.4 standalone validity rules (see
/// [`validate_source_integrity`] for the correspondence + the suppressed
/// AOM2-only rules, each spec-cited at its check site).
///
/// Not run in phase 1, though the catalogue carries their variants: the
/// reference-model checks and the path halves of VRANP/VRRLP/VRMVP ([`rm`]),
/// VDIFP + VSONIF (`specialisation`), the flat-form halves
/// VATDF/VTVSMD/VACMCU/VCOSU (`flat`), the external-reference half of VARXR
/// ([`slots`]), and VETDF, which needs the [`bindings::TerminologyResolver`]
/// seam because a network-free engine holds no terminology-service client.
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

/// Validate an assembled [`Archetype`] against the AOM2 basic-integrity
/// catalogue (master08 "phase 1 — basic integrity" in the spec's guide
/// vocabulary).
///
/// The catalogue is the ADL2 one: an assembled [`Archetype`] carries no memory
/// of the dialect it was read from, so the dialect-sensitive entry is the
/// source-level [`validate_source_integrity`].
///
/// `repo`, when supplied, resolves the archetype's parent (and suppliers) so
/// the parent-dependent integrity checks (VACSD depth comparison, VASID, VALC,
/// VTPL) can run; when `None`, those checks compute only their standalone half
/// or are skipped. Source-level checks that need the raw ODIN text (VOKU) are
/// not run here — use [`validate_source_integrity`] for those.
#[must_use]
pub fn validate_integrity(
    archetype: &Archetype,
    repo: Option<&ArchetypeRepository>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    run_integrity_checks(&view(archetype), repo, None, Dialect::Adl2, &mut issues);
    issues
}

/// Parses `src` in `dialect` and runs that dialect's basic-integrity catalogue.
///
/// The pass includes the source-level checks (VOKU keyed-list uniqueness,
/// VRRLP rule paths) that need the raw ODIN text (master08 "phase 1 — basic
/// integrity" in the spec's guide vocabulary).
///
/// Under [`Dialect::Adl2`] this is the full AOM2 integrity catalogue.
///
/// Under [`Dialect::Adl14`] it is the subset that corresponds to the ADL 1.4 /
/// AOM 1.4 standalone validity rules, plus the 1.4-only definition-path walk
/// (VDFPT).
///
/// A 1.4 upload is judged AS 1.4, on its 1.4-shaped `openehr_am::v2_4` model,
/// never post-conversion: converting to ADL 2 changes the artefact. The checks
/// corresponding to an ADL 1.4 / AOM 1.4 rule run unchanged; four AOM2-only
/// rules that would false-reject a valid 1.4 archetype are suppressed at their
/// check sites in `identification` / `structure` / `terminology`, each cited
/// there: VARAV/VARRV (AOM 1.4 has no 3-part `adl_version`/`rm_release`), VCOID
/// (relaxed to the AOM 1.4 `node_id` rule), VATCV (1.4 terminology constraints
/// are not ADL2 code forms) and VCOSU (1.4 node ids are only sibling-unique).
///
/// Three checks are 1.4-ONLY, absent from AOM2:
/// - **VCOC** — cardinality/occurrences validity over the children's EFFECTIVE
///   occurrences (`ADL1.4/master05-cadl.adoc` §Occurrences L321-324).
/// - **VATDF/VACDF over the 1.4 term-constraint spelling** — the qualified form
///   `[local:: a, b ; assumed]` of `ADL1.4/master09-customising_adl.adoc`
///   §Custom Syntax is decomposed so definedness is judged per code; external
///   terminology codes are not archetype terms and are excluded.
/// - **VDFPT** — `use_node` target paths must resolve within the definition
///   section (`ADL1.4/master08-adl.adoc` §Definition Section).
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

/// Parses an **ADL 1.4** source and validates it against the full 1.4 catalogue.
///
/// The catalogue is the 1.4 basic-integrity pass
/// ([`validate_source_integrity`] with [`Dialect::Adl14`]) plus the one 1.4
/// validity rule that needs a reference model, VUNT
/// ([`rm::validate_rm_conformance`] in the 1.4 dialect).
///
/// The 1.4 counterpart of [`validate_source`], and a separate entry rather than
/// a dialect branch because ADL 1.4 has no differential lineage to flatten and
/// no AOM2 external-binding rule: an [`ArchetypeRepository`] or a
/// [`bindings::TerminologyResolver`] would be a silently ignored parameter.
///
/// VUNT is a rule of the ADL 1.4 formalism itself (`ADL1.4/master05-cadl.adoc`
/// §Internal References L512-513). The RM pass runs only when the integrity
/// pass raised no [`Severity::Error`] (`master08` §Overview phase gate), and a
/// type `rm` does not know is undecidable rather than wrong, so an artefact on
/// an unknown reference model raises no VUNT.
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

/// Validates an assembled [`Archetype`] against the full schedule.
///
/// The schedule is basic integrity, then — only if that raised no
/// [`Severity::Error`] — the reference-model checks against `rm`, the
/// parent-conformance checks, and the flat-form checks (master08's "phase 1 /
/// phase 2 / phase 3" in the spec's guide vocabulary).
///
/// The gating is the `master08` §Overview phase gate ("more basic kinds of
/// errors being checked first"): each later pass runs on a structurally-sound
/// archetype only. [`validate_source`] runs this same schedule from source
/// text, plus the source-level integrity checks (VOKU) that need the raw ODIN
/// and are not available here.
///
/// `resolver` verifies external term bindings (VETDF); pass
/// [`bindings::NoTerminologyResolver`] when no terminology service is
/// available (VETDF is then not raised — `master03` §Validity Rules "subject to
/// tool accessibility").
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
    bindings::check_external_term_bindings(&view(archetype), resolver, &mut issues);
    issues
}

/// Run the specialised-child-against-flat-parent checks, gated on a supplied
/// repository and a still-clean issue list (`master08` §Overview phase gate;
/// these are its "phase 2 — validate against parent" in the spec's guide
/// vocabulary). A non-specialised archetype, an unresolved parent, or a parent
/// that itself needs the flattener silently skips the checks (never a wrong
/// answer against an un-flattened parent).
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
                && let Some(parent) = repo.get(parent_id)
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

/// Run the flat-form checks (VUNP, VACMCO) on the flattened archetype, gated on
/// a still-clean issue list (`master08` §Phase 3 - Validation of Flat Form —
/// carried out after successful flat-form generation). A specialised archetype
/// is flattened via [`crate::flatten::flat_form`] (needs `repo`); a level-0
/// archetype is its own flat form. If flattening is impossible (specialised, no
/// resolvable parent) the checks are skipped rather than run against a wrong
/// (un-flattened) form.
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

/// Parses and validates ADL2 `src` against the same schedule as [`validate`].
///
/// The source-level integrity checks are added: basic integrity, then — only
/// if that is error-free — the reference-model checks against `rm` (`master08`
/// §Overview phase gate), the parent-conformance checks, and the flat-form
/// checks.
///
/// NOTE: the flat-form checks run even for a top-level archetype — the
/// V-codes are unconditional (`master03-archetype_package.adoc` §Validity
/// Rules: "apply to all varieties of ARCHETYPE object"), AOM2 defines no
/// partial-validation profile, and a top-level archetype's flat form IS its
/// differential form (`ADL2/master09.02-spec_concepts.adoc` §Differential
/// and Flat Forms), so no phase may be omitted.
///
/// `resolver` verifies external term bindings (VETDF); pass
/// [`bindings::NoTerminologyResolver`] when no terminology service is
/// available (VETDF is then not raised — `master03` §Validity Rules "subject to
/// tool accessibility").
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
    bindings::check_external_term_bindings(&view(&archetype), resolver, &mut issues);
    Ok(issues)
}
