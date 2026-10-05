// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Slot-topic validity: `ARCHETYPE_SLOT` and its fillers.
//!
//! Covers redefining an inherited `ARCHETYPE_SLOT`, filling one with a
//! `C_ARCHETYPE_ROOT`, and the template/`use_archetype` filler checks that
//! resolve those references against a supplier repository.
//!
//! Rule texts:
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules — `ARCHETYPE_SLOT` (VDSSID L462 / VDSSM / VDSSP L468 /
//! VDSSC L471) and `C_ARCHETYPE_ROOT` (VARXR L419 / VARXS L424 / VARXID L427) —
//! plus `master03-archetype_package.adoc` §Validity Rules VTPL and
//! `master08-validation.adoc` §Phase 2 → Validate Specialised Definition.
//!
//! The first half is the slot arm of the specialisation walk: a second
//! `impl` block on `ParentScan`, reached from
//! `ParentScan::check_object_pair` in `super::specialisation` in unchanged
//! invocation order.
//!
//! The second half ([`validate_fillers`]) is the repository-dependent pass. VTPL
//! and VARXR both need the supplier repository (a filler is resolved,
//! flattened, and inspected), so they run only when a repository is available —
//! separately from the standalone integrity / parent-conformance passes. They
//! un-defer:
//!
//! * **VTPL** — template/filler language consistency (`master03` §Validity
//!   Rules, VTPL): every filler flattened into a *template* must support the
//!   template's `original_language`, so the flattened OPT can carry a common
//!   language (`OPT2/master03` §Terminology). Only templates are subject to
//!   VTPL — a plain archetype that merely references a mono-lingual filler is
//!   still valid, since it is not (yet) being flattened into an OPT.
//! * **VARXR** — external reference resolution (`master08` §Phase 2 → Validate
//!   Specialised Definition; `master04.5` §`C_ARCHETYPE_ROOT`): every
//!   `use_archetype` external reference / slot-filler must resolve to an
//!   archetype in the repository.
//!
//! Both walk the *flattened* artefact (`crate::flatten::flat_form`) so fillers
//! inherited from a specialisation parent are seen.

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::beom::core::assertion::Assertion;

use super::catalogue::ValidationCode;
use super::specialisation::ParentScan;
use super::{ValidationIssue, push_issue};
use crate::artefact::{ArchetypeRepository, ArchetypeView, view};
use crate::codes::specialisation_depth;
use crate::flatten::flat_form;
use crate::source::ArtefactKind;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

impl<'a> ParentScan<'a> {
    /// Checks for a child node that redefines an `ARCHETYPE_SLOT` in the flat
    /// parent (slot filling or slot narrowing).
    pub(super) fn check_slot_redefinition(
        &mut self,
        child: &'a CObject,
        parent_slot: &'a ArchetypeSlot,
        path: &str,
    ) {
        match child {
            // A `C_ARCHETYPE_ROOT` filler for the slot (`master04.5` §`C_ARCHETYPE_ROOT`
            // VARXID L427 / VARXS L424 / VARXR L419).
            CObject::CComplexObject(CComplexObject::CArchetypeRoot(root)) => {
                self.check_slot_filler(root, parent_slot, path);
            }
            // A narrowed / closed `ARCHETYPE_SLOT` (`master04.5` §`ARCHETYPE_SLOT`
            // VDSSID L462 / VDSSP L468 / VDSSC L471).
            CObject::ArchetypeSlot(child_slot) => {
                self.check_slot_identity(child_slot, parent_slot, path);
                // VDSSM: a specialised slot must narrow the parent slot or be
                // closed (`master04.5` §`ARCHETYPE_SLOT`), i.e. be a PROPER
                // subset of its admitted-archetype set. Regex-language subset is
                // undecidable, so `slot_narrows` stands in with three decidable
                // checks.
                let narrows = slot_narrows(child_slot);
                if !child_slot.is_closed {
                    if !narrows {
                        push_issue(
                            &mut self.issues,
                            ValidationCode::Vdssm,
                            "a specialised slot must narrow the parent slot or be closed",
                            path,
                        );
                    } else if slot_assertions_equal(
                        child_slot.includes.as_deref().unwrap_or_default(),
                        parent_slot.includes.as_deref().unwrap_or_default(),
                    ) && slot_assertions_equal(
                        child_slot.excludes.as_deref().unwrap_or_default(),
                        parent_slot.excludes.as_deref().unwrap_or_default(),
                    ) {
                        push_issue(
                            &mut self.issues,
                            ValidationCode::Vdssm,
                            "a specialised slot must be a proper narrowing, not a restatement of the parent slot constraints",
                            path,
                        );
                    } else if let Some(widened) = slot_widens_by_literal(child_slot, parent_slot) {
                        push_issue(
                            &mut self.issues,
                            ValidationCode::Vdssm,
                            format!(
                                "the specialised slot admits archetype {widened:?}, which the parent slot does not — not a proper subset"
                            ),
                            path,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// The identity and closure rules of a specialised slot (`master04.5`
    /// §`ARCHETYPE_SLOT`): VDSSID (identical node id), VDSSP (the parent slot
    /// must not already be closed), VDSSC (closed OR narrowed, never both).
    ///
    /// NOTE: the vendored `masterAppB` slot grammar admits `closed` XOR a
    /// `matches` body, so the VDSSC guard can only fire for AOM input that was
    /// not parsed from ADL2 source.
    fn check_slot_identity(
        &mut self,
        child_slot: &ArchetypeSlot,
        parent_slot: &ArchetypeSlot,
        path: &str,
    ) {
        if child_slot.node_id != parent_slot.node_id {
            push_issue(
                &mut self.issues,
                ValidationCode::Vdssid,
                format!(
                    "specialised slot node id {:?} is not identical to the parent slot id {:?}",
                    child_slot.node_id, parent_slot.node_id
                ),
                path,
            );
        }
        if parent_slot.is_closed {
            push_issue(
                &mut self.issues,
                ValidationCode::Vdssp,
                "cannot specialise a slot that is already closed in the flat parent",
                path,
            );
        }
        if child_slot.is_closed && slot_narrows(child_slot) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vdssc,
                "a specialised slot cannot be both closed and narrowed",
                path,
            );
        }
    }

    /// VARXID / VARXS / VARXR for a `C_ARCHETYPE_ROOT` filling a parent slot.
    pub(super) fn check_slot_filler(
        &mut self,
        root: &'a CArchetypeRoot,
        parent_slot: &'a ArchetypeSlot,
        path: &str,
    ) {
        // VARXID: the filler node id must be a specialisation of the slot id —
        // conformant and strictly deeper (`master04.5` §`C_ARCHETYPE_ROOT`, VARXID
        // L427).
        let id_ok = AdlCodeDefinitionsData::codes_conformant(&root.node_id, &parent_slot.node_id)
            && specialisation_depth(&root.node_id) > specialisation_depth(&parent_slot.node_id);
        if !id_ok {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxid,
                format!(
                    "slot filler node id {:?} is not a specialisation of the slot id {:?}",
                    root.node_id, parent_slot.node_id
                ),
                path,
            );
        }

        // VARXS: the filler archetype must satisfy the parent slot's include /
        // exclude constraints (`master04.5` §`C_ARCHETYPE_ROOT`, VARXS L424).
        let matches_slot = slot_admits(parent_slot, &root.archetype_ref);
        if !matches_slot {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxs,
                format!(
                    "slot filler {:?} does not match the parent slot constraint",
                    root.archetype_ref
                ),
                path,
            );
            return;
        }

        // VARXR: the referenced archetype must be resolvable in the repository
        // (`master04.5` §`C_ARCHETYPE_ROOT`, VARXR L419).
        if !root.archetype_ref.is_empty() && self.repo.get(&root.archetype_ref).is_none() {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxr,
                format!(
                    "external reference {:?} cannot be resolved in the repository",
                    root.archetype_ref
                ),
                path,
            );
        }
    }
}

/// True if two slot-assertion lists are structurally identical, used by VDSSM to
/// detect a restatement (not a proper narrowing) of the parent slot.
///
/// Every assertion counts, whatever its shape: the comparison key is the
/// assertion's own string form rendered from its expression tree
/// ([`crate::print::assertion_text`]), so an assertion the regex reading cannot
/// express still distinguishes the two lists.
///
/// An assertion the printer refuses has no comparison key, so the two lists are
/// reported as different: VDSSM fires only on a proven restatement, and an
/// unrenderable assertion proves nothing.
/// Whether a slot states any narrowing at all — an `includes` or `excludes`
/// assertion.
fn slot_narrows(slot: &ArchetypeSlot) -> bool {
    !slot.includes.as_ref().is_none_or(Vec::is_empty)
        || !slot.excludes.as_ref().is_none_or(Vec::is_empty)
}

fn slot_assertions_equal(a: &[Assertion], b: &[Assertion]) -> bool {
    let rendered = |list: &[Assertion]| {
        list.iter()
            .map(crate::print::assertion_text)
            .collect::<Result<Vec<String>, _>>()
    };
    let (Ok(mut as_), Ok(mut bs)) = (rendered(a), rendered(b)) else {
        return false;
    };
    as_.sort();
    bs.sort();
    as_ == bs
}

/// VDSSM widening check: if any `include` in the child slot names a **literal**
/// archetype id (a regex body with no meta-characters) that the parent slot does
/// not admit, the child admits an archetype outside the parent's set — a
/// widening, not a subset. Returns the first such literal id.
///
/// VDSSM is a proper-subset test over the archetype sets the two slot
/// definitions match (`master04.5` §Validity Rules: `ARCHETYPE_SLOT`), and a
/// slot's set is the union over its `include` assertions
/// (`ADL2/master04.3` §Archetype Slots). So each include is judged on its own:
/// one whose constraint is not a readable archetype-id regex (the
/// §Slots based on other Constraints form) contributes an unknown share and is
/// SKIPPED, while the remaining literals are still judged — an unreadable
/// assertion is undecidable, never a violation. The parent side is all-or-
/// nothing for the same reason: if any parent `include` is unreadable the
/// admitted superset is unknown, and an unknown superset can refute nothing.
fn slot_widens_by_literal(
    child_slot: &ArchetypeSlot,
    parent_slot: &ArchetypeSlot,
) -> Option<String> {
    if parent_slot
        .includes
        .iter()
        .flatten()
        .any(|a| assertion_regex(a).is_none())
    {
        return None;
    }
    for inc in child_slot.includes.iter().flatten() {
        let Some(body) = assertion_regex(inc) else {
            continue; // not an archetype-id regex — its contribution is unknown
        };
        // Unescape the ADL id-regex `\.` dots; a literal id carries no other
        // regex meta-characters.
        let literal = body.replace("\\.", ".");
        if literal.is_empty()
            || literal.contains(['*', '+', '?', '(', ')', '[', ']', '|', '^', '$'])
        {
            continue; // a genuine pattern, not a literal id — undecidable subset
        }
        if !slot_admits(parent_slot, &literal) {
            return Some(literal);
        }
    }
    None
}

/// True if the parent slot admits the archetype `id` (satisfies an `include`
/// assertion and is not caught by a specific `exclude`).
///
/// The parent slot's include/exclude assertions constrain `archetype_id/value`
/// with a regex; a filler matches if it satisfies an include regex.
fn slot_admits(slot: &ArchetypeSlot, id: &str) -> bool {
    if slot.is_closed {
        return false;
    }
    if slot.includes.as_ref().is_none_or(Vec::is_empty) {
        // An open slot with no includes admits anything not excluded.
        return !slot
            .excludes
            .iter()
            .flatten()
            .any(|a| assertion_matches(a, id));
    }
    slot.includes
        .iter()
        .flatten()
        .any(|a| assertion_matches(a, id))
        && !slot
            .excludes
            .iter()
            .flatten()
            .any(|a| assertion_specific_match(a, id))
}

/// True if a slot assertion's regex constraint matches `id`.
fn assertion_matches(a: &Assertion, id: &str) -> bool {
    let Some(re) = assertion_regex(a) else {
        return false;
    };
    regex::Regex::new(&re).is_ok_and(|rx| rx.is_match(id))
}

/// True if a slot assertion is a *specific* (non-universal) regex that matches
/// `id` (a universal `.*`/`.+` exclude does not exclude a matched include).
fn assertion_specific_match(a: &Assertion, id: &str) -> bool {
    let Some(re) = assertion_regex(a) else {
        return false;
    };
    let trimmed = re.trim();
    if trimmed == ".*" || trimmed == ".+" {
        return false;
    }
    regex::Regex::new(&re).is_ok_and(|rx| rx.is_match(id))
}

/// Extract the regex body of a slot assertion's `matches {/re/}` constraint,
/// from the assertion's expression tree.
fn assertion_regex(a: &Assertion) -> Option<String> {
    crate::rules::slot_assertion_regex(a).map(str::to_owned)
}

// ── template / external-reference fillers (VTPL + VARXR) ──────────────────

/// Validates an archetype's `use_archetype` fillers against `repo`.
///
/// The checks are VARXR (a reference that does not resolve) and — for templates
/// only — VTPL (a filler that does not support the template's
/// `original_language`).
///
/// The artefact is flattened first so inherited fillers are seen; a flatten
/// failure yields no filler issues (the flattener's own errors surface through
/// the specialisation validator).
#[must_use]
pub fn validate_fillers(archetype: &Archetype, repo: &ArchetypeRepository) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let Ok(flat) = flat_form(archetype, repo) else {
        return issues;
    };
    let v = view(&flat);
    let is_template = matches!(
        v.kind,
        ArtefactKind::Template | ArtefactKind::TemplateOverlay
    );
    let root_language = template_language(&v);
    let mut roots = Vec::new();
    collect_roots(v.definition, &mut roots);
    for root in roots {
        check_filler(
            root,
            repo,
            is_template,
            root_language.as_deref(),
            &mut issues,
        );
    }
    issues
}

/// The template's declared original language code (`master07.13`
/// `ARCHETYPE_TERMINOLOGY.original_language` / the language section), for the
/// VTPL comparison.
fn template_language(v: &ArchetypeView<'_>) -> Option<String> {
    if !v.terminology.original_language.is_empty() {
        return Some(v.terminology.original_language.clone());
    }
    v.original_language.map(|c| c.code_string.clone())
}

/// Check one filler root: VARXR if it does not resolve; VTPL (templates only) if
/// the resolved filler's flat languages do not include `root_language`.
fn check_filler(
    root: &CArchetypeRoot,
    repo: &ArchetypeRepository,
    is_template: bool,
    root_language: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if root.archetype_ref.is_empty() {
        return;
    }
    let Some(constituent) = repo.get(&root.archetype_ref) else {
        issues.push(
            ValidationIssue::new(
                ValidationCode::Varxr,
                format!(
                    "external reference {:?} does not resolve to an archetype in the repository",
                    root.archetype_ref
                ),
            )
            .at_path(root.node_id.clone()),
        );
        return;
    };
    if !is_template {
        return;
    }
    let Some(lang) = root_language else {
        return;
    };
    // The filler must support the template's language in its flat terminology
    // (VTPL). Flatten the filler so inherited translations are counted.
    let Ok(flat_filler) = flat_form(constituent, repo) else {
        return;
    };
    if !filler_supports_language(&view(&flat_filler), lang) {
        issues.push(
            ValidationIssue::new(
                ValidationCode::Vtpl,
                format!(
                    "slot filler {:?} does not support the template language {lang:?}",
                    root.archetype_ref
                ),
            )
            .at_path(root.node_id.clone()),
        );
    }
}

/// True if a filler's flat terminology carries `lang` (as its original
/// language, a translation, or a `term_definitions` language bucket).
fn filler_supports_language(v: &ArchetypeView<'_>, lang: &str) -> bool {
    if v.terminology.original_language == lang {
        return true;
    }
    if v.terminology.term_definitions.contains_key(lang) {
        return true;
    }
    v.original_language.is_some_and(|c| c.code_string == lang)
        || v.translations.is_some_and(|t| t.contains_key(lang))
}

/// Collect every `C_ARCHETYPE_ROOT` in the definition tree (the fillers /
/// external references).
fn collect_roots<'a>(def: &'a CComplexObject, out: &mut Vec<&'a CArchetypeRoot>) {
    match def {
        CComplexObject::CComplexObject(d) => {
            for attr in d.attributes.iter().flatten() {
                collect_roots_attr(attr, out);
            }
        }
        CComplexObject::CArchetypeRoot(r) => {
            out.push(r);
            for attr in r.attributes.iter().flatten() {
                collect_roots_attr(attr, out);
            }
        }
    }
}

fn collect_roots_attr<'a>(attr: &'a CAttribute, out: &mut Vec<&'a CArchetypeRoot>) {
    for child in attr.children.iter().flatten() {
        if let CObject::CComplexObject(cco) = child {
            collect_roots(cco, out);
        }
    }
}
