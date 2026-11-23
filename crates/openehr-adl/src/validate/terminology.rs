// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 terminology: the archetype's own terms, constraint codes and value
//! sets, and the codes the definition references (`AOM2/master07` and
//! `master03` §Validity Rules; `master08` §Phase 1). It runs only on clean
//! identification and a sound terminology structure (`master08` §Overview).
//!
//! NOTE: WOUC appears in no vendored spec text: our own design.

use std::collections::{BTreeMap, BTreeSet};

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_base::v1_3::base_types::definitions::definitions_impl::LOCAL_TERMINOLOGY_ID;

use super::ValidationIssue;
use super::bindings::{check_bindings, check_term_bindings};
use super::catalogue::ValidationCode;
use super::identification::languages;
use super::scope;
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
    let defined = scope::defined_codes(term);

    // Re-scan the definition for referenced codes (usage + assumed values); on
    // an operational template the scan stops at each nested archetype root.
    let mut usage = CodeUsage::default();
    let root = CObject::CComplexObject(v.definition.clone());
    let mut roots = Vec::new();
    collect_usage_at(&root, "", &mut usage, v.components.map(|_| &mut roots));

    check_root_concept_code(v, &defined, issues);
    check_node_id_definedness(v, dialect, &usage, &defined, issues);
    let level = v.specialisation_level();
    check_referenced_codes(Some(level), !v.is_specialised(), &usage, &defined, issues);
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
    let first_roots = check_constituents(v, roots, issues);
    check_component_terminologies(v, &first_roots, issues);
}

/// The nested archetype roots a scoped usage scan stopped at, with their paths.
type Roots<'o> = Vec<(String, &'o CComplexObject)>;

/// The first archetype root met for each constituent, with its path.
type FirstRoots<'o> = BTreeMap<&'o str, (String, &'o CComplexObject)>;

/// VATDF / VACDF / VATDA below each nested archetype root of an operational
/// template, judged against that constituent's own flat terminology
/// ([`super::scope`]); a root met below a root is judged against its own.
/// Returns the first root met for each constituent.
fn check_constituents<'o>(
    v: &ArchetypeView<'_>,
    mut roots: Roots<'o>,
    issues: &mut Vec<ValidationIssue>,
) -> FirstRoots<'o> {
    let mut first = FirstRoots::new();
    let mut next = 0;
    while let Some((path, cco)) = roots.get(next).cloned() {
        next += 1;
        let CComplexObject::CArchetypeRoot(root) = cco else {
            continue;
        };
        first
            .entry(root.archetype_ref.as_str())
            .or_insert_with(|| (path.clone(), cco));
        let Some(term) = scope::constituent_terminology(v.components, root) else {
            continue;
        };
        let mut usage = CodeUsage::default();
        collect_complex_usage(cco, &path, &mut usage, Some(&mut roots));
        let start = issues.len();
        // NOTE: OPT2 master03-opt_raw keeps a constituent's terminology, not its
        // root concept code, so its level (and VATCD) is not judged below a root.
        check_referenced_codes(None, true, &usage, &scope::defined_codes(term), issues);
        check_assumed_values(term, &usage, issues);
        for issue in issues.iter_mut().skip(start) {
            issue.path.get_or_insert_with(|| path.clone());
        }
    }
    first
}

/// The standalone terminology rules on each `component_terminologies` entry of
/// an operational template (`OPT2/master03-opt_raw.adoc`), naming the component.
fn check_component_terminologies(
    v: &ArchetypeView<'_>,
    first_roots: &FirstRoots<'_>,
    issues: &mut Vec<ValidationIssue>,
) {
    for (id, term) in v.components.into_iter().flatten() {
        let start = issues.len();
        let defined = scope::defined_codes(term);
        if let Some(level) = codes::specialisation_depth(&term.concept_code) {
            check_code_levels(&defined, level, false, issues);
        }
        check_defined_code_forms(&defined, issues);
        check_language_coverage(term, issues);
        check_value_sets(term, &defined, true, issues);
        let root = first_roots.get(id.as_str());
        check_term_bindings(term, root.map(|(_, cco)| *cco), &defined, issues);
        for issue in issues.iter_mut().skip(start) {
            issue.message = format!("component terminology {id}: {}", issue.message);
            if let Some((path, _)) = root {
                issue.path.get_or_insert_with(|| path.clone());
            }
        }
    }
}

/// VATID: the root concept code must be defined (master08 §Code Validation); the
/// per-node half runs in [`super::rm`].
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

/// VATDF (ADL 1.4, node-id half): every node at-code must be defined in the
/// ontology (ADL1.4 master08 §Validity Rules VATDF; AOM1.4
/// `ARCHETYPE.node_ids_valid`).
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

/// The definedness and level rules over the referenced codes (master03
/// §Validity Rules): VATDF against the flat form (own-flat only here, else
/// [`super::flat`]), VACDF against the current archetype, and VATCD over level.
fn check_referenced_codes(
    level: Option<usize>,
    flat_self: bool,
    usage: &CodeUsage,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
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
            && let Some(level) = level
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

/// VTSD: every defined code is at the archetype's level (differential) or at
/// most it (flat) (master07 §Validity Rules); ac-codes are flat, and a 1.4
/// archetype always takes the flat rule (AOM1.4 master07 §Specialisation Depth).
fn check_defined_code_levels(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    defined: &BTreeSet<&str>,
    issues: &mut Vec<ValidationIssue>,
) {
    let differential = v.is_differential && dialect == Dialect::Adl2;
    check_code_levels(defined, v.specialisation_level(), differential, issues);
}

/// VTSD over `defined` at archetype `level`: differential codes sit at the
/// level, flat codes at or below it, ac-codes never above it.
fn check_code_levels(
    defined: &BTreeSet<&str>,
    level: usize,
    differential: bool,
    issues: &mut Vec<ValidationIssue>,
) {
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
/// NOTE: no openEHR spec governs WOUC: our own design, off in the 1.4 dialect,
/// whose value codes sit inside verbatim constraint strings.
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

/// One value set's integrity rules (master07 §Validity Rules): VTVSID against
/// the current archetype, VTVSUQ, and VTVSMD against the flat form (own-flat
/// only here, else [`super::flat`]).
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

/// Collects the codes `obj` uses; with `roots`, a nested archetype root is
/// recorded there and its subtree left to its own scope.
pub(super) fn collect_usage_at<'o>(
    obj: &'o CObject,
    path: &str,
    usage: &mut CodeUsage,
    roots: Option<&mut Roots<'o>>,
) {
    let nid = object_node_id(obj);
    if !nid.is_empty()
        && (AdlCodeDefinitionsData::is_id_code(nid) || AdlCodeDefinitionsData::is_at_code(nid))
        && !aom_type(obj).is_primitive()
    {
        usage.node_codes.insert(nid.to_owned());
    }
    match obj {
        CObject::CComplexObject(cco) => match roots {
            Some(found) if !path.is_empty() && matches!(cco, CComplexObject::CArchetypeRoot(_)) => {
                found.push((path.to_owned(), cco));
            }
            roots => collect_complex_usage(cco, path, usage, roots),
        },
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

/// Walks a complex object's attributes and its second-order tuples (master04.4).
fn collect_complex_usage<'o>(
    cco: &'o CComplexObject,
    path: &str,
    usage: &mut CodeUsage,
    mut roots: Option<&mut Roots<'o>>,
) {
    for attr in complex_attributes(cco) {
        let apath = format!("{path}/{}", attr.rm_attribute_name);
        for child in attr.children.iter().flatten() {
            let cpath = child_path(&apath, object_node_id(child));
            collect_usage_at(child, &cpath, usage, roots.as_deref_mut());
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

/// The archetype-local codes a `C_TERMINOLOGY_CODE.constraint` names: an ADL 2
/// code with an optional `@terminology` binding, or the ADL 1.4
/// `terminology::code[,code]*[;assumed]` form (`master09-customising_adl.adoc`).
///
/// Only `local::` codes and the assumed code count; an external code is VETDF's
/// question (`ADL1.4/master08-adl.adoc` §Validity Rules).
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
        assert_eq!(constraint_codes(""), Vec::<String>::new());
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
        assert_eq!(constraint_codes("openehr::127"), Vec::<String>::new());
        assert_eq!(
            constraint_codes("openehr::253,271,273"),
            Vec::<String>::new()
        );
        assert_eq!(constraint_codes("ISO_639-1::en"), Vec::<String>::new());
    }
}
