// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ARCHETYPE_SLOT` redefinition and filling.
//!
//! `AOM2/master04.5` §Validity Rules: `ARCHETYPE_SLOT`, `C_ARCHETYPE_ROOT`; and
//! the repository-dependent filler checks of [`validate_fillers`]: VTPL for
//! templates (`master03`; `OPT2/master03` §Terminology) and VARXR (`master08`
//! §Phase 2), over the flattened artefact.

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
    /// §`ARCHETYPE_SLOT`): VDSSID, VDSSP and VDSSC.
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
        if !root.archetype_ref.is_empty()
            && self
                .repo
                .resolve(&root.archetype_ref, self.referrer_namespace)
                .is_none()
        {
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

/// Whether a slot states any narrowing, an `includes` or `excludes` assertion.
fn slot_narrows(slot: &ArchetypeSlot) -> bool {
    !slot.includes.as_ref().is_none_or(Vec::is_empty)
        || !slot.excludes.as_ref().is_none_or(Vec::is_empty)
}

/// Whether two slot-assertion lists are identical by their printed forms
/// ([`crate::print::assertion_text`]); an unprintable assertion makes them differ.
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

/// The first literal archetype id a child `include` names that the parent slot
/// does not admit, a VDSSM widening (`master04.5` §Validity Rules:
/// `ARCHETYPE_SLOT`; `ADL2/master04.3` §Archetype Slots).
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
/// VARXR, and VTPL for templates, whose `template_overlay` sections are local
/// constituents (`ADL2/master10-templates.adoc`). A flatten failure yields none.
#[must_use]
pub fn validate_fillers(archetype: &Archetype, repo: &ArchetypeRepository) -> Vec<ValidationIssue> {
    let overlays = crate::opt::root_overlays(archetype);
    let seeded;
    let repo = if overlays.is_empty() {
        repo
    } else {
        let mut work = repo.clone();
        for overlay in &overlays {
            work.insert(Archetype::TemplateOverlay(Box::new(overlay.clone())));
        }
        seeded = work;
        &seeded
    };
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
    check_fillers_of(&v, repo, is_template, root_language.as_deref(), &mut issues);
    for overlay in overlays {
        let overlay = Archetype::TemplateOverlay(Box::new(overlay));
        if let Ok(flat_overlay) = flat_form(&overlay, repo) {
            check_fillers_of(
                &view(&flat_overlay),
                repo,
                is_template,
                root_language.as_deref(),
                &mut issues,
            );
        }
    }
    issues
}

/// Check every filler root of one flat artefact.
fn check_fillers_of(
    v: &ArchetypeView<'_>,
    repo: &ArchetypeRepository,
    is_template: bool,
    root_language: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut roots = Vec::new();
    collect_roots(v.definition, &mut roots);
    for root in roots {
        let namespace = v.archetype_id.namespace.as_deref();
        check_filler(root, repo, namespace, is_template, root_language, issues);
    }
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
    referrer_namespace: Option<&str>,
    is_template: bool,
    root_language: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if root.archetype_ref.is_empty() {
        return;
    }
    let Some(constituent) = repo.resolve(&root.archetype_ref, referrer_namespace) else {
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
pub(crate) fn collect_roots<'a>(def: &'a CComplexObject, out: &mut Vec<&'a CArchetypeRoot>) {
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
