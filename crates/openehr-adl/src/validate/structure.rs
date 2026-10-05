// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Basic-integrity structural topic: the definition-tree walk and the rules that are
//! decidable from the archetype's own constraint structure — node identity and
//! uniqueness, sibling attribute uniqueness, differential-path placement,
//! container cardinality vs child occurrences, `C_ARCHETYPE_ROOT` shape, slot
//! include/exclude consistency, terminology-constraint code form, and primitive
//! assumed values.
//!
//! Rule texts:
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules (`C_OBJECT` / `C_COMPLEX_OBJECT` / `C_ATTRIBUTE` /
//! `ARCHETYPE_SLOT` / `C_PRIMITIVE_OBJECT`) and `master08-validation.adoc`
//! §Phase 1 - Basic Integrity; the ADL 1.4-only VCOC rule is
//! `ADL1.4/master05-cadl.adoc` §Occurrences.

use std::collections::{BTreeSet, HashMap};

use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::beom::core::assertion::Assertion;
use openehr_base::prelude::Interval;
use openehr_base::prelude::MultiplicityInterval;

use super::catalogue::ValidationCode;
use super::conformance::effective_occurrences_adl14;
use super::{ValidationIssue, push_issue};
use crate::aom::access::{aom_type, child_occurrences, complex_attributes, object_node_id};
use crate::aom::interval::finite_upper;
use crate::artefact::ArchetypeView;
use crate::hrid::is_archetype_id;
use crate::parse::Dialect;
use crate::paths::child_path;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// raises issues; the terminology checks re-derive code usage in a second pass
/// (`collect_usage`), so no code sets are accumulated here.
struct StructureScan<'a> {
    v: &'a ArchetypeView<'a>,
    dialect: Dialect,
    issues: Vec<ValidationIssue>,
    /// node id → first path seen (VCOSU uniqueness).
    seen_node_ids: HashMap<String, String>,
}

pub(super) fn check_structure(
    v: &ArchetypeView<'_>,
    dialect: Dialect,
    issues: &mut Vec<ValidationIssue>,
) {
    let mut scan = StructureScan {
        v,
        dialect,
        issues: Vec::new(),
        seen_node_ids: HashMap::new(),
    };
    let root = CObject::CComplexObject(v.definition.clone());
    // The root object always requires a node id (the concept code, `at0000`/
    // `id1`); child requirement is decided per owning attribute in
    // [`StructureScan::walk_attribute`].
    scan.walk_object("", &root, true);
    issues.append(&mut scan.issues);
}

impl StructureScan<'_> {
    fn walk_object(&mut self, path: &str, obj: &CObject, require_node_id: bool) {
        let nid = object_node_id(obj);
        let is_identified = !aom_type(obj).is_primitive();

        // VCOID: every non-primitive object node must have a node id
        // (master04.5 §`C_OBJECT`), relaxed under ADL 1.4 to the AOM 1.4 rule
        // (AOM1.4 master04 §Node_id and Paths). A 1.4 `use_node` is a reference,
        // not a node definition, so it is exempt in that dialect.
        let is_proxy_ref =
            self.dialect == Dialect::Adl14 && matches!(obj, CObject::CComplexObjectProxy(_));
        if is_identified && nid.is_empty() && require_node_id && !is_proxy_ref {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcoid,
                "object node has no node identifier",
                path,
            );
        }
        // VCOSU: object node ids must be unique archetype-wide (master04.5
        // §`C_OBJECT`); synthetic primitive ids are exempt. A specialised
        // archetype defers it to [`super::flat`], and the 1.4 dialect gets the
        // sibling-scoped check instead (AOM1.4 master04 §Node_id and Paths).
        if is_identified
            && !nid.is_empty()
            && !self.v.is_specialised()
            && self.dialect == Dialect::Adl2
        {
            if let Some(first) = self.seen_node_ids.get(nid) {
                let dup = format!("node id {nid:?} is not unique (also at {first})");
                push_issue(&mut self.issues, ValidationCode::Vcosu, dup, path);
            } else {
                self.seen_node_ids.insert(nid.to_owned(), path.to_owned());
            }
        }

        match obj {
            CObject::CComplexObject(cco) => self.walk_complex(path, cco),
            CObject::ArchetypeSlot(slot) => self.check_slot(path, slot),
            CObject::CTerminologyCode(tc) => {
                // VATCV (code form) applies only to ADL2 constraint codes; the
                // ADL 1.4 dialect preserves 1.4 terminology constraints
                // verbatim (`local, at0004`, `[openehr::524]`, listed forms) in
                // the constraint string — these are not ADL2 code forms, and
                // their validity is ontology-definedness (ADL1.4 master08
                // §Local Constraint Codes / VATDF/VACDF), not the ADL2 regex.
                if self.dialect == Dialect::Adl2 {
                    self.check_terminology_code_form(path, &tc.constraint);
                }
            }
            CObject::CBoolean(_)
            | CObject::CInteger(_)
            | CObject::CReal(_)
            | CObject::CString(_)
            | CObject::CDate(_)
            | CObject::CTime(_)
            | CObject::CDateTime(_)
            | CObject::CDuration(_) => self.check_primitive_assumed(path, obj),
            // NOTE: VUNP (`C_COMPLEX_OBJECT_PROXY` target-path validity) is a
            // flat-form (phase-3) check, so it runs in [`super::flat`]
            // (`master08` §Phase 3; `master04.5` VUNP), not in the phase-1 walk.
            CObject::CComplexObjectProxy(_) => {}
        }
    }

    fn walk_complex(&mut self, path: &str, cco: &CComplexObject) {
        // VARXNC / VARXAV / VARXTV: `C_ARCHETYPE_ROOT` validity (master08 §Phase 1
        // §Various Structure Validation).
        //
        // NOTE: VARXR (external-reference *resolution*) is a phase-2 check that
        // needs the supplier repository, so it runs in the specialisation
        // validator ([`super::slots`], `master08` §Phase 2), not here.
        if let CComplexObject::CArchetypeRoot(r) = cco {
            self.check_archetype_root(path, r);
        }

        // VCATU: sibling attributes uniquely named (master04.5 §`C_COMPLEX_OBJECT`).
        // In a differential archetype a root-level attribute is identified by
        // its whole differential path: `/items` and `/items[id9]/items` end in
        // the same RM attribute name yet address different nodes of the flat
        // parent (ADL2 master09.02 §Differential Paths).
        let mut seen_attrs = BTreeSet::new();
        for attr in complex_attributes(cco) {
            let key = (
                attr.differential_path.as_deref(),
                attr.rm_attribute_name.as_str(),
            );
            if !seen_attrs.insert(key) {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vcatu,
                    format!(
                        "attribute {:?} is defined more than once",
                        attr.differential_path
                            .as_deref()
                            .unwrap_or(&attr.rm_attribute_name)
                    ),
                    path,
                );
            }
        }

        for attr in complex_attributes(cco) {
            if self.dialect == Dialect::Adl14 {
                self.check_sibling_node_ids(path, attr);
            }
            self.walk_attribute(path, attr);
        }
    }

    /// VARXNC / VARXAV / VARXTV: `C_ARCHETYPE_ROOT` validity (master08 §Phase 1
    /// §Various Structure Validation).
    ///
    /// NOTE: VARXR (external-reference *resolution*) is a phase-2 check that
    /// needs the supplier repository, so it runs in the specialisation
    /// validator ([`super::slots`], `master08` §Phase 2), not here.
    fn check_archetype_root(
        &mut self,
        path: &str,
        r: &openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot,
    ) {
        if r.node_id.is_empty() {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxnc,
                "C_ARCHETYPE_ROOT has no node id",
                path,
            );
        }
        if r.rm_type_name.is_empty() {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxtv,
                "C_ARCHETYPE_ROOT has no RM type",
                path,
            );
        }
        if !r.archetype_ref.is_empty() && !is_archetype_id(&r.archetype_ref) {
            push_issue(
                &mut self.issues,
                ValidationCode::Varxav,
                format!(
                    "C_ARCHETYPE_ROOT reference {:?} is not a valid archetype id",
                    r.archetype_ref
                ),
                path,
            );
        }
    }

    /// VCOSU (AOM 1.4 sibling scope): in the 1.4 dialect node ids are only
    /// *sibling*-unique — children under the same container attribute must have
    /// distinct node ids (AOM1.4 master04 §`Node_id` and Paths — "guarantees
    /// sibling node unique identification").
    ///
    /// ADL2 uses the stronger archetype-wide uniqueness (`walk_object` / the
    /// flat-form walk), so this sibling-scoped pass is 1.4-only.
    fn check_sibling_node_ids(&mut self, path: &str, attr: &CAttribute) {
        let mut sibling_ids: BTreeSet<&str> = BTreeSet::new();
        for child in attr.children.iter().flatten() {
            let cid = object_node_id(child);
            if !cid.is_empty()
                && (AdlCodeDefinitionsData::is_id_code(cid)
                    || AdlCodeDefinitionsData::is_at_code(cid))
                && !sibling_ids.insert(cid)
            {
                let cpath = child_path(&format!("{path}/{}", attr.rm_attribute_name), cid);
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vcosu,
                    format!("node id {cid:?} is not unique among siblings"),
                    &cpath,
                );
            }
        }
    }

    fn walk_attribute(&mut self, parent_path: &str, attr: &CAttribute) {
        let attr_path = format!("{parent_path}/{}", attr.rm_attribute_name);

        // VDIFV: a differential path is only valid in a specialised archetype
        // (master04.5 §`C_ATTRIBUTE`).
        if attr.differential_path.is_some() && !self.v.is_specialised() {
            push_issue(
                &mut self.issues,
                ValidationCode::Vdifv,
                "differential path in a non-specialised archetype",
                &attr_path,
            );
        }

        // VACMCU/WACMCL compare a child's occurrences against its owning
        // attribute's STATED cardinality (master04.5 §`C_ATTRIBUTE`), so they
        // run only when a cardinality is present and the archetype is its own
        // flat form.
        //
        // NOTE: VACSO needs `C_ATTRIBUTE._is_multiple_`, which the parser's
        // cardinality heuristic cannot supply, so it runs in [`super::rm`].
        if !self.v.is_specialised() && attr.is_multiple {
            self.check_container_cardinality(&attr_path, attr);
        }

        // VCOC is the ADL 1.4 formalism's own cardinality/occurrences rule,
        // distinct from the AOM2 VACMCU/WACMCL pair above, so it runs on the 1.4
        // dialect only. Gated to the non-specialised (own-flat-form) case for the
        // same reason as VACMCU: a specialised archetype need not restate the
        // inherited cardinality, so the sums here would be computed against a
        // cardinality that is not the effective one.
        if self.dialect == Dialect::Adl14 && !self.v.is_specialised() {
            self.check_vcoc(&attr_path, attr);
        }

        // Whether a child object is required to carry a node id. AOM2 requires
        // one on every non-primitive object (master04.5 §`C_OBJECT`); AOM 1.4
        // requires one only for children of a container (multiple) attribute —
        // "any leaf or near-leaf node which has no sibling nodes from the same
        // attribute can safely have no node_id" (AOM1.4 master04 §Node_id and
        // Paths; ADL1.4 master08 §Definition Section).
        let require_child_node_id = match self.dialect {
            Dialect::Adl2 => true,
            Dialect::Adl14 => attr.is_multiple,
        };
        for child in attr.children.iter().flatten() {
            let cpath = child_path(&attr_path, object_node_id(child));
            self.walk_object(&cpath, child, require_child_node_id);
        }
    }

    /// VACMCU (error) + WACMCL (warning): container cardinality vs child
    /// occurrences (master04.5 §`C_ATTRIBUTE`).
    fn check_container_cardinality(&mut self, attr_path: &str, attr: &CAttribute) {
        let Some(card) = attr.cardinality.as_ref() else {
            return;
        };
        let Some(card_upper) = finite_upper(&card.interval) else {
            return; // open cardinality upper — nothing to bound
        };
        let mut sum_lower = 0i64;
        for child in attr.children.iter().flatten() {
            let Some(occ) = child_occurrences(child) else {
                continue;
            };
            // VACMCU: a finite child occurrences upper must be <= cardinality upper.
            if let Some(u) = finite_upper(occ)
                && i64::from(u) > i64::from(card_upper)
            {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vacmcu,
                    format!(
                        "child occurrences upper {u} exceeds the cardinality upper {card_upper}"
                    ),
                    attr_path,
                );
            }
            sum_lower += i64::from(occurrences_lower(occ));
        }
        // WACMCL: the sum of child occurrences lowers should be below the
        // cardinality upper (advisory warning).
        if sum_lower > i64::from(card_upper) {
            push_issue(
                &mut self.issues,
                ValidationCode::Wacmcl,
                format!(
                    "sum of child occurrences lowers {sum_lower} exceeds the cardinality upper {card_upper}"
                ),
                attr_path,
            );
        }
    }

    /// VCOC (ADL 1.4): cardinality/occurrences validity —
    /// `ADL1.4/master05-cadl.adoc` §Occurrences L321-324: "the interval
    /// represented by: (the sum of all occurrences minimum values) .. (the sum of
    /// all occurrences maximum values) must be inside the interval of the
    /// cardinality."
    ///
    /// The children's occurrences are the EFFECTIVE ones
    /// ([`effective_occurrences_adl14`]): a child with no stated `occurrences` is
    /// `{1..1}` (master05 L316), and a `use_node` with none takes the referenced
    /// node's (master05 L515) — without those defaults the sums would be computed
    /// from zero-width intervals and the rule would never see a real archetype's
    /// child set.
    ///
    /// NOTE: the sum-LOWER half of the containment (`cardinality.lower <= Σ
    /// occurrences.lower`) is NOT raised — AOM2 itself downgrades that half
    /// to the WARNING WACMCL while the upper half is the ERROR VACMCU
    /// (`AOM2/master04.5-constraint_model-class_definitions.adoc` §Validity
    /// Rules: `C_ATTRIBUTE`), and openEHR's own regression corpus passes
    /// archetypes violating it; the two genuine-defect halves (overfillable
    /// upper, unsatisfiable lower) are raised.
    fn check_vcoc(&mut self, attr_path: &str, attr: &CAttribute) {
        let Some(card) = attr.cardinality.as_ref() else {
            return; // no cardinality ⇒ not a container ⇒ VCOC does not apply.
        };
        if attr.children.as_ref().is_none_or(Vec::is_empty) {
            return;
        }
        let card_lower = occurrences_lower(&card.interval);
        let card_upper = finite_upper(&card.interval);

        // Σ of the children's effective occurrences maxima; `None` = unbounded
        // (any child with an open upper makes the sum open).
        let mut sum_upper: Option<i64> = Some(0);
        for child in attr.children.iter().flatten() {
            let occ = effective_occurrences_adl14(self.v.definition, child);
            match (sum_upper, occ.upper) {
                (Some(sum), Some(u)) => sum_upper = Some(sum + i64::from(u)),
                (_, None) => sum_upper = None,
                (None, _) => {}
            }
        }
        let Some(sum_upper) = sum_upper else {
            return; // an unbounded sum is inside any cardinality with an open upper
        };

        if let Some(cu) = card_upper
            && sum_upper > i64::from(cu)
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcoc,
                format!(
                    "the sum of the children's occurrences maxima ({sum_upper}) is outside the \
                     cardinality {card_lower}..{cu}"
                ),
                attr_path,
            );
        }
        if sum_upper < i64::from(card_lower) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcoc,
                format!(
                    "the sum of the children's occurrences maxima ({sum_upper}) cannot reach the \
                     cardinality lower bound ({card_lower})"
                ),
                attr_path,
            );
        }
    }

    fn check_slot(&mut self, path: &str, slot: &ArchetypeSlot) {
        // VDSEV / VDSIV: slot include/exclude consistency (master04.5
        // §`ARCHETYPE_SLOT`), the spec's if/elseif chain, so exactly one branch
        // fires.
        //
        // NOTE: with the include-side branches first, VDSIV is unreachable on a
        // real slot, which always has an `include`, so every inconsistency
        // reports as VDSEV even though the spec defines both.
        let inc_empty = slot.includes.as_ref().is_none_or(Vec::is_empty);
        let exc_empty = slot.excludes.as_ref().is_none_or(Vec::is_empty);
        let inc_any = !inc_empty && slot.includes.iter().flatten().all(is_any_assertion);
        let exc_any = !exc_empty && slot.excludes.iter().flatten().all(is_any_assertion);

        // A real slot always carries an `include`, so only the include-side
        // branches of the spec table are reachable (see the NOTE above): a
        // violation is reported as VDSEV whether the offending pairing is an
        // any-include/any-exclude or a specific-include/specific-exclude.
        if !inc_empty {
            let contradictory = if inc_any {
                exc_non_any_and_any(exc_empty, exc_any)
            } else {
                !(exc_empty || exc_any)
            };
            if contradictory {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vdsev,
                    "slot 'include' and 'exclude' constraints are contradictory",
                    path,
                );
            }
        }

        // VDFAI: archetype ids in slot assertions must be valid (master04.5
        // §`ARCHETYPE_SLOT`).
        for a in slot
            .includes
            .iter()
            .flatten()
            .chain(slot.excludes.iter().flatten())
        {
            for id in assertion_archetype_ids(a) {
                if !is_archetype_id(&id) {
                    push_issue(
                        &mut self.issues,
                        ValidationCode::Vdfai,
                        format!("slot assertion references an invalid archetype id {id:?}"),
                        path,
                    );
                }
            }
        }
    }

    /// VATCV: a terminology constraint code must be a well-formed code
    /// (master08 §Code Validation; NOTE-flagged, no full vendored text). The
    /// definedness / value-set / assumed-value checks run in the gated
    /// terminology pass ([`check_terminology`](super::terminology::check_terminology)).
    fn check_terminology_code_form(&mut self, path: &str, constraint: &str) {
        // Strip an optional operational `@terminology` binding suffix.
        let code = constraint.split('@').next().unwrap_or(constraint).trim();
        if !code.is_empty() && !AdlCodeDefinitionsData::is_valid_code(code) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vatcv,
                format!("terminology constraint code {code:?} is not a valid code"),
                path,
            );
        }
    }

    /// VOBAV: a primitive assumed value must fall within its own constraint
    /// (master04.5 §`C_PRIMITIVE_OBJECT`).
    ///
    /// The enumerable primitives (Boolean / String) test list membership; the
    /// ordered primitives (Integer / Real and the temporal `Iso8601_*` types)
    /// test point-in-interval containment (`master04.5` §`C_ORDERED` — the value
    /// space is a list of `Interval`s, `has` = a point falls in some interval).
    /// A primitive with an empty constraint (`any_allowed`) admits any assumed
    /// value. The temporal branches use `Interval::has_definite` so an
    /// incomparable (undecidable) value never raises a false violation.
    /// Raise VOBAV at `path` with `msg` — the shared shape of every arm of
    /// [`StructureScan::check_primitive_assumed`].
    fn vobav(&mut self, msg: &'static str, path: &str) {
        push_issue(&mut self.issues, ValidationCode::Vobav, msg, path);
    }

    fn check_primitive_assumed(&mut self, path: &str, obj: &CObject) {
        let violation = match obj {
            CObject::CBoolean(b) => b
                .assumed_value
                .is_some_and(|av| {
                    !b.constraint.as_ref().is_none_or(Vec::is_empty)
                        && !b.constraint.as_ref().is_some_and(|c| c.contains(&av))
                })
                .then_some("boolean assumed value is not in the constraint"),
            CObject::CInteger(i) => integer_assumed_violates(i)
                .then_some("integer assumed value is not within any constraint interval"),
            CObject::CReal(r) => r
                .assumed_value
                .is_some_and(|av| {
                    !r.constraint.as_ref().is_none_or(Vec::is_empty)
                        && !r.constraint.iter().flatten().any(|iv| iv.has(&av))
                })
                .then_some("real assumed value is not within any constraint interval"),
            CObject::CDate(d) => d
                .assumed_value
                .as_ref()
                .is_some_and(|av| {
                    temporal_assumed_violates(d.constraint.as_deref().unwrap_or_default(), av)
                })
                .then_some("date assumed value is not within any constraint interval"),
            CObject::CTime(t) => t
                .assumed_value
                .as_ref()
                .is_some_and(|av| {
                    temporal_assumed_violates(t.constraint.as_deref().unwrap_or_default(), av)
                })
                .then_some("time assumed value is not within any constraint interval"),
            CObject::CDateTime(dt) => dt
                .assumed_value
                .as_ref()
                .is_some_and(|av| {
                    temporal_assumed_violates(dt.constraint.as_deref().unwrap_or_default(), av)
                })
                .then_some("date/time assumed value is not within any constraint interval"),
            CObject::CDuration(du) => du
                .assumed_value
                .as_ref()
                .is_some_and(|av| {
                    temporal_assumed_violates(du.constraint.as_deref().unwrap_or_default(), av)
                })
                .then_some("duration assumed value is not within any constraint interval"),
            CObject::CString(s) => s
                .assumed_value
                .as_ref()
                .is_some_and(|av| {
                    !s.constraint.as_ref().is_none_or(Vec::is_empty)
                        && !s.constraint.iter().flatten().any(|c| c == av)
                })
                .then_some("string assumed value is not in the constraint list"),
            _ => None,
        };
        if let Some(msg) = violation {
            self.vobav(msg, path);
        }
    }
}

/// VOBAV for `C_INTEGER`: the generated model types the integer assumed value
/// as `f64`, so a valid assumed value is a whole number lying in some
/// constraint interval.
fn integer_assumed_violates(
    i: &openehr_am::v2_4::aom2::constraint_model::primitive::c_integer::CInteger,
) -> bool {
    let Some(av) = i.assumed_value else {
        return false;
    };
    if i.constraint.as_ref().is_none_or(Vec::is_empty) {
        return false;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "guarded by fract()"
    )]
    let inside = av.fract() == 0.0 && i.constraint.iter().flatten().any(|iv| iv.has(&(av as i32)));
    !inside
}

/// VOBAV for an ordered temporal primitive: a non-empty constraint list is
/// violated only when the assumed value is *definitely* outside every interval
/// (`Interval::has_definite` returns `Some(false)` for all of them). An
/// undecidable pairing (`None`, e.g. a partial date whose completions overlap a
/// bound) leaves containment unknown and never raises — mirroring the numeric
/// `has`-based check while staying conservative under partial order.
fn temporal_assumed_violates<T: PartialOrd>(constraint: &[Interval<T>], av: &T) -> bool {
    !constraint.is_empty()
        && constraint
            .iter()
            .all(|iv| iv.has_definite(av) == Some(false))
}

// ── helpers ───────────────────────────────────────────────────────────────

pub(super) fn occurrences_lower(mi: &MultiplicityInterval) -> i32 {
    mi.lower.unwrap_or(0)
}

/// True if a slot assertion expresses "any archetype" (its regex constraint is
/// a match-anything pattern), for the include/exclude consistency table.
fn is_any_assertion(a: &Assertion) -> bool {
    // The regex the assertion's `matches` constrains; an "any" slot is `/.*/`
    // or `/.+/`.
    crate::rules::slot_assertion_regex(a).is_some_and(|regex| {
        let regex = regex.trim();
        regex == ".*" || regex == ".+"
    })
}

/// Helper for the VDSEV branch-1 condition `not (excludes empty or /= any)`.
fn exc_non_any_and_any(exc_empty: bool, exc_any: bool) -> bool {
    !exc_empty && exc_any
}

/// The archetype-id literals referenced by a slot assertion (read from its
/// expression tree — the constraint targets an id via a regex).
fn assertion_archetype_ids(a: &Assertion) -> Vec<String> {
    // Slot assertions constrain `archetype_id/value matches {/regex/}`; the
    // regex, when it is a literal id (no meta-characters), is itself the id.
    // VDFAI's subject is the ARCHETYPE IDENTIFIER (ADL1.4 master05 §Archetype
    // Slots), so an assertion targeting another property (`domain_concept`,
    // `short_concept_name`, a path) constrains something that is not an
    // archetype id and yields none.
    let targets_archetype_id = crate::rules::slot_assertion_path(a)
        .is_some_and(|path| path.trim_start().starts_with("archetype_id"));
    if !targets_archetype_id {
        return Vec::new();
    }
    let Some(regex) = crate::rules::slot_assertion_regex(a) else {
        return Vec::new();
    };
    // A literal id regex contains no unescaped regex meta-characters beyond the
    // escaped `\.` dots.
    let literal = regex.replace("\\.", ".");
    if literal.is_empty() || literal.contains(['*', '+', '?', '(', ')', '[', ']', '|', '^', '$']) {
        Vec::new()
    } else {
        vec![literal]
    }
}

/// The second-order attribute tuples of a [`CComplexObject`] (either subtype).
pub(super) fn complex_attribute_tuples(cco: &CComplexObject) -> &[CAttributeTuple] {
    match cco {
        CComplexObject::CComplexObject(d) => d.attribute_tuples.as_deref().unwrap_or_default(),
        CComplexObject::CArchetypeRoot(r) => r.attribute_tuples.as_deref().unwrap_or_default(),
    }
}

#[cfg(test)]
mod temporal_vobav_tests {
    use openehr_base::prelude::{Interval, Iso8601Date, ProperInterval, ProperIntervalData};

    use super::temporal_assumed_violates;

    fn date(v: &str) -> Iso8601Date {
        Iso8601Date {
            value: v.to_owned(),
        }
    }

    /// A closed date interval `[lo, hi]`.
    fn date_interval(lo: &str, hi: &str) -> Interval<Iso8601Date> {
        Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
            lower: Some(date(lo)),
            upper: Some(date(hi)),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }))
    }

    #[test]
    fn assumed_date_outside_the_interval_raises() {
        let constraint = vec![date_interval("2020-01-01", "2020-12-31")];
        // 2019-06-15 is definitely before the interval ⇒ VOBAV fires.
        assert!(temporal_assumed_violates(&constraint, &date("2019-06-15")));
    }

    #[test]
    fn assumed_date_inside_the_interval_does_not_raise() {
        let constraint = vec![date_interval("2020-01-01", "2020-12-31")];
        assert!(!temporal_assumed_violates(&constraint, &date("2020-06-15")));
    }

    #[test]
    fn incomparable_assumed_date_does_not_raise() {
        let constraint = vec![date_interval("2020-01-01", "2020-12-31")];
        // The partial year 2020 overlaps the interval — containment is
        // undecidable, so it must NOT raise (honest incomparability).
        assert!(!temporal_assumed_violates(&constraint, &date("2020")));
    }

    #[test]
    fn empty_constraint_admits_any_assumed_value() {
        assert!(!temporal_assumed_violates::<Iso8601Date>(
            &[],
            &date("2019-06-15")
        ));
    }
}
