// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Terminology-side artefact validity for the OPT 1.4 pass: the codes,
//! bindings and language-consistency rules of the AOM2 terminology package.
//!
//! Rules enforced (`AOM2/master07-terminology_package.adoc`,
//! `AOM2/master08-validation.adoc`):
//! - **VATID** — every at-code used as a `node_id` in the definition is
//!   defined in terminology (master08 line 56).
//! - **VTTBK** — every term-binding key is a defined archetype term or a path
//!   (master07 line 77).
//! - **VTCBK** — every constraint-binding key is a defined ac-code
//!   (master07 line 80).
//! - **VTLC** — all term/constraint codes exist in all languages
//!   (master07 line 74).
//!
//! This module also owns the global code-collection helpers and the flattened
//! ontology / nested-root accessors the whole pass shares.

use std::collections::HashSet;

use openehr_its::opt14::types::{
    CArchetypeRoot, CAttribute, CObject, Codedefinitionset, FlatArchetypeOntology,
    OperationalTemplate,
};
use openehr_rm::v1_2::paths::archetype_node_id_is_term_code;

use super::{NodeView, RuleViolation, attribute_children};

// ─── VATID (node-id codes defined in terminology) ───────────────────────────────

/// VATID: "check that all codes mentioned in `definition` are defined in
/// terminology" (`AOM2/master08-validation.adoc` line 56). Applied to at-code
/// `node_id`s (the addressable, sibling-identifying codes, AOM14/04 §`Node_id`).
/// What counts as a term code is the RM's own reading of `archetype_node_id`
/// ([`archetype_node_id_is_term_code`] — an `at`/`id` leader followed by
/// `.`-separated numeric segments), so this pass and the RM path layer can
/// never disagree about which node ids are codes. Empty `node_ids`
/// (non-addressable leaves), archetype-root ids, and free text are exempt: VATID
/// constrains *codes*, and a string outside the code grammar is not one.
/// The defined-code set is collected globally across the flattened OPT (the
/// definition roots + every `component_ontologies` set), which is deliberately
/// lenient about per-archetype scoping — it still catches a `node_id` that is
/// defined nowhere while never mis-rejecting a correctly-scoped code.
pub(super) fn check_node_id(
    node_id: &str,
    defined_at: &HashSet<String>,
) -> Result<(), RuleViolation> {
    if !archetype_node_id_is_term_code(node_id) {
        // A node id CARRYING the at/id leader but failing the code-body grammar
        // (`at0abc`) is a malformed CLAIMED code, not free text: AOM2's own
        // predicate is leader-based (`adl_code_definitions.adoc` §is_at_code),
        // so the string claims code-hood and its body must then satisfy the code
        // syntax.
        let claims_code_leader = (node_id.starts_with("at") || node_id.starts_with("id"))
            && node_id.len() > 2
            && !openehr_rm::v1_2::paths::is_archetype_root_node_id(node_id);
        if claims_code_leader {
            return Err(RuleViolation::new(
                "VATID",
                format!(
                    "node_id '{node_id}' carries the at/id code leader but is not a \
                     well-formed archetype local code (leader + '.'-separated numeric \
                     segments) — a malformed claimed code, not free text"
                ),
            ));
        }
        return Ok(());
    }
    if !defined_at.contains(node_id) {
        return Err(RuleViolation::new(
            "VATID",
            format!("node_id '{node_id}' is used in the definition but not defined in terminology"),
        ));
    }
    Ok(())
}

// ─── VTTBK / VTCBK (binding key validity) ───────────────────────────────────────

/// VTTBK: "terminology term binding key valid. Every term binding must be to
/// either a defined archetype term ('at-code') or to a path that is valid in the
/// flat archetype." (`AOM2/master07-terminology_package.adoc` line 77.) A `/`
/// path key is accepted without full path resolution (conservative — never
/// mis-reject a real flat path).
pub(super) fn check_term_bindings(
    opt: &OperationalTemplate,
    defined_at: &HashSet<String>,
) -> Result<(), RuleViolation> {
    for set in &opt.definition.term_bindings {
        for item in &set.items {
            check_term_binding_key(&item.code, defined_at)?;
        }
    }
    for onto in flat_ontologies(opt) {
        for set in &onto.term_bindings {
            for item in &set.items {
                check_term_binding_key(&item.code, defined_at)?;
            }
        }
    }
    // Nested C_ARCHETYPE_ROOTs carry their own term_bindings.
    for root in nested_roots(opt) {
        for set in &root.term_bindings {
            for item in &set.items {
                check_term_binding_key(&item.code, defined_at)?;
            }
        }
    }
    Ok(())
}

/// One VTTBK key: a path, or a defined archetype term.
///
/// NOTE (flattened-OPT tolerance): a *specialised* code (`at0.23`, AOM2
/// §specialisation depth) names a code DEFINED IN THE PARENT archetype, which
/// the released AM text never requires to be repeated locally. (The shape is
/// observed in the vendored blood-pressure corpus OPTs — evidence that it
/// occurs, never the authority for accepting it.)
fn check_term_binding_key(code: &str, defined_at: &HashSet<String>) -> Result<(), RuleViolation> {
    let specialised = code.contains('.');
    if code.starts_with('/')
        || !archetype_node_id_is_term_code(code)
        || specialised
        || defined_at.contains(code)
    {
        return Ok(());
    }
    Err(RuleViolation::new(
        "VTTBK",
        format!(
            "term binding key '{code}' is neither a defined archetype term (at-code) nor a path"
        ),
    ))
}

/// VTCBK: "terminology constraint binding key valid. Every constraint binding
/// must be to a defined archetype constraint code ('ac-code')."
/// (`AOM2/master07-terminology_package.adoc` line 80.)
pub(super) fn check_constraint_bindings(
    opt: &OperationalTemplate,
    defined_ac: &HashSet<String>,
) -> Result<(), RuleViolation> {
    for onto in flat_ontologies(opt) {
        for set in &onto.constraint_bindings {
            for item in &set.items {
                if !defined_ac.contains(&item.code) {
                    return Err(RuleViolation::new(
                        "VTCBK",
                        format!(
                            "constraint binding key '{}' is not a defined archetype constraint \
                             code (ac-code)",
                            item.code
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

// ─── VTLC (language consistency) ────────────────────────────────────────────────

/// VTLC: "language consistency. Languages consistent: all term codes and
/// constraint codes exist in all languages."
/// (`AOM2/master07-terminology_package.adoc` line 74.) Applied to each
/// `FlatArchetypeOntology` whose `term_definitions` / `constraint_definitions`
/// are grouped per language: every language must define the same code set.
///
/// NOTE: the per-`C_ARCHETYPE_ROOT` `term_definitions` (a flat
/// `Vec<ARCHETYPE_TERM>`, single-language) carry no language grouping, so VTLC
/// is inert for a single-language OPT — the multi-language code sets live only
/// in `ontology` / `component_ontologies`.
pub(super) fn check_language_consistency(opt: &OperationalTemplate) -> Result<(), RuleViolation> {
    for onto in flat_ontologies(opt) {
        language_consistent(&codes_by_language(&onto.term_definitions), "term")?;
        language_consistent(
            &codes_by_language(&onto.constraint_definitions),
            "constraint",
        )?;
    }
    Ok(())
}

fn codes_by_language(sets: &[Codedefinitionset]) -> Vec<(String, HashSet<String>)> {
    sets.iter()
        .map(|s| {
            (
                s.language.clone(),
                s.items.iter().map(|t| t.code.clone()).collect(),
            )
        })
        .collect()
}

fn language_consistent(
    by_lang: &[(String, HashSet<String>)],
    kind: &str,
) -> Result<(), RuleViolation> {
    if by_lang.len() < 2 {
        return Ok(());
    }
    let Some((ref_lang, ref_codes)) = by_lang.first() else {
        return Ok(());
    };
    for (lang, codes) in by_lang.iter().skip(1) {
        if codes != ref_codes {
            let missing: Vec<&str> = ref_codes
                .symmetric_difference(codes)
                .map(String::as_str)
                .collect();
            return Err(RuleViolation::new(
                "VTLC",
                format!(
                    "the {kind} code set differs between languages '{ref_lang}' and '{lang}' \
                     (e.g. {missing:?}); all codes must exist in all languages"
                ),
            ));
        }
    }
    Ok(())
}

// ─── code collection + ontology / nested-root accessors ─────────────────────────

/// Every archetype term (`at`/`id`) code defined anywhere in the flattened OPT.
pub(super) fn collect_defined_at_codes(opt: &OperationalTemplate) -> HashSet<String> {
    let mut out = HashSet::new();
    out.extend(
        opt.definition
            .term_definitions
            .iter()
            .map(|t| t.code.clone()),
    );
    for root in nested_roots(opt) {
        out.extend(root.term_definitions.iter().map(|t| t.code.clone()));
    }
    for onto in flat_ontologies(opt) {
        for set in &onto.term_definitions {
            out.extend(set.items.iter().map(|t| t.code.clone()));
        }
    }
    out
}

/// Every archetype constraint (`ac`) code defined in the flattened OPT.
pub(super) fn collect_defined_ac_codes(opt: &OperationalTemplate) -> HashSet<String> {
    let mut out = HashSet::new();
    for onto in flat_ontologies(opt) {
        for set in &onto.constraint_definitions {
            out.extend(set.items.iter().map(|t| t.code.clone()));
        }
    }
    out
}

/// `ontology` + every `component_ontologies` entry.
pub(super) fn flat_ontologies(opt: &OperationalTemplate) -> Vec<&FlatArchetypeOntology> {
    opt.ontology
        .iter()
        .chain(opt.component_ontologies.iter())
        .collect()
}

/// Every nested `C_ARCHETYPE_ROOT` under the definition (the flattened slot
/// fillers), excluding the root definition itself.
fn nested_roots(opt: &OperationalTemplate) -> Vec<&CArchetypeRoot> {
    let mut out = Vec::new();
    for attr in &opt.definition.attributes {
        collect_roots_in_attr(attr, &mut out);
    }
    out
}

fn collect_roots_in_attr<'a>(attr: &'a CAttribute, out: &mut Vec<&'a CArchetypeRoot>) {
    for child in attribute_children(attr) {
        if let CObject::CArchetypeRoot(root) = child {
            out.push(root);
            for a in &root.attributes {
                collect_roots_in_attr(a, out);
            }
        } else {
            for a in NodeView::of(child).attributes {
                collect_roots_in_attr(a, out);
            }
        }
    }
}
