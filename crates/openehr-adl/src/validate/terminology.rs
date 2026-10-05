// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 terminology topic: the archetype's own `term_definitions`,
//! `constraint_definitions` and `value_sets`, and the codes the definition
//! references against them — definedness, specialisation level, code form,
//! language coverage, value-set integrity and the unused-code warning.
//!
//! Rule texts:
//! `docs/specs/openehr/AM/docs/AOM2/master07-terminology_package.adoc`
//! §Validity Rules (VTSD, VTLC, VTVSID, VTVSMD, VTVSUQ),
//! `master03-archetype_package.adoc` §Validity Rules (VATDF, VACDF, VATDA,
//! VATCD, VOTM) and `master08-validation.adoc` §Phase 1 - Basic Integrity
//! (VATID, VATCV). WOUC appears nowhere in the vendored spec text — our own
//! design/extension, flagged at its check site.
//!
//! The pass is gated by `super::run`: it runs only when the basic
//! identification checks are clean and the terminology structure is sound
//! (`master08` §Overview, "more basic kinds of errors being checked first" — a
//! code cannot be judged against a missing or inconsistent terminology).

use std::collections::BTreeSet;

use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;

use super::ValidationIssue;
use super::bindings::check_bindings;
use super::catalogue::ValidationCode;
use super::identification::languages;
use super::structure::complex_attribute_tuples;
use crate::aom::access::{aom_type, complex_attributes, complex_node_id, object_node_id};
use crate::artefact::ArchetypeView;
use crate::codes;
use crate::parse::Dialect;
use crate::paths::child_path;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

pub(super) fn check_terminology(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    issues: &mut Vec<ValidationIssue>,
) {
    let term = v.terminology;

    // Union of all defined codes across languages (a code defined in any
    // language counts as defined for the definedness checks).
    let defined: BTreeSet<&str> = term
        .term_definitions
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();

    // Re-scan the definition for referenced codes (usage + assumed values).
    let mut usage = CodeUsage::default();
    let root = CObject::CComplexObject(v.definition.clone());
    collect_usage(&root, &mut usage);

    check_root_concept_code(v, &defined, issues);
    check_node_id_definedness(v, dialect, &usage, &defined, issues);
    check_referenced_codes(v, &usage, &defined, issues);
    check_assumed_values(term, &usage, issues);
    check_defined_code_levels(v, dialect, &defined, issues);
    check_defined_code_forms(&defined, issues);
    // VTLC: every code defined in one language must be defined in all languages
    // (master07 §Validity Rules).
    check_language_coverage(term, issues);
    check_declared_languages(v, term, issues);
    // VTVSID / VTVSMD / VTVSUQ: value-set integrity (master07 §Validity Rules).
    check_value_sets(term, &defined, !v.is_specialised(), issues);
    // VTTBK / VTCBK: term/constraint binding key validity (master07 §Validity
    // Rules).
    check_bindings(v, &defined, issues);
    check_unused_codes(v, dialect, &usage, &defined, issues);
}

/// VATID: the root concept code must be defined in the terminology (master08
/// §Code Validation; NOTE-flagged, no full vendored text).
///
/// NOTE: the per-node id-code definedness half is a reference-model check
/// (master07 §Overview: a term definition is optional for children of a
/// single-valued attribute), so it runs in [`super::rm`].
fn check_root_concept_code(
    v: &ArchetypeView<'_>,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let root_id = complex_node_id(v.definition);
    if !root_id.is_empty()
        && (AdlCodeDefinitionsData::is_id_code(root_id)
            || AdlCodeDefinitionsData::is_at_code(root_id))
        && !defined.contains(root_id)
    {
        issues.push(ValidationIssue::new(
            ValidationCode::Vatid,
            format!("root concept code {root_id:?} is not defined in the terminology"),
        ));
    }
}

/// VATDF (ADL 1.4, node-id half): in ADL 1.4 EVERY at-code used as a node
/// identifier in the definition must be defined in the ontology's
/// `term_definitions` (ADL1.4 master08 §Validity Rules VATDF; AOM1.4
/// `ARCHETYPE.node_ids_valid`).
///
/// ADL2 defers interior-node-id definedness to the RM phase, but the 1.4
/// formalism has no such optionality for a code that IS present — "each
/// archetype term used as a node identifier … must be defined". A
/// non-specialised 1.4 archetype is its own flat form, so the phase-1 subset
/// closes VATDF's interior half for a 1.4 upload.
fn check_node_id_definedness(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    usage: &CodeUsage,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if dialect != Dialect::Adl14 || v.is_specialised() {
        return;
    }
    for code in &usage.node_codes {
        if AdlCodeDefinitionsData::is_at_code(code) && !defined.contains(code.as_str()) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vatdf,
                format!("node identifier code {code:?} is not defined in the terminology"),
            ));
        }
    }
}

/// The definedness and level rules over the codes the definition references.
///
/// VATDF: at-codes used in term constraints defined in the terminology of the
/// flattened form (master03 §Validity Rules). For a specialised archetype the
/// flat form is not available here, so this runs only when the archetype is
/// its own flat form (non-specialised); the specialised flat-form half runs in
/// [`super::flat`]. VACDF: ac-codes defined in the current archetype (master03
/// — "current", not flattened; runs for all). VATCD: at/id codes at a level
/// greater than the archetype level.
fn check_referenced_codes(
    v: &ArchetypeView<'_>,
    usage: &CodeUsage,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let level = v.specialisation_level();
    let flat_self = !v.is_specialised();
    for code in &usage.value_codes {
        if AdlCodeDefinitionsData::is_at_code(code) {
            if flat_self && !defined.contains(code.as_str()) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Vatdf,
                    format!("value code {code:?} is not defined in the terminology"),
                ));
            }
        } else if AdlCodeDefinitionsData::is_value_set_code(code)
            && !defined.contains(code.as_str())
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Vacdf,
                format!("constraint code {code:?} is not defined in the terminology"),
            ));
        }
        if !AdlCodeDefinitionsData::is_value_set_code(code)
            && let Some(d) = codes::specialisation_depth(code)
            && d > level
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Vatcd,
                format!("code {code:?} has specialisation level {d} > archetype level {level}"),
            ));
        }
    }
}

/// VATDA: an assumed value at-code must be a member of the referenced value
/// set (master03 §Validity Rules).
fn check_assumed_values(
    term: &openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology,
    usage: &CodeUsage,
    issues: &mut Vec<ValidationIssue>,
) {
    for (path, ac, assumed) in &usage.assumed_refs {
        let members = term
            .value_sets
            .as_ref()
            .and_then(|vs| vs.get(ac))
            .is_some_and(|vs| vs.members.iter().any(|m| m == assumed));
        if !members {
            issues.push(
                ValidationIssue::new(
                    ValidationCode::Vatda,
                    format!("assumed value {assumed:?} is not a member of value set {ac:?}"),
                )
                .at_path(path.clone()),
            );
        }
    }
}

/// VTSD: every defined term/constraint code is at the archetype's
/// specialisation level (differential) or the same-or-less (flat) (master07
/// §Validity Rules).
///
/// ac-codes are a flat code space (master07 §Specialisation Depth), so only an
/// over-level ac-code is invalid, never the strict differential-equality test.
/// A 1.4 specialised archetype is a FLAT artefact (its ontology legitimately
/// carries inherited codes at lower levels alongside the level-N additions),
/// even though the 1.4-shaped model is marked `is_differential` for the
/// converter's re-differentiation pass — so the 1.4 dialect always uses the
/// flat-form rule (`d <= level`), never the differential `d == level` (AOM1.4
/// master07 §Specialisation Depth).
fn check_defined_code_levels(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let level = v.specialisation_level();
    let differential = v.is_differential && dialect == Dialect::Adl2;
    for code in defined {
        let Some(d) = codes::specialisation_depth(code) else {
            continue;
        };
        let bad = if AdlCodeDefinitionsData::is_value_set_code(code) {
            d > level
        } else if differential {
            d != level
        } else {
            d > level
        };
        if bad {
            issues.push(ValidationIssue::new(
                ValidationCode::Vtsd,
                format!(
                    "terminology code {code:?} specialisation level {d} is invalid for archetype level {level}"
                ),
            ));
        }
    }
}

/// VATCV (defined-code form): every defined code must be a valid code form
/// (master08 §Code Validation).
///
/// Value-code form on definition-referenced codes is covered in the walk.
fn check_defined_code_forms(defined: &BTreeSet<&str>, issues: &mut Vec<ValidationIssue>) {
    for code in defined {
        if !AdlCodeDefinitionsData::is_valid_code(code) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vatcv,
                format!("terminology code {code:?} is not a valid code form"),
            ));
        }
    }
}

/// VOTM: every language declared in description/translations must have
/// `term_definitions` (master03 §Validity Rules).
fn check_declared_languages(
    v: &ArchetypeView<'_>,
    term: &openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology,
    issues: &mut Vec<ValidationIssue>,
) {
    for l in languages(v) {
        if !term.term_definitions.contains_key(&l) {
            issues.push(ValidationIssue::new(
                ValidationCode::Votm,
                format!("language {l:?} has no term_definitions"),
            ));
        }
    }
}

/// WOUC: a defined at/ac code never used in the definition.
///
/// NOTE: no openEHR spec governs WOUC — our own design/extension. It is
/// suppressed in the 1.4 dialect: 1.4 value codes are carried inside the
/// verbatim terminology-constraint strings (not recognised as ADL2 code
/// usage), so the "unused" heuristic is unreliable on a 1.4-shaped model and
/// would flag legitimately-used codes.
fn check_unused_codes(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    usage: &CodeUsage,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if dialect != Dialect::Adl2 {
        return;
    }
    let mut used_all: BTreeSet<&str> = usage.value_codes.iter().map(String::as_str).collect();
    used_all.extend(usage.node_codes.iter().map(String::as_str));
    // value-set membership also counts as "use" of a member at-code.
    if let Some(vs) = v.terminology.value_sets.as_ref() {
        for set in vs.values() {
            used_all.insert(set.id.as_str());
            for m in &set.members {
                used_all.insert(m.as_str());
            }
        }
    }
    for code in defined {
        // The root concept code and id-code node ids are structural, not
        // "unused" terms; WOUC targets value at-codes and ac-codes.
        if (AdlCodeDefinitionsData::is_at_code(code)
            || AdlCodeDefinitionsData::is_value_set_code(code))
            && *code != complex_node_id(v.definition)
            && !used_all.contains(code)
        {
            issues.push(ValidationIssue::new(
                ValidationCode::Wouc,
                format!("terminology code {code:?} is defined but unused in the definition"),
            ));
        }
    }
}

fn check_language_coverage(
    term: &openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology,
    issues: &mut Vec<ValidationIssue>,
) {
    let langs: Vec<&String> = term.term_definitions.keys().collect();
    if langs.len() < 2 {
        return;
    }
    let all_codes: BTreeSet<&str> = term
        .term_definitions
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    for (lang, codes) in &term.term_definitions {
        for code in &all_codes {
            if !codes.contains_key(*code) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Vtlc,
                    format!("code {code:?} is missing in language {lang:?}"),
                ));
            }
        }
    }
}

fn check_value_sets(
    term: &openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology,
    defined: &BTreeSet<&str>,
    flat_self: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(vs) = term.value_sets.as_ref() else {
        return;
    };
    for set in vs.values() {
        check_value_set(set, defined, flat_self, issues);
    }
}

/// One value set's integrity rules (master07 §Validity Rules).
///
/// VTVSID: the id must be defined in the terminology of the current archetype
/// ("current", runs for all). VTVSUQ: members must be unique within the set.
/// VTVSMD: members must be defined in the terminology of the *flattened* form,
/// which runs only when the archetype is its own flat form — the specialised
/// flat-form half runs in [`super::flat`].
fn check_value_set(
    set: &openehr_am::v2_4::aom2::terminology::value_set::ValueSet,
    defined: &BTreeSet<&str>,
    flat_self: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    if !defined.contains(set.id.as_str()) {
        issues.push(ValidationIssue::new(
            ValidationCode::Vtvsid,
            format!(
                "value set id {:?} is not defined in the terminology",
                set.id
            ),
        ));
    }
    let mut seen = BTreeSet::new();
    for m in &set.members {
        if !seen.insert(m.as_str()) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vtvsuq,
                format!("value set {:?} has a duplicate member {m:?}", set.id),
            ));
        }
    }
    if !flat_self {
        return;
    }
    for m in &set.members {
        if !defined.contains(m.as_str()) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vtvsmd,
                format!(
                    "value set {:?} member {m:?} is not defined in the terminology",
                    set.id
                ),
            ));
        }
    }
}

// ── code-usage collector (second pass for the terminology checks) ──────────

#[derive(Default)]
pub(super) struct CodeUsage {
    pub(super) value_codes: BTreeSet<String>,
    pub(super) node_codes: BTreeSet<String>,
    pub(super) assumed_refs: Vec<(String, String, String)>,
}

pub(super) fn collect_usage(obj: &CObject, usage: &mut CodeUsage) {
    collect_usage_at(obj, "", usage);
}

fn collect_usage_at(obj: &CObject, path: &str, usage: &mut CodeUsage) {
    let nid = object_node_id(obj);
    if !nid.is_empty()
        && (AdlCodeDefinitionsData::is_id_code(nid) || AdlCodeDefinitionsData::is_at_code(nid))
        && !aom_type(obj).is_primitive()
    {
        usage.node_codes.insert(nid.to_owned());
    }
    match obj {
        CObject::CComplexObject(cco) => collect_complex_usage(cco, path, usage),
        CObject::CTerminologyCode(tc) => {
            let codes = constraint_codes(&tc.constraint);
            if let Some(a) = tc.assumed_value.as_ref()
                && let Some(ac) = codes
                    .iter()
                    .find(|c| AdlCodeDefinitionsData::is_value_set_code(c))
            {
                usage
                    .assumed_refs
                    .push((path.to_owned(), ac.clone(), a.code_string.clone()));
            }
            usage.value_codes.extend(codes);
        }
        _ => {}
    }
}

/// Walks a complex object's attribute children and its second-order tuples.
///
/// The tuples (e.g. ordinals) carry primitive constraints outside the normal
/// attribute tree (master04.4), so their terminology-code values are collected
/// too.
fn collect_complex_usage(
    cco: &openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject,
    path: &str,
    usage: &mut CodeUsage,
) {
    for attr in complex_attributes(cco) {
        let apath = format!("{path}/{}", attr.rm_attribute_name);
        for child in attr.children.iter().flatten() {
            let cpath = child_path(&apath, object_node_id(child));
            collect_usage_at(child, &cpath, usage);
        }
    }
    for tuple in complex_attribute_tuples(cco) {
        for prim_tuple in tuple.tuples.iter().flatten() {
            for member in &prim_tuple.members {
                if let CPrimitiveObject::CTerminologyCode(tc) = member {
                    usage.value_codes.extend(constraint_codes(&tc.constraint));
                }
            }
        }
    }
}

/// The ARCHETYPE-LOCAL codes a `C_TERMINOLOGY_CODE.constraint` string names.
///
/// Two spellings reach this function, because the constraint string is the
/// verbatim carrier for both dialects:
///
/// - **ADL 2** — a single `at`/`ac` code, optionally suffixed with an operational
///   binding `@terminology` (`ADL2/master08-terminology_integration.adoc`).
/// - **ADL 1.4** — the qualified/listed custom-syntax form
///   `terminology::code[,code]*[;assumed]` of
///   `ADL1.4/master09-customising_adl.adoc` §Custom Syntax, which the 1.4 dialect
///   preserves verbatim. This is the DOMINANT 1.4 spelling of a coded value set,
///   so without decomposing it here the definedness rules never see its codes.
///
/// Only `local::` codes are archetype terms: VATDF/VACDF
/// (`ADL1.4/master08-adl.adoc` §Validity Rules) judge definedness against the
/// archetype's OWN `term_definitions`/`constraint_definitions`, and a code of an
/// external terminology (`[openehr::127]`, `[ISO_639-1::en]`) is not an archetype
/// term — its resolution is a terminology-service question (VETDF), not this one.
/// The ADL 1.4 assumed code (after `;`) IS an archetype term and is included.
fn constraint_codes(constraint: &str) -> Vec<String> {
    // Drop any ADL2 operational-binding suffix first.
    let body = constraint.split('@').next().unwrap_or(constraint).trim();
    let Some((terminology, rest)) = body.split_once("::") else {
        // ADL2 form: the constraint IS the code.
        return if body.is_empty() {
            Vec::new()
        } else {
            vec![body.to_owned()]
        };
    };
    if terminology.trim() != LOCAL_TERMINOLOGY_ID {
        return Vec::new();
    }
    // `code[,code]*[;assumed]`
    let (codes, assumed) = match rest.split_once(';') {
        Some((codes, assumed)) => (codes, Some(assumed)),
        None => (rest, None),
    };
    codes
        .split(',')
        .chain(assumed)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod constraint_code_tests {
    use super::constraint_codes;

    /// The ADL2 spelling: the constraint IS the code, with any operational
    /// binding suffix (`ADL2/master08-terminology_integration.adoc`) stripped.
    #[test]
    fn adl2_single_code_and_binding_suffix() {
        assert_eq!(constraint_codes("at1"), vec!["at1".to_owned()]);
        assert_eq!(constraint_codes("ac1@snomed"), vec!["ac1".to_owned()]);
        assert!(constraint_codes("").is_empty());
    }

    /// The ADL 1.4 spelling (`ADL1.4/master09-customising_adl.adoc` §Custom
    /// Syntax) decomposes into its listed codes plus the assumed code — all of
    /// them archetype terms VATDF must see.
    #[test]
    fn adl14_listed_codes_and_assumed_code() {
        assert_eq!(
            constraint_codes("local::at0136,at0137"),
            vec!["at0136".to_owned(), "at0137".to_owned()]
        );
        assert_eq!(
            constraint_codes("local::at0136,at0137;at0136"),
            vec![
                "at0136".to_owned(),
                "at0137".to_owned(),
                "at0136".to_owned()
            ]
        );
        assert_eq!(constraint_codes("local::ac0001"), vec!["ac0001".to_owned()]);
    }

    /// Codes of an EXTERNAL terminology are not archetype terms, so VATDF/VACDF
    /// definedness (`ADL1.4/master08-adl.adoc` §Validity Rules, judged against the
    /// archetype's own terminology) does not apply to them.
    #[test]
    fn external_terminology_codes_are_not_archetype_terms() {
        assert!(constraint_codes("openehr::127").is_empty());
        assert!(constraint_codes("openehr::253,271,273").is_empty());
        assert!(constraint_codes("ISO_639-1::en").is_empty());
    }
}
