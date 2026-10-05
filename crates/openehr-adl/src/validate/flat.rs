// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Flat-form topic: everything decidable only after flattening.
//!
//! Two groups, both run by [`super::run_flat_form_checks`] on the flat form
//! ([`crate::flatten::flat_form`]) and both gated on an error-free
//! basic-integrity pass (`master08` §Overview phase gate).
//!
//! **The flat-form pass proper** —
//! `docs/specs/openehr/AM/docs/AOM2/master08-validation.adoc`
//! §Phase 3 - Validation of Flat Form ("carried out after successful generation
//! of the flat form"), [`validate_flat_form_structure`]:
//!
//! * VUNP — every `C_COMPLEX_OBJECT_PROXY` (`use_node`) target path must resolve
//!   to an object node in the flat form (`master04.5`
//!   §`C_COMPLEX_OBJECT_PROXY`, VUNP L482-483).
//! * VACMCO — every object node's occurrences must be satisfiable within its
//!   enclosing attribute's cardinality (`master04.5` §`C_ATTRIBUTE`, VACMCO
//!   L158-159).
//!
//! These run on the flat form because a proxy target or a container's full child
//! set may be assembled from several specialisation levels. The same proxy walk
//! serves the ADL 1.4 rule VDFPT ([`validate_definition_paths_adl14`]), whose
//! resolution target is a 1.4 artefact's own (standalone) definition.
//!
//! **Deferred basic-integrity halves for a SPECIALISED archetype**
//! ([`validate_flat_form`]) — four checks are properties of the FLAT form. For a
//! non-specialised archetype the differential is the flat form, so
//! [`super::structure`] / [`super::terminology`] run them directly:
//!
//! * **VATDF** — every at-code used as a value must be defined in the flattened
//!   terminology (`master03` §Validity Rules).
//! * **VTVSMD** — every value-set member must be defined there too (`master07`
//!   §Validity Rules).
//! * **VACMCU** (error) / **WACMCL** (warning) — a container attribute's child
//!   occurrences against its flattened cardinality (`master04.5`
//!   §`C_ATTRIBUTE`); a specialised child may not restate the inherited
//!   cardinality.
//! * **VCOSU** — object node ids must be unique archetype-wide (`master04.5`
//!   §`C_OBJECT`), judged per node IDENTITY because flattening clones a
//!   redefined node's whole subtree.
//!
//! [`super::run_flat_form_checks`] runs that second group only for a specialised
//! archetype whose flat form was produced, so no check double-fires.

use std::collections::{BTreeSet, HashMap};

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use super::catalogue::ValidationCode;
use super::structure::occurrences_lower;
use super::terminology::{CodeUsage, collect_usage};
use super::{ValidationIssue, push_issue};
use crate::aom::access::{aom_type, child_occurrences, complex_attributes, object_node_id};
use crate::aom::interval::finite_upper;
use crate::artefact::view;
use crate::codes::specialisation_parent_from_code;
use crate::paths::{Resolution, child_path, is_ancestor_path, locate, resolve};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

// ── the flat-form proxy + cardinality walk ────────────────────────────────

/// Validate the FLAT form `flat` against the internal-reference and
/// cardinality/occurrences catalogue: VUNP (every `use_node` proxy target
/// resolves) and VACMCO (`master08` "phase 3 — validation of flat form" in the
/// spec's guide vocabulary).
#[must_use]
pub(super) fn validate_flat_form_structure(flat: &Archetype) -> Vec<ValidationIssue> {
    let v = view(flat);
    let mut scan = FlatScan {
        root: v.definition,
        issues: Vec::new(),
        proxy_code: ValidationCode::Vunp,
        check_cardinality: true,
    };
    scan.walk(v.definition, "");
    scan.issues
}

/// VDFPT for an assembled **ADL 1.4** archetype: every `use_node` internal
/// reference (a `C_COMPLEX_OBJECT_PROXY` after assembly) must carry a target
/// path that resolves within the definition section (`ADL1.4/master08-adl.adoc`
/// §Definition Section validity rules, VDFPT). A 1.4 artefact is standalone —
/// no differential lineage — so its own definition tree IS the resolution
/// target; the AOM2 flat-form mirror of this rule is VUNP
/// ([`validate_flat_form_structure`]). Cardinality/occurrences stay with the
/// 1.4-dialect VCOC check in the basic-integrity pass, so only the proxy walk
/// runs here.
#[must_use]
pub(super) fn validate_definition_paths_adl14(archetype: &Archetype) -> Vec<ValidationIssue> {
    let v = view(archetype);
    let mut scan = FlatScan {
        root: v.definition,
        issues: Vec::new(),
        proxy_code: ValidationCode::Vdfpt,
        check_cardinality: false,
    };
    scan.walk(v.definition, "");
    scan.issues
}

struct FlatScan<'a> {
    root: &'a CComplexObject,
    issues: Vec<ValidationIssue>,
    /// The catalogue code a non-resolving proxy target raises: VUNP on the
    /// AOM2 flat form, VDFPT on an assembled ADL 1.4 definition.
    proxy_code: ValidationCode,
    /// VACMCO runs on the AOM2 flat form only — the ADL 1.4 dialect enforces
    /// its own VCOC in phase 1 instead.
    check_cardinality: bool,
}

impl FlatScan<'_> {
    fn walk(&mut self, node: &CComplexObject, path: &str) {
        for attr in complex_attributes(node) {
            let attr_path = format!("{path}/{}", attr.rm_attribute_name);
            if self.check_cardinality {
                self.check_cardinality_occurrences(attr, &attr_path);
            }
            for child in attr.children.iter().flatten() {
                let child_path = child_path(&attr_path, object_node_id(child));
                match child {
                    CObject::CComplexObjectProxy(proxy) => {
                        self.check_proxy(&proxy.target_path, &child_path);
                    }
                    CObject::CComplexObject(CComplexObject::CComplexObject(_)) => {
                        if let CObject::CComplexObject(cco) = child {
                            self.walk(cco, &child_path);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The proxy target path must resolve to an object node in the walked
    /// definition — VUNP on the AOM2 flat form (`master04.5`
    /// §`C_COMPLEX_OBJECT_PROXY`, VUNP L482-483), VDFPT on an assembled
    /// ADL 1.4 definition (`ADL1.4/master08-adl.adoc` §Definition Section).
    /// Beyond bare resolution, VUNP's own text requires the target "is not
    /// itself an internal reference node", and the target must not lie on the
    /// proxy's own ancestor path ("The path must not be in the parent path of
    /// the proxy object itself, but may be a sibling",
    /// `ADL2/master04.3-cadl_complex_types.adoc` §Internal References) — an
    /// ancestor target makes the proxy's deep-copy expansion infinitely
    /// recursive, so it is rejected for both dialect codes.
    fn check_proxy(&mut self, target_path: &str, path: &str) {
        if target_path.is_empty() {
            push_issue(
                &mut self.issues,
                self.proxy_code,
                "use_node proxy has no target path",
                path,
            );
            return;
        }
        if is_ancestor_path(target_path, path) {
            push_issue(
                &mut self.issues,
                self.proxy_code,
                format!(
                    "use_node target path {target_path:?} is in the parent path of the proxy object itself"
                ),
                path,
            );
            return;
        }
        match locate(self.root, target_path) {
            Some(CObject::CComplexObjectProxy(_)) => {
                push_issue(
                    &mut self.issues,
                    self.proxy_code,
                    format!(
                        "use_node target path {target_path:?} refers to another internal reference node"
                    ),
                    path,
                );
            }
            Some(_) => {}
            None => {
                if resolve(self.root, target_path) != Resolution::Found {
                    push_issue(
                        &mut self.issues,
                        self.proxy_code,
                        format!(
                            "use_node target path {target_path:?} does not resolve to an object node in the definition"
                        ),
                        path,
                    );
                }
            }
        }
    }

    /// VACMCO: for a container attribute with a finite cardinality upper bound, it
    /// must be possible to include one instance of every mandatory child (sum of
    /// stated occurrences lower bounds) plus one instance of one optional child
    /// within the cardinality range (`master04.5` §`C_ATTRIBUTE`, VACMCO
    /// L158-159).
    fn check_cardinality_occurrences(&mut self, attr: &CAttribute, path: &str) {
        let Some(card) = attr.cardinality.as_ref() else {
            return;
        };
        if card.interval.upper_unbounded {
            return;
        }
        let Some(upper) = card.interval.upper else {
            return;
        };
        let mut mandatory_floor: i64 = 0;
        let mut has_optional = false;
        for child in attr.children.iter().flatten() {
            let Some(occ) = child_occurrences(child) else {
                continue;
            };
            let lower = if occ.lower_unbounded {
                0
            } else {
                occ.lower.unwrap_or(0)
            };
            if lower == 0 {
                has_optional = true;
            } else {
                mandatory_floor += i64::from(lower);
            }
        }
        let needed = mandatory_floor + i64::from(has_optional);
        if needed > i64::from(upper) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vacmco,
                format!(
                    "the minimum required occurrences ({needed}) of the child objects cannot fit within the cardinality upper bound ({upper})"
                ),
                path,
            );
        }
    }
}

// ── the deferred phase-1 halves for a specialised archetype ───────────────

/// Run the deferred flat-form checks (VATDF / VTVSMD / VACMCU / WACMCL / VCOSU)
/// against the flattened archetype `flat`.
#[must_use]
pub(super) fn validate_flat_form(flat: &Archetype) -> Vec<ValidationIssue> {
    let v = view(flat);
    let mut issues = Vec::new();

    let defined: BTreeSet<&str> = v
        .terminology
        .term_definitions
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();

    // VATDF: value at-codes used in the flattened definition must be defined.
    let mut usage = CodeUsage::default();
    let root = CObject::CComplexObject(v.definition.clone());
    collect_usage(&root, &mut usage);
    for code in &usage.value_codes {
        if AdlCodeDefinitionsData::is_at_code(code) && !defined.contains(code.as_str()) {
            issues.push(ValidationIssue::new(
                ValidationCode::Vatdf,
                format!("value code {code:?} is not defined in the flattened terminology"),
            ));
        }
    }

    // VTVSMD: value-set members must be defined in the flattened terminology.
    if let Some(value_sets) = v.terminology.value_sets.as_ref() {
        for set in value_sets.values() {
            for m in &set.members {
                if !defined.contains(m.as_str()) {
                    issues.push(ValidationIssue::new(
                        ValidationCode::Vtvsmd,
                        format!(
                            "value set {:?} member {m:?} is not defined in the flattened terminology",
                            set.id
                        ),
                    ));
                }
            }
        }
    }

    // VACMCU / WACMCL + VCOSU: the flat-form structural walk.
    let mut seen_node_ids: HashMap<String, String> = HashMap::new();
    walk_flat(v.definition, "", &mut seen_node_ids, &mut issues);

    issues
}

/// Walk the flat definition tree: VCOSU archetype-wide node-id uniqueness on
/// every object, VACMCU/WACMCL on every container attribute.
fn walk_flat(
    cco: &CComplexObject,
    path: &str,
    seen: &mut HashMap<String, String>,
    issues: &mut Vec<ValidationIssue>,
) {
    for attr in complex_attributes(cco) {
        let attr_path = format!("{path}/{}", attr.rm_attribute_name);
        // VACMCU / WACMCL: container cardinality vs child occurrences.
        if attr.cardinality.is_some() {
            check_container_cardinality(&attr_path, attr, issues);
        }
        for child in attr.children.iter().flatten() {
            let cpath = child_path(&attr_path, object_node_id(child));
            check_node_id_unique(child, &cpath, seen, issues);
            if let CObject::CComplexObject(child_cco) = child {
                walk_flat(child_cco, &cpath, seen, issues);
            }
        }
    }
}

/// VCOSU: an identified (non-primitive) object node's id must be unique across
/// the flat form — judged per NODE IDENTITY, not per materialisation.
///
/// NOTE: raw id counting is unsound on a flat form — flattening CLONES a
/// parent subtree per specialised child, ids included
/// (`master08-validation.adoc` §Flattening;
/// `ADL2/master09.05-spec_object_redef.adoc` §Flattening) — so two
/// occurrences are compared on their SPECIALISATION-ROOT paths: equal root
/// paths are clones of one node (legal), different root paths are two nodes
/// wearing one id (the VCOSU violation).
fn check_node_id_unique(
    obj: &CObject,
    path: &str,
    seen: &mut HashMap<String, String>,
    issues: &mut Vec<ValidationIssue>,
) {
    let nid = object_node_id(obj);
    if nid.is_empty()
        || !(AdlCodeDefinitionsData::is_id_code(nid) || AdlCodeDefinitionsData::is_at_code(nid))
    {
        return;
    }
    if aom_type(obj).is_primitive() {
        return;
    }
    let identity = specialisation_root_path(path);
    match seen.get(nid) {
        Some(first) if *first != identity => push_issue(
            issues,
            ValidationCode::Vcosu,
            format!("node id {nid:?} is not unique in the flat form (also at {first})"),
            path,
        ),
        Some(_) => {}
        None => {
            seen.insert(nid.to_owned(), identity);
        }
    }
}

/// A path with every REDEFINED node code reduced to the level-0 code it
/// specialises (`[id4.1]` → `[id4]`, `[id4.1.2]` → `[id4]`), so two clones of
/// one parent node share a path and two distinct nodes do not.
///
/// A code that is NEW at its level (`[id0.32]`) specialises nothing —
/// [`AdlCodeDefinitionsData::is_redefined_code`] is false for it — so it is left
/// alone and two independently-added nodes never collapse onto each other.
///
/// [`AdlCodeDefinitionsData::is_redefined_code`]: openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData::is_redefined_code
fn specialisation_root_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('[') {
        let Some(head) = rest.get(..=open) else { break };
        out.push_str(head);
        let Some(after) = rest.get(open + 1..) else {
            break;
        };
        let Some(close) = after.find(']') else {
            out.push_str(after);
            return out;
        };
        let Some(code) = after.get(..close) else {
            break;
        };
        out.push_str(&specialisation_root_code(code));
        out.push(']');
        rest = after.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// The level-0 code a redefined code specialises; the code itself otherwise.
fn specialisation_root_code(code: &str) -> String {
    let mut current = code.to_owned();
    while AdlCodeDefinitionsData::is_redefined_code(&current) {
        let Some(parent) = specialisation_parent_from_code(&current) else {
            break;
        };
        current = parent;
    }
    current
}

/// VACMCU (error) + WACMCL (warning): container cardinality upper vs child
/// occurrences on the flat form (`master04.5` §`C_ATTRIBUTE`).
fn check_container_cardinality(
    attr_path: &str,
    attr: &CAttribute,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(card) = attr.cardinality.as_ref() else {
        return;
    };
    let Some(card_upper) = finite_upper(&card.interval) else {
        return;
    };
    let mut sum_lower = 0i64;
    for child in attr.children.iter().flatten() {
        let Some(occ) = child_occurrences(child) else {
            continue;
        };
        if let Some(u) = finite_upper(occ)
            && i64::from(u) > i64::from(card_upper)
        {
            push_issue(
                issues,
                ValidationCode::Vacmcu,
                format!("child occurrences upper {u} exceeds the cardinality upper {card_upper}"),
                attr_path,
            );
        }
        sum_lower += i64::from(occurrences_lower(occ));
    }
    if sum_lower > i64::from(card_upper) {
        push_issue(
            issues,
            ValidationCode::Wacmcl,
            format!(
                "sum of child occurrences lowers {sum_lower} exceeds the cardinality upper {card_upper}"
            ),
            attr_path,
        );
    }
}

#[cfg(test)]
mod flat_form_structure_tests {
    use super::validate_flat_form_structure;
    use crate::assemble::parse_artefact;
    use crate::parse::Dialect;
    use crate::validate::ValidationCode;

    fn codes(src: &str) -> Vec<ValidationCode> {
        let art = parse_artefact(src, Dialect::Adl2).unwrap();
        validate_flat_form_structure(&art)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn vacmco_orphan_mandatory_children_exceed_cardinality() {
        // items cardinality {0..2} but three mandatory children (occurrences {1}
        // each): the minimum 3 cannot fit in 2 (`master04.5` §`C_ATTRIBUTE`
        // VACMCO L158-159).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vacmco.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..2} matches {
\t\t\tELEMENT[id2] occurrences matches {1}
\t\t\tELEMENT[id3] occurrences matches {1}
\t\t\tELEMENT[id4] occurrences matches {1}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vacmco));
    }

    #[test]
    fn vacmco_clean_when_children_fit() {
        // items cardinality {0..3}, three mandatory children: 3 <= 3, clean.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vacmco_ok.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..3} matches {
\t\t\tELEMENT[id2] occurrences matches {1}
\t\t\tELEMENT[id3] occurrences matches {1}
\t\t\tELEMENT[id4] occurrences matches {1}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(!codes(src).contains(&ValidationCode::Vacmco));
    }

    #[test]
    fn vunp_use_node_path_does_not_resolve() {
        // A use_node whose target path points at a non-existent node
        // (`master04.5` §`C_COMPLEX_OBJECT_PROXY` VUNP L482-483).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vunp.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems matches {
\t\t\tELEMENT[id2]
\t\t\tuse_node ELEMENT[id5] /items[id99]
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id5\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vunp));
    }

    #[test]
    fn vunp_clean_when_use_node_path_resolves() {
        // A use_node whose target path resolves to a real object node — clean.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vunp_ok.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems matches {
\t\t\tELEMENT[id2]
\t\t\tuse_node ELEMENT[id5] /items[id2]
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id5\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(!codes(src).contains(&ValidationCode::Vunp));
    }
    #[test]
    fn vunp_ancestor_target_is_rejected() {
        // `ADL2/master04.3` §Internal References: "The path must not be in the
        // parent path of the proxy object itself" — an ancestor target defines
        // an infinitely recursive expansion.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vunp_cycle.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..*} matches {
\t\t\tCLUSTER[id2] matches {
\t\t\t\titems cardinality matches {0..*} matches {
\t\t\t\t\tuse_node CLUSTER[id3] /items[id2]
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vunp));
    }

    #[test]
    fn vunp_proxy_target_that_is_a_proxy_is_rejected() {
        // VUNP's own text (`master04.5` §C_COMPLEX_OBJECT_PROXY): the target
        // "is not itself an internal reference node".
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vunp_chain.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..*} matches {
\t\t\tELEMENT[id2] matches {*}
\t\t\tuse_node ELEMENT[id3] /items[id2]
\t\t\tuse_node ELEMENT[id4] /items[id3]
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vunp));
    }

    #[test]
    fn vunp_sibling_and_cross_branch_targets_stay_clean() {
        // The sibling case is expressly legal ("may be a sibling of the proxy
        // object", `master04.3` §Internal References); a cross-branch target
        // sharing an attribute name but a different node id is not an ancestor.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.vunp_ok.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..*} matches {
\t\t\tELEMENT[id2] matches {*}
\t\t\tuse_node ELEMENT[id3] /items[id2]
\t\t\tCLUSTER[id5] matches {
\t\t\t\titems cardinality matches {0..*} matches {
\t\t\t\t\tuse_node ELEMENT[id6] /items[id2]
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id5\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id6\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(!codes(src).contains(&ValidationCode::Vunp));
    }
}

#[cfg(test)]
mod flat_form_tests {
    use super::{specialisation_root_path, validate_flat_form};
    use crate::assemble::parse_artefact;
    use crate::parse::Dialect;
    use crate::validate::ValidationCode;

    fn codes(src: &str) -> Vec<ValidationCode> {
        let art = parse_artefact(src, Dialect::Adl2).unwrap();
        validate_flat_form(&art)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn vtvsmd_value_set_member_not_defined_in_flat_terminology() {
        // A value-set member (`at0.5`) that is not in the flat term_definitions
        // (`master07` §Validity Rules VTVSMD): the specialised flat-form half.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.flat_vtvsmd.v1.0.0

specialize
\topenEHR-EHR-OBSERVATION.parent.v1

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1.1] matches {*}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"ac0.1\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
\tvalue_sets = <
\t\t[\"ac0.1\"] = <id=<\"ac0.1\"> members=<\"at0.5\">>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vtvsmd));
    }

    #[test]
    fn vcosu_duplicate_node_id_across_flat_form() {
        // The same id-code `id2` at two non-sibling object paths in the flat form
        // (`master04.5` §C_OBJECT VCOSU — archetype-wide uniqueness).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.flat_vcosu.v1.0.0

specialize
\topenEHR-EHR-OBSERVATION.parent.v1

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1.1] matches {
\t\tdata matches {
\t\t\tHISTORY[id2] occurrences matches {0..1}
\t\t}
\t\tprotocol matches {
\t\t\tITEM_TREE[id2] occurrences matches {0..1}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vcosu));
    }

    /// Cloning is what makes raw id counting unsound, so the identity a VCOSU
    /// occurrence is keyed on collapses redefined codes but not new-at-level
    /// ones (`master08` §Flattening; `ADL2/master09.05` §Flattening).
    #[test]
    fn the_vcosu_identity_path_collapses_clones_but_not_added_nodes() {
        assert_eq!(
            specialisation_root_path("/data/events[id3]/data/items[id4.1]/value[id11]"),
            "/data/events[id3]/data/items[id4]/value[id11]"
        );
        assert_eq!(
            specialisation_root_path("/items[id4.1.2]/value[id11]"),
            "/items[id4]/value[id11]"
        );
        // A node ADDED at level 1 specialises nothing, so it keeps its code and
        // two added siblings stay distinct identities.
        assert_eq!(
            specialisation_root_path("/items[id0.1]/value[id5]"),
            "/items[id0.1]/value[id5]"
        );
        assert_ne!(
            specialisation_root_path("/items[id0.1]/value[id5]"),
            specialisation_root_path("/items[id0.2]/value[id5]")
        );
    }

    #[test]
    fn vacmcu_child_occurrences_exceed_flat_cardinality() {
        // A child occurrences upper (2) above the container cardinality upper (1)
        // in the flat form (`master04.5` §C_ATTRIBUTE VACMCU).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.flat_vacmcu.v1.0.0

specialize
\topenEHR-EHR-OBSERVATION.parent.v1

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1.1] matches {
\t\tdata cardinality matches {0..1} matches {
\t\t\tHISTORY[id2] occurrences matches {0..2}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vacmcu));
    }

    #[test]
    fn vatdf_value_at_code_not_defined_in_flat_terminology() {
        // A value at-code (`at0.9`) used in a `defining_code` leaf but absent from
        // the flat term_definitions (`master03` §Validity Rules VATDF — the
        // specialised flat-form half).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.flat_vatdf.v1.0.0

specialize
\topenEHR-EHR-OBSERVATION.parent.v1

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1.1] matches {
\t\tdata matches {
\t\t\tELEMENT[id2] matches {
\t\t\t\tvalue matches {
\t\t\t\t\tDV_CODED_TEXT[id3] matches {
\t\t\t\t\t\tdefining_code matches {[at0.9]}
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).contains(&ValidationCode::Vatdf));
    }

    #[test]
    fn clean_flat_form_raises_nothing() {
        // A well-formed flat form raises none of the deferred checks.
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.flat_ok.v1.0.0

specialize
\topenEHR-EHR-OBSERVATION.parent.v1

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1.1] matches {
\t\tdata cardinality matches {0..2} matches {
\t\t\tHISTORY[id2] occurrences matches {0..1}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        assert!(codes(src).is_empty());
    }
}
