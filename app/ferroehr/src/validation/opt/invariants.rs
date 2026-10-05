// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AOM 1.4 constraint-model per-node-kind invariants for the OPT 1.4 pass
//! (T4, T5b, T13, T14, T15, T16 of the AM constraint taxonomy).
//!
//! These are the invariants the AOM 1.4 constraint-model class files declare
//! (`AM/docs/AOM1.4/master04-constraint_model_package.adoc`) plus the ADL 1.4
//! identifier and slot syntax rules (`AM/docs/ADL1.4/master08-adl.adoc`,
//! `master05-cadl.adoc`), all decidable structurally on a flattened OPT tree
//! without running the `valid_value` cascade:
//!
//! - `Rm_attribute_name_valid` and `Existence_set` (`C_ATTRIBUTE`),
//!   `Members_valid` (`C_SINGLE_ATTRIBUTE`);
//! - VARID and VARDT, archetype-identifier syntax and the RM type / id type slot
//!   match (master08 lines 544/556);
//! - VDFAI, a slot-referenced archetype id is well-formed (master08 573);
//! - `Target_path_valid`, an `ARCHETYPE_INTERNAL_REF` target path;
//! - VACDF, a `CONSTRAINT_REF` ac-code is defined (master08 566);
//! - STCDC, a terminology-code list has no duplicate codes
//!   (`ADL2/master04.6-cadl_validity_rules.adoc`).

use std::collections::HashSet;

use openehr_its::opt14::types::{
    ArchetypeInternalRef, ArchetypeSlot, Assertion, CObject, ConstraintRef, ExprItem,
    Intervalofinteger,
};
use openehr_its::xml::runtime::XmlAny;

use super::interval::{iv_lower, iv_upper};
use super::{Ctx, NodeView, RuleViolation};

// ─── C_ATTRIBUTE / C_SINGLE_ATTRIBUTE invariants (T4, T5b) ───────────────────────

/// `C_ATTRIBUTE` invariant `Rm_attribute_name_valid`: `not rm_attribute_name.
/// is_empty` (AOM1.4 `c_attribute` class file, Invariants).
pub(super) fn check_attribute_name(attr_name: &str, parent_rm: &str) -> Result<(), RuleViolation> {
    if attr_name.is_empty() {
        return Err(RuleViolation::new(
            "Rm_attribute_name_valid",
            format!("an attribute constraint under '{parent_rm}' has an empty rm_attribute_name"),
        ));
    }
    Ok(())
}

/// `C_ATTRIBUTE` invariant `Existence_set`: `existence.lower >= 0 and
/// existence.upper <= 1` (AOM1.4 `c_attribute` class file, Invariants).
pub(super) fn check_existence_set(
    attr_name: &str,
    parent_rm: &str,
    existence: &Intervalofinteger,
) -> Result<(), RuleViolation> {
    if iv_lower(existence) < 0
        || existence.upper_unbounded
        || iv_upper(existence).is_none_or(|u| u > 1)
        || iv_upper(existence).is_some_and(|u| u < iv_lower(existence))
    {
        return Err(RuleViolation::new(
            "Existence_set",
            format!(
                "attribute '{attr_name}' on '{parent_rm}' has an existence outside 0..1 \
                 (existence.lower >= 0 and existence.upper <= 1)"
            ),
        ));
    }
    Ok(())
}

/// `C_SINGLE_ATTRIBUTE` invariant `Members_valid`: every alternative child
/// satisfies `co.occurrences.upper <= 1` — a single-valued attribute can hold
/// at most one value (AOM1.4 `c_single_attribute` class file, Invariants; also
/// cADL: occurrences upper > 1 only under a container attribute, AOM1.4
/// `c_object` class file, `occurrences`). Called only for a single-valued
/// attribute (no cardinality).
pub(super) fn check_members_valid(
    attr_name: &str,
    parent_rm: &str,
    children: &[CObject],
) -> Result<(), RuleViolation> {
    for child in children {
        let view = NodeView::of(child);
        let occ = view.occurrences;
        if occ.upper_unbounded || iv_upper(occ).is_some_and(|u| u > 1) {
            return Err(RuleViolation::new(
                "Members_valid",
                format!(
                    "attribute '{attr_name}' on '{parent_rm}' is single-valued but child object \
                     '{}' has occurrences upper > 1",
                    view.node_id
                ),
            ));
        }
    }
    Ok(())
}

// ─── archetype identifiers (VARID / VARDT, T16) ─────────────────────────────────

/// VARID: the archetype id must conform to the openEHR archetype-identifier
/// syntax (ADL1.4 master08 line 544; BASE `base_types` master05 §Syntaxes), and
/// VARDT: the RM type named by the constraint node must match the type slot of
/// the identifier's first segment (ADL1.4 master08 line 556; composite
/// identifiers compare case-insensitively, BASE `base_types` master05
/// §"Composite Identifiers and Case").
pub(super) fn check_archetype_id(id: &str, rm_type_name: &str) -> Result<(), RuleViolation> {
    if !is_archetype_id_shaped(id) {
        return Err(RuleViolation::new(
            "VARID",
            format!("'{id}' is not a valid openEHR archetype identifier"),
        ));
    }
    // qualified_rm_entity = rm_originator '-' rm_name '-' rm_entity; the
    // rm_entity is everything after the second '-'.
    let qualified = id.split('.').next().unwrap_or("");
    let entity = qualified.splitn(3, '-').nth(2).unwrap_or_default();
    let bare_rm = rm_type_name.split('<').next().unwrap_or(rm_type_name);
    if !entity.eq_ignore_ascii_case(bare_rm) {
        return Err(RuleViolation::new(
            "VARDT",
            format!(
                "the definition node's RM type '{rm_type_name}' does not match the type slot \
                 '{entity}' of archetype id '{id}'"
            ),
        ));
    }
    Ok(())
}

/// Archetype-identifier shape for uploaded artefacts:
/// `rm_originator-rm_name-rm_entity.domain_concept.v<version>` (BASE `base_types`
/// master05 §Syntaxes), read here rather than through the BASE parser, whose
/// grammar is narrower than an uploaded OPT's identifiers on both axes below.
/// Tolerances beyond that grammar, each adjudicated against real published
/// templates (never against CNF valid fixtures, which all conform strictly):
///
/// - the version may be multi-part numeric (`v1.0.0`) — the ADL2-era archetype
///   HRID form appears in deployed OPT 1.4 exports (the vendored
///   `Request_for_Pancreas_Special_Urgency_Listing` corpus template);
/// - NOTE: concept segments tolerate `(`/`)` and digit-leading segments —
///   Ocean/LANIT tooling emits concept names like
///   `t_neurologist_examination(1-17)_lanit` (vendored Better corpus); the
///   strict grammar would refuse real-world templates.
fn is_archetype_id_shaped(id: &str) -> bool {
    fn alphanum_str(s: &str) -> bool {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    // The split is done here rather than through `ARCHETYPE_ID::from_str`,
    // because BASE's `version_id` production is single-part by definition
    // (`'0' | non-zero-digit, [ number ]`, master05 §Syntaxes) — it REFUSES the
    // `v1.0.0` this function's own tolerances exist to accept, so routing
    // through it would make both documented tolerances unreachable.
    let Some((qualified_and_concept, version)) = id.rsplit_once(".v") else {
        return false;
    };
    let Some((qualified, concept)) = qualified_and_concept.split_once('.') else {
        return false;
    };
    let version_ok = version.split('.').count() <= 3
        && version
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let entity_parts: Vec<&str> = qualified.split('-').collect();
    let entity_ok = entity_parts.len() == 3 && entity_parts.iter().all(|p| alphanum_str(p));
    let concept_ok = !concept.is_empty()
        && concept
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '(' | ')' | '.'));
    version_ok && entity_ok && concept_ok
}

// ─── ARCHETYPE_SLOT (VDFAI, T13) ────────────────────────────────────────────────

/// `ARCHETYPE_SLOT` checks: VDFAI — an archetype identifier mentioned in a slot
/// must conform to the archetype-identifier syntax (ADL1.4 master08 line 573).
/// Slot include/exclude expressions are Perl regexes over archetype ids (cADL
/// §Archetype Slots), so only a *literal* pattern (regex-escaped dots, no other
/// metacharacters) is decidable as an identifier; genuine regexes are left to
/// runtime slot admission.
///
/// NOTE: this decides only the *literal id-shape* case; a genuine PERL
/// regex include/exclude expression is not a decidable identifier at upload
/// and is deferred to runtime slot admission (the `WebTemplate` instance walk)
/// — that surface is out of scope for the artefact
/// pass here (cADL §Archetype Slots, `ADL1.4/master05-cadl.adoc` L535-601).
pub(super) fn check_slot(slot: &ArchetypeSlot) -> Result<(), RuleViolation> {
    for assertion in slot.includes.iter().chain(&slot.excludes) {
        let Some(pattern) = slot_assertion_pattern(assertion) else {
            continue;
        };
        for alt in pattern.split('|') {
            let Some(literal) = regex_literal(alt) else {
                continue;
            };
            if !is_archetype_id_shaped(&literal) {
                return Err(RuleViolation::new(
                    "VDFAI",
                    format!(
                        "slot '{}' names '{literal}', which is not a valid openEHR archetype \
                         identifier",
                        slot.node_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// The archetype-id regex carried by a slot assertion (`archetype_id/value
/// matches {/…/}`), if the expression has that shape.
fn slot_assertion_pattern(a: &Assertion) -> Option<String> {
    fn find_pattern(e: &ExprItem) -> Option<String> {
        match e {
            ExprItem::ExprLeaf(l) => {
                let raw = l
                    .item
                    .child("pattern")
                    .map_or_else(|| l.item.text(), XmlAny::text);
                // Document formatting, not payload: a pretty-printed OPT wraps
                // the pattern text, and xs:string preserves that whitespace.
                Some(raw.trim().trim_matches('/').to_owned())
                    .filter(|s| s.contains("openEHR") || s.contains('\\') || s.contains('.'))
            }
            ExprItem::ExprBinaryOperator(b) => {
                find_pattern(&b.right_operand).or_else(|| find_pattern(&b.left_operand))
            }
            ExprItem::ExprUnaryOperator(u) => find_pattern(&u.operand),
        }
    }
    find_pattern(&a.expression)
}

/// If a regex alternative is a literal archetype id (only `\.`-escaped dots,
/// no other metacharacters), return the unescaped literal; else `None`.
fn regex_literal(alt: &str) -> Option<String> {
    let mut out = String::with_capacity(alt.len());
    let mut chars = alt.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('.') => out.push('.'),
                _ => return None,
            },
            '.' | '*' | '+' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '^' | '$' | '|' => {
                return None;
            }
            _ => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

// ─── ARCHETYPE_INTERNAL_REF (Target_path_valid, T14) ────────────────────────────

/// `ARCHETYPE_INTERNAL_REF` invariant `Target_path_valid`: `target_path /= Void
/// and then not target_path.is_empty` (AOM1.4 `archetype_internal_ref` class
/// file, Invariants); the path must be an absolute archetype path (VDFPT,
/// ADL1.4 master08 line 576). (Flattened OPTs expand internal refs, so this
/// fires only on a malformed artefact — the whole vendored corpus carries
/// none.)
pub(super) fn check_internal_ref(r: &ArchetypeInternalRef) -> Result<(), RuleViolation> {
    if r.target_path.is_empty() || !r.target_path.starts_with('/') {
        return Err(RuleViolation::new(
            "Target_path_valid",
            format!(
                "internal reference '{}' has an invalid target_path '{}' (must be a non-empty \
                 absolute path)",
                r.node_id, r.target_path
            ),
        ));
    }
    Ok(())
}

// ─── CONSTRAINT_REF (VACDF, T15) ────────────────────────────────────────────────

/// VACDF: each constraint code (`acNNNN`) used in the definition must be
/// defined in the `constraint_definitions` part of the ontology (ADL1.4
/// master08 line 566).
///
/// NOTE (flattened-OPT tolerance): deployed OPT 1.4 exports routinely
/// carry `CONSTRAINT_REF` nodes with NO `constraint_definitions` sets at all
/// (Ocean Template Designer drops the constraint vocabulary on flatten — the
/// vendored RIPPLE/Better corpus templates). VACDF is therefore enforced only
/// when the artefact declares a constraint vocabulary; an artefact with none
/// is tolerated.
pub(super) fn check_constraint_ref(r: &ConstraintRef, ctx: &Ctx) -> Result<(), RuleViolation> {
    if ctx.has_constraint_defs && !ctx.defined_ac.contains(&r.reference) {
        return Err(RuleViolation::new(
            "VACDF",
            format!(
                "constraint reference '{}' (node '{}') is not defined in constraint_definitions",
                r.reference, r.node_id
            ),
        ));
    }
    Ok(())
}

// ─── terminology-code list (STCDC, T9/T10) ──────────────────────────────────────

/// Duplicate codes in a terminology-code code list are invalid (ADL2
/// master04.6 STCDC — "constraint code list contains duplicate codes"; the
/// same defect in an OPT 1.4 `C_CODE_PHRASE` list).
pub(super) fn check_code_list(code_list: &[String], node_id: &str) -> Vec<RuleViolation> {
    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    let mut violations = Vec::new();
    for code in code_list {
        // Empty entries are tooling noise, not codes (Ocean exports emit
        // repeated empty <code_list/> elements — the vendored UK AoMRC corpus
        // template); only real codes participate in the duplicate check.
        if code.is_empty() {
            continue;
        }
        // Every duplicated code is reported, once each however many times it
        // repeats: a list with seventeen of them used to cost seventeen uploads
        // to find (#3129), and a code repeated three times is still one defect.
        if !seen.insert(code) && reported.insert(code) {
            violations.push(RuleViolation::new(
                "STCDC",
                format!("node '{node_id}': code '{code}' is duplicated in the code list"),
            ));
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::check_code_list;

    /// The reported case (#3129): one template held seventeen duplicated codes
    /// and the refusal named the first, so finding them all took seventeen
    /// uploads. Every duplicate is reported now.
    #[test]
    fn every_duplicated_code_is_reported_not_just_the_first() {
        let list: Vec<String> = ["a", "b", "a", "c", "b", "d", "e", "e"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();

        let details: Vec<String> = check_code_list(&list, "at0001")
            .iter()
            .map(|v| v.detail.clone())
            .collect();

        assert_eq!(details.len(), 3, "three codes repeat: {details:?}");
        for code in ["'a'", "'b'", "'e'"] {
            assert!(
                details.iter().any(|d| d.contains(code)),
                "missing {code} in {details:?}"
            );
        }
    }

    /// A code repeated three times is one defect, not two.
    #[test]
    fn a_code_repeated_several_times_is_reported_once() {
        let list: Vec<String> = ["x", "x", "x", "x"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(check_code_list(&list, "at0002").len(), 1);
    }

    /// Empty entries are tooling noise, not codes, so repeating them is not a
    /// duplicate (Ocean exports emit repeated empty `code_list` elements).
    #[test]
    fn repeated_empty_entries_are_not_duplicates() {
        let list: Vec<String> = ["", "", "a", ""].iter().map(|s| (*s).to_owned()).collect();
        assert!(check_code_list(&list, "at0003").is_empty());
    }
}
