// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 identification / header topic: the archetype identifier, the
//! definition root typename and concept code, the ADL/RM version metadata, the
//! original language + description, the resource-description language
//! consistency, and the terminology-structure gate the code checks hang off.
//!
//! Rule texts:
//! `docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc` §Validity
//! Rules (VARID, VARDT, VARCN, VARAV, VARRV, VDEOL, VARD) with the
//! orchestration in `master08-validation.adoc` §Phase 1 - Basic Integrity
//! (STCNT, VOLT); the ADL 1.4 concept-term half is
//! `ADL1.4/master08-adl.adoc` §Validity Rules VARCN.

use std::collections::BTreeSet;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use super::specialisation::{check_language_conformance, check_specialisation_depth};
use crate::aom::access::{complex_node_id, complex_rm_type};
use crate::artefact::{ArchetypeRepository, ArchetypeView};
use crate::codes::is_root_code_at_depth;
use crate::parse::Dialect;
use crate::source::SourceArtefact;

pub(super) fn check_identification(
    v: &ArchetypeView<'_>,
    repo: Option<&ArchetypeRepository>,
    dialect: Dialect,
    out: &mut Vec<ValidationIssue>,
) {
    let h = v.archetype_id;
    let is_overlay = v.kind == crate::source::ArtefactKind::TemplateOverlay;

    // VARID: archetype identifier validity (master03 §Validity Rules) — applies
    // to all except TEMPLATE_OVERLAY (G2). The id must have all mandatory parts.
    if !is_overlay
        && (h.rm_publisher.is_empty()
            || h.rm_package.is_empty()
            || h.rm_class.is_empty()
            || h.concept_id.is_empty()
            || !is_three_part_version(&h.release_version))
    {
        out.push(ValidationIssue::new(
            ValidationCode::Varid,
            format!(
                "archetype identifier is not well-formed: publisher={:?} package={:?} class={:?} concept={:?} version={:?}",
                h.rm_publisher, h.rm_package, h.rm_class, h.concept_id, h.release_version
            ),
        ));
    }

    // VARDT: definition typename matches the RM class of the identifier
    // (master03 §Validity Rules).
    let root_rm = complex_rm_type(v.definition);
    if !root_rm.is_empty() && !h.rm_class.is_empty() && root_rm != h.rm_class {
        out.push(ValidationIssue::new(
            ValidationCode::Vardt,
            format!(
                "definition root type {root_rm:?} does not match the identifier RM class {:?}",
                h.rm_class
            ),
        ));
    }

    // VARCN: root concept code form for the specialisation level (master03
    // §Validity Rules — the FORM half; terminology-definedness is checked by
    // VATID, per the master08 grouping and the corpus `regression` oracle).
    let root_id = complex_node_id(v.definition);
    if !root_id.is_empty() && !is_root_code_at_depth(root_id, v.specialisation_level()) {
        out.push(ValidationIssue::new(
            ValidationCode::Varcn,
            format!(
                "root node id {root_id:?} is not a valid root code at specialisation depth {}",
                v.specialisation_level()
            ),
        ));
    }

    // VARAV / VARRV: adl_version / rm_release 3-part version formats (master03
    // §Validity Rules). AOM2-only: an ADL 1.4 artefact carries a `1.4`-form
    // `adl_version` (two-part, optional metadata) and NO `rm_release`, and AOM
    // 1.4 defines no 3-part-version rule for either (ADL1.4 master08 §Syntax
    // Specification, `arch_identification` meta-data; AOM1.4 master03 ARCHETYPE
    // §Invariants — version validity is only `version = archetype_id.version_id`).
    // So both are suppressed in the 1.4 dialect: applying the AOM2 rule would
    // reject every valid 1.4 archetype.
    if !is_overlay && dialect == Dialect::Adl2 {
        check_version_formats(v, out);
    }
    if !is_overlay {
        // VDEOL / VARD: original language + description present (master03
        // §Validity Rules, G2).
        if v.original_language.is_none() {
            out.push(ValidationIssue::new(
                ValidationCode::Vdeol,
                "no original_language section",
            ));
        }
        if v.description.is_none() {
            out.push(ValidationIssue::new(
                ValidationCode::Vard,
                "no description section",
            ));
        }
    }

    // VACSD / VASID: specialisation depth + parent id (master03 §Validity Rules,
    // G3). Standalone half for non-specialised; parent-dependent half via `repo`.
    check_specialisation_depth(v, repo, out);

    // VALC: language conformance to the flat parent (master03 §Validity Rules,
    // G3) — needs the parent; runs only when resolvable.
    check_language_conformance(v, repo, out);
}

/// VARAV / VARRV: the `adl_version` / `rm_release` 3-part version formats
/// (master03 §Validity Rules).
///
/// AOM2-only: an ADL 1.4 artefact carries a `1.4`-form `adl_version`
/// (two-part, optional metadata) and NO `rm_release`, and AOM 1.4 defines no
/// 3-part-version rule for either (ADL1.4 master08 §Syntax Specification,
/// `arch_identification` meta-data; AOM1.4 master03 ARCHETYPE §Invariants —
/// version validity is only `version = archetype_id.version_id`). So the
/// caller suppresses both in the 1.4 dialect: applying the AOM2 rule would
/// reject every valid 1.4 archetype.
fn check_version_formats(v: &ArchetypeView<'_>, out: &mut Vec<ValidationIssue>) {
    if !v.adl_version.is_some_and(is_three_part_version) {
        out.push(ValidationIssue::new(
            ValidationCode::Varav,
            format!(
                "adl_version {:?} is not a valid 3-part version",
                v.adl_version
            ),
        ));
    }
    if !is_three_part_version(v.rm_release) {
        out.push(ValidationIssue::new(
            ValidationCode::Varrv,
            format!(
                "rm_release {:?} is not a valid 3-part version",
                v.rm_release
            ),
        ));
    }
}

/// VARCN (ADL 1.4, terminology half): the `concept` section's term must exist
/// in the archetype ontology — `ADL1.4/master08-adl.adoc` §Validity Rules
/// VARCN: "archetype concept validity. The archetype must have an archetype
/// term value in the concept section. The term must exist in the archetype
/// ontology."
///
/// The FORM half (a root code well-formed for the specialisation depth) is
/// checked for every dialect in [`check_identification`]; this half needs the
/// parsed 1.4 `concept` clause, which only the source-level entry points carry
/// (ADL2 has no concept section — `ADL2/master07.09`). A concept term is judged
/// defined when any language's `term_definitions` bucket carries it, matching
/// the definedness union [`check_terminology`](super::terminology::check_terminology)
/// uses. An ABSENT concept section
/// is refused earlier, at assembly ([`crate::error::SyntaxErrorCode::Saco`]).
pub(super) fn check_concept_term_adl14(
    v: &ArchetypeView<'_>,
    source: &SourceArtefact,
    out: &mut Vec<ValidationIssue>,
) {
    let Some(code) = source.concept.as_deref() else {
        return;
    };
    let defined = v
        .terminology
        .term_definitions
        .values()
        .any(|bucket| bucket.contains_key(code));
    if !defined {
        out.push(ValidationIssue::new(
            ValidationCode::Varcn,
            format!("concept term {code:?} is not defined in the archetype ontology"),
        ));
    }
}

/// VRDLA: resource-description language-code consistency — the `language` code
/// declared inside a `description.details` / `translations` block must match
/// the block's own language key (e.g. a `["zh-cn"]` block whose inner
/// `language` is `zh` is inconsistent).
///
/// NOTE: no openEHR spec governs this — our own design/extension; the code
/// `VRDLA` appears nowhere in the vendored AOM2 text, and the rule was
/// adjudicated from the `validity/basics` corpus cases.
pub(super) fn check_resource_description_languages(
    v: &ArchetypeView<'_>,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(desc) = v.description
        && let Some(details) = desc.details.as_ref()
    {
        for (key, item) in details {
            if !item.language.code_string.is_empty() && item.language.code_string != *key {
                issues.push(ValidationIssue::new(
                    ValidationCode::Vrdla,
                    format!(
                        "description details block {key:?} declares inconsistent language {:?}",
                        item.language.code_string
                    ),
                ));
            }
        }
    }
    if let Some(translations) = v.translations {
        for (key, tr) in translations {
            if !tr.language.code_string.is_empty() && tr.language.code_string != *key {
                issues.push(ValidationIssue::new(
                    ValidationCode::Vrdla,
                    format!(
                        "translation block {key:?} declares inconsistent language {:?}",
                        tr.language.code_string
                    ),
                ));
            }
        }
    }
}

// ── the terminology-structure gate (STCNT / VOLT) ─────────────────────────

#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) enum TermStructure {
    /// The `terminology` section defines no `term_definitions` at all (STCNT).
    Empty,
    /// The original language has no `term_definitions` bucket (VOLT).
    MissingOriginalLanguage,
    /// The terminology structure is sound; the code checks may run.
    Ok,
}

pub(super) fn terminology_structure(v: &ArchetypeView<'_>) -> TermStructure {
    let term = v.terminology;
    if term.term_definitions.is_empty() {
        return TermStructure::Empty;
    }
    if !term.term_definitions.contains_key(&original_language(v)) {
        return TermStructure::MissingOriginalLanguage;
    }
    TermStructure::Ok
}

pub(super) fn original_language(v: &ArchetypeView<'_>) -> String {
    if v.terminology.original_language.is_empty() {
        v.original_language
            .map(|c| c.code_string.clone())
            .unwrap_or_default()
    } else {
        v.terminology.original_language.clone()
    }
}

/// The set of languages the archetype declares (original + translations +
/// description details).
pub(super) fn languages(v: &ArchetypeView<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(c) = v.original_language {
        out.insert(c.code_string.clone());
    }
    if let Some(t) = v.translations {
        out.extend(t.keys().cloned());
    }
    out
}

/// `is_three_part_version` — a `major.minor.patch` version (with an optional
/// pre-release suffix), per the archetype identification version rule.
fn is_three_part_version(s: &str) -> bool {
    let core = s.split('-').next().unwrap_or(s);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
