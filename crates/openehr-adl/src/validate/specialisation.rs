// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A differential child archetype against its flat parent, plus the header
//! checks that need the parent (VACSD, VASID, VALC).
//!
//! `AOM2/master08-validation.adoc` §Phase 2 → Validate Specialised Definition,
//! over [`super::conformance`]. Nodes correspond by path congruence
//! (`ADL2/master09.02` §Path Congruence). Slots live in [`super::slots`], and
//! VUNT in [`super::rm`], since an [`RmModel`] decides it.

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
use openehr_base::prelude::MultiplicityInterval;

use super::catalogue::ValidationCode;
use super::conformance::{
    self, TupleConformance, ValueConformance, collective_occurrences_of, effective_occurrences,
    meta_type_conforms, tuple_conforms_to, tuple_member_names,
};
use super::identification::languages;
use super::rm::RmModel;
use super::structure::complex_attribute_tuples;
use super::{ValidationIssue, push_issue};
use crate::aom::access::{
    aom_type, child_occurrences, complex_attributes, complex_rm_type, object_node_id,
    object_rm_type, sibling_order,
};
use crate::aom::interval::{Bounds, display_bounds_always_range, finite_cardinality_upper};
use crate::artefact::{ArchetypeRepository, ArchetypeView, view};
use crate::codes::{is_new_at_level, specialisation_depth};
use crate::hrid::{hrid_lookup_key, raw_id_lookup_key};
use crate::paths::{PathSegment, child_path, parse_path};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// Validate the differential child `child` against its flat parent `flat_parent`
/// (master08 "phase 2 — validate specialised definition" in the spec's guide
/// vocabulary). `repo` resolves external references for VARXR.
#[must_use]
pub(super) fn validate_against_flat_parent<'a>(
    child: &'a Archetype,
    flat_parent: &'a Archetype,
    rm: &'a dyn RmModel,
    repo: &'a ArchetypeRepository,
) -> Vec<ValidationIssue> {
    let cv = view(child);
    let pv = view(flat_parent);
    let mut scan = ParentScan {
        rm,
        repo,
        referrer_namespace: cv.archetype_id.namespace.as_deref(),
        child_level: cv.specialisation_level(),
        child_value_sets: value_set_members(cv.terminology),
        parent_value_sets: value_set_members(pv.terminology),
        issues: Vec::new(),
    };
    let parent_rm = complex_rm_type(pv.definition).to_owned();
    scan.walk_attributes(cv.definition, pv.definition, &parent_rm, "");
    scan.issues
}

/// The value-set membership map (`ac-code` → member codes) of an archetype
/// terminology, for the VPOV `value_set_expanded` subset check (`master04.5`
/// §`C_TERMINOLOGY_NODE` L683-690).
fn value_set_members(
    term: &openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology,
) -> std::collections::BTreeMap<String, Vec<String>> {
    term.value_sets
        .as_ref()
        .map(|vs| {
            vs.values()
                .map(|set| (set.id.clone(), set.members.to_vec()))
                .collect()
        })
        .unwrap_or_default()
}

/// Mutable state threaded through the specialisation walk. The archetype data is
/// borrowed for `'a` (independent of the `&mut self` used to append issues), so
/// the walk can hold parent/child node references while pushing findings.
pub(super) struct ParentScan<'a> {
    rm: &'a dyn RmModel,
    pub(super) repo: &'a ArchetypeRepository,
    /// The child's namespace, which a reference without one takes.
    pub(super) referrer_namespace: Option<&'a str>,
    child_level: usize,
    /// The child archetype's value-set membership (`ac-code` → members).
    child_value_sets: std::collections::BTreeMap<String, Vec<String>>,
    /// The flat parent's value-set membership (`ac-code` → members).
    parent_value_sets: std::collections::BTreeMap<String, Vec<String>>,
    pub(super) issues: Vec<ValidationIssue>,
}

impl<'a> ParentScan<'a> {
    /// Walk the attributes of a child complex object against the corresponding
    /// parent complex object `parent_obj` (RM type `parent_rm`), at `base_path`.
    fn walk_attributes(
        &mut self,
        child_obj: &'a CComplexObject,
        parent_obj: &'a CComplexObject,
        parent_rm: &str,
        base_path: &str,
    ) {
        self.check_attribute_tuples(child_obj, parent_obj, base_path);
        for attr in complex_attributes(child_obj) {
            self.check_attribute(attr, parent_obj, parent_rm, base_path);
        }
    }

    /// VTPNC / VTPIN: second-order tuple conformance to the flat parent
    /// ([`tuple_conforms_to`], `master04.5` §`C_SECOND_ORDER`).
    ///
    /// NOTE: `master08` §Phase 2 glosses neither code: our own split, VTPIN for an
    /// incomparable tuple, VTPNC for a comparable one refused.
    fn check_attribute_tuples(
        &mut self,
        child_obj: &'a CComplexObject,
        parent_obj: &'a CComplexObject,
        path: &str,
    ) {
        let parent_tuples = complex_attribute_tuples(parent_obj);
        for tuple in complex_attribute_tuples(child_obj) {
            let group = tuple_member_names(tuple).join(", ");
            match tuple_conforms_to(tuple, parent_tuples) {
                TupleConformance::Conforms => {}
                TupleConformance::GroupMismatch => push_issue(
                    &mut self.issues,
                    ValidationCode::Vtpin,
                    format!(
                        "tuple [{group}] redefines a parent tuple over a different attribute group"
                    ),
                    path,
                ),
                TupleConformance::RowArityMismatch => push_issue(
                    &mut self.issues,
                    ValidationCode::Vtpin,
                    format!("a tuple [{group}] row does not carry one member per attribute"),
                    path,
                ),
                TupleConformance::RowViolates => push_issue(
                    &mut self.issues,
                    ValidationCode::Vtpnc,
                    format!("a tuple [{group}] row narrows no row of the parent tuple"),
                    path,
                ),
            }
        }
    }

    /// Locate the parent attribute and its owning object for a child attribute,
    /// then run the attribute-level and object-level checks.
    fn check_attribute(
        &mut self,
        attr: &'a CAttribute,
        current_parent: &'a CComplexObject,
        current_parent_rm: &str,
        base_path: &str,
    ) {
        let attr_path = match attr.differential_path.as_deref() {
            Some(diff) => full_attr_path(base_path, diff, &attr.rm_attribute_name),
            None => format!("{base_path}/{}", attr.rm_attribute_name),
        };

        let Some((owner, owner_rm)) =
            self.resolve_owner(attr, current_parent, current_parent_rm, base_path)
        else {
            return;
        };

        let parent_attr = complex_attributes(owner)
            .iter()
            .find(|a| a.rm_attribute_name == attr.rm_attribute_name);

        let Some(parent_attr) = parent_attr else {
            self.check_absent_parent_attribute(attr, &attr_path);
            return;
        };

        self.check_attribute_multiplicity(attr, parent_attr, &owner_rm, &attr_path);

        // VSSM: sibling order node id must be in the same flat-parent container
        // (`master04.5` §`C_OBJECT`, VSSM L391).
        self.check_sibling_order(attr, parent_attr, &attr_path);

        // VSONCO (multiple-occurrence collective case) — `master04.5` §`C_OBJECT`
        // VSONCO L359-379, evaluated at the owning attribute.
        self.check_collective_occurrences(attr, parent_attr, &owner_rm, &attr_path);

        self.check_redefined_children(attr, parent_attr, &owner_rm, &attr_path);
    }

    /// Resolve the parent object a child attribute redefines within, with its RM
    /// type; `None` records VDIFP (`master04.5` §`C_ATTRIBUTE`).
    fn resolve_owner(
        &mut self,
        attr: &'a CAttribute,
        current_parent: &'a CComplexObject,
        current_parent_rm: &str,
        base_path: &str,
    ) -> Option<(&'a CComplexObject, String)> {
        let Some(diff) = attr.differential_path.as_deref() else {
            return Some((current_parent, current_parent_rm.to_owned()));
        };
        if let Some(o) = resolve_object(current_parent, diff) {
            return Some((o, complex_rm_type(o).to_owned()));
        }
        push_issue(
            &mut self.issues,
            ValidationCode::Vdifp,
            format!(
                "differential path {:?} does not resolve in the flat parent",
                full_attr_path(base_path, diff, &attr.rm_attribute_name)
            ),
            base_path,
        );
        None
    }

    /// Check a child attribute the flat parent's owning object does not declare:
    /// VDIFP for a differential path, else a new attribute of new nodes.
    fn check_absent_parent_attribute(&mut self, attr: &'a CAttribute, attr_path: &str) {
        if attr.differential_path.is_some() {
            push_issue(
                &mut self.issues,
                ValidationCode::Vdifp,
                format!(
                    "attribute {:?} of differential path does not exist in the flat parent",
                    attr.rm_attribute_name
                ),
                attr_path,
            );
            return;
        }
        for child in attr.children.iter().flatten() {
            self.check_new_object(child, attr_path);
        }
    }

    /// Pair each child object of a redefined attribute with its congruent parent
    /// node (or same-type primitive leaf); the rest are new nodes.
    fn check_redefined_children(
        &mut self,
        attr: &'a CAttribute,
        parent_attr: &'a CAttribute,
        owner_rm: &str,
        attr_path: &str,
    ) {
        for child in attr.children.iter().flatten() {
            let child_path = child_path(attr_path, object_node_id(child));
            if let Some(parent_obj) = find_congruent(parent_attr, child)
                .or_else(|| pair_primitive_leaf(parent_attr, child))
            {
                self.check_object_pair(child, parent_obj, attr, owner_rm, &child_path);
            } else {
                self.check_added_object(child, parent_attr, &child_path);
            }
        }
    }

    /// Check an object node the specialisation adds to a container attribute.
    fn check_added_object(
        &mut self,
        child: &'a CObject,
        parent_attr: &'a CAttribute,
        child_path: &str,
    ) {
        // VSONIF (`master04.5` §`C_OBJECT`): a new node beside identified siblings must
        // be identified.
        //
        // NOTE: master04.5 defers VSONIF to VACMI, which no vendored text defines; the
        // identification requirement is what this implements.
        if object_node_id(child).is_empty()
            && !aom_type(child).is_primitive()
            && parent_attr
                .children
                .iter()
                .flatten()
                .any(|p| !object_node_id(p).is_empty())
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonif,
                "a new object node in a specialised container must be identified (its flattened siblings are)",
                child_path,
            );
        }
        self.check_new_object(child, child_path);
    }

    /// Existence, cardinality and multiplicity conformance of a redefined
    /// attribute (`master04.5` §`C_ATTRIBUTE`: VSANCE, VSANCC, VSAM).
    fn check_attribute_multiplicity(
        &mut self,
        attr: &CAttribute,
        parent_attr: &CAttribute,
        owner_rm: &str,
        attr_path: &str,
    ) {
        if !attr.existence_conforms_to(parent_attr) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsance,
                "redefined existence does not conform to the flat parent",
                attr_path,
            );
        }
        if !attr.cardinality_conforms_to(parent_attr) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsancc,
                "redefined cardinality does not conform to the flat parent",
                attr_path,
            );
        }
        if attr.cardinality.is_some()
            && let Some(rm_attr) = self.rm.attribute(owner_rm, &attr.rm_attribute_name)
            && !rm_attr.is_multiple
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsam,
                "a redefined single-valued attribute cannot be given a cardinality (multiplicity mismatch)",
                attr_path,
            );
        }
    }

    /// The prohibition rules (`master04.5` §`C_OBJECT`): VSONPT needs the same AOM
    /// type, VSONPI exactly the parent node id.
    fn check_prohibited_redefinition(&mut self, child: &CObject, parent: &CObject, path: &str) {
        if aom_type(child) != aom_type(parent) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonpt,
                "prohibited (occurrences {0}) redefinition must match the parent AOM type",
                path,
            );
        }
        if object_node_id(child) != object_node_id(parent) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonpi,
                "prohibited redefinition must have the same node id as the parent node",
                path,
            );
        }
    }

    /// Object-level conformance of a redefined child node to its congruent flat
    /// parent node.
    fn check_object_pair(
        &mut self,
        child: &'a CObject,
        parent: &'a CObject,
        owning_attr: &'a CAttribute,
        owner_rm: &str,
        path: &str,
    ) {
        if is_prohibited(child_occurrences(child)) {
            self.check_prohibited_redefinition(child, parent, path);
        }

        // Slot handling: a redefinition of an `ARCHETYPE_SLOT` in the parent.
        if let CObject::ArchetypeSlot(parent_slot) = parent {
            self.check_slot_redefinition(child, parent_slot, path);
            return;
        }

        // Proxy handling: a `C_COMPLEX_OBJECT_PROXY` in the parent.
        if let CObject::CComplexObjectProxy(parent_proxy) = parent {
            self.check_proxy_redefinition(child, parent_proxy, path);
            return;
        }

        // VCORMT (`master04.5` §`C_OBJECT`): a CODE_PHRASE leaf cannot become a
        // non-terminology primitive; checked before VSONT.
        if matches!(parent, CObject::CTerminologyCode(_))
            && aom_type(child).is_primitive()
            && !matches!(child, CObject::CTerminologyCode(_))
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcormt,
                "a terminology-code (CODE_PHRASE) node cannot be redefined by a non-terminology primitive",
                path,
            );
            return;
        }

        // VSONT: meta-type conformance (`master04.5` §`C_OBJECT`, VSONT L342).
        if !meta_type_conforms(child, parent) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsont,
                format!(
                    "redefined node AOM type {:?} does not conform to the parent AOM type {:?}",
                    aom_type(child),
                    aom_type(parent)
                ),
                path,
            );
            return;
        }

        // Single-occurrence VSONCO (`master04.5` §`C_OBJECT`, occurrences_conforms_to
        // L287-299): a child redefining a single-occurrence (upper 1) parent node
        // must be wholly contained.
        let parent_occ = effective_occurrences(parent, owning_attr, owner_rm, self.rm);
        if parent_occ.upper == Some(1)
            && let Some(child_occ) = child_occurrences(child)
            && !parent_occ.contains(crate::aom::interval::bounds(child_occ))
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonco,
                "redefined occurrences of a single-occurrence parent node is not wholly contained",
                path,
            );
        }

        // Reference-model type conformance of the redefined object.
        self.check_rm_type_conformance(child, parent, owning_attr, owner_rm, path);

        if aom_type(child).is_primitive() && aom_type(parent).is_primitive() {
            self.check_leaf_value_redefinition(child, parent, path);
        }

        // Recurse into complex children (VSONT already passed).
        if let (CObject::CComplexObject(cco), CObject::CComplexObject(pco)) = (child, parent) {
            self.walk_attributes(cco, pco, complex_rm_type(pco), path);
        }
    }

    /// Leaf value redefinition (VPOV / VUNK) for a primitive node pair; terminology
    /// codes compare with value-set expansion (`master04.5` §`C_TERMINOLOGY_NODE`).
    fn check_leaf_value_redefinition(&mut self, child: &CObject, parent: &CObject, path: &str) {
        if let (CObject::CTerminologyCode(c), CObject::CTerminologyCode(p)) = (child, parent) {
            self.check_terminology_leaf(c, p, path);
            return;
        }
        match conformance::c_value_conforms_to(child, parent) {
            ValueConformance::Conforms => {}
            ValueConformance::Violates => push_issue(
                &mut self.issues,
                ValidationCode::Vpov,
                "redefined leaf value constraint is not within the parent value constraint",
                path,
            ),
            ValueConformance::Unknown => push_issue(
                &mut self.issues,
                ValidationCode::Vunk,
                "redefined leaf value constraint cannot be verified against the parent",
                path,
            ),
        }
    }

    /// VPOV: `C_TERMINOLOGY_CODE` leaf value conformance with value-set expansion
    /// (`master04.5` §`C_TERMINOLOGY_NODE` `c_value_conforms_to`).
    ///
    /// A child's status must not exceed the parent's; a non-required parent
    /// constrains nothing; two required codes must be `codes_conformant`, and a
    /// child value set must be a subset of the parent's expansion.
    fn check_terminology_leaf(
        &mut self,
        child: &CTerminologyCode,
        parent: &CTerminologyCode,
        path: &str,
    ) {
        let child_code = child.constraint.split('@').next().unwrap_or("").trim();
        let parent_code = parent.constraint.split('@').next().unwrap_or("").trim();
        // Parent `any_allowed` ⇒ conforms.
        if parent_code.is_empty() {
            return;
        }
        let child_status = child.constraint_status.map_or(0, ConstraintStatus::value);
        let parent_status = parent.constraint_status.map_or(0, ConstraintStatus::value);
        if child_status > parent_status {
            push_issue(
                &mut self.issues,
                ValidationCode::Vpov,
                "redefined terminology constraint status is weaker than the parent",
                path,
            );
            return;
        }
        // A non-required parent imposes no real constraint.
        if parent_status > 0 {
            return;
        }
        // Both required: lexical code conformance first.
        if !AdlCodeDefinitionsData::codes_conformant(child_code, parent_code) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vpov,
                format!(
                    "redefined terminology code {child_code:?} does not conform to the parent code {parent_code:?}"
                ),
                path,
            );
            return;
        }
        // Value-set expansion subset (only when the parent constrains a value-set).
        if AdlCodeDefinitionsData::is_value_set_code(parent_code)
            && let Some(parent_members) = self.parent_value_sets.get(parent_code)
            && !parent_members.is_empty()
        {
            let child_members = self.expand_child_value_set(child_code);
            if let Some(missing) = child_members.iter().find(|m| !parent_members.contains(m)) {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vpov,
                    format!(
                        "redefined value-set member {missing:?} is not in the parent value-set {parent_code:?}"
                    ),
                    path,
                );
            }
        }
    }

    /// The expanded member set of a child `C_TERMINOLOGY_CODE` constraint: an
    /// `ac-code` expands to its child value-set members, an `at-code` (or any
    /// non-value-set constraint) is its own singleton value set (`master04.5`
    /// §`C_TERMINOLOGY_NODE` `value_set_expanded`).
    fn expand_child_value_set(&self, code: &str) -> Vec<String> {
        if AdlCodeDefinitionsData::is_value_set_code(code)
            && let Some(members) = self.child_value_sets.get(code)
        {
            return members.clone();
        }
        vec![code.to_owned()]
    }

    /// VCORMT, then VSONCT: the redefined node's type conforms to its attribute's
    /// RM type and to the parent node's type (`master04.5` §`C_OBJECT`).
    fn check_rm_type_conformance(
        &mut self,
        child: &'a CObject,
        parent: &'a CObject,
        owning_attr: &'a CAttribute,
        owner_rm: &str,
        path: &str,
    ) {
        let child_rm = object_rm_type(child);
        if child_rm.is_empty() {
            return;
        }
        // VCORMT: conformance to the owning attribute's RM-declared type.
        if let Some(rm_attr) = self.rm.attribute(owner_rm, &owning_attr.rm_attribute_name)
            && self.rm.conforms(child_rm, &rm_attr.declared_type) == Some(false)
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcormt,
                format!(
                    "redefined object type {child_rm:?} does not conform to the reference-model attribute type {:?}",
                    rm_attr.declared_type
                ),
                path,
            );
            return;
        }
        // VSONCT: conformance to the parent node's RM type.
        let parent_rm = object_rm_type(parent);
        if !parent_rm.is_empty() && self.rm.conforms(child_rm, parent_rm) == Some(false) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonct,
                format!(
                    "redefined object type {child_rm:?} does not conform to the parent node type {parent_rm:?}"
                ),
                path,
            );
        }
    }

    /// VSONCO collective-occurrences rule for the container (multiple-occurrence)
    /// case (`master04.5` §`C_OBJECT` VSONCO L359-379).
    fn check_collective_occurrences(
        &mut self,
        child_attr: &'a CAttribute,
        parent_attr: &'a CAttribute,
        owner_rm: &str,
        path: &str,
    ) {
        // The flattened cardinality upper = the child's restated cardinality if
        // present, else the parent's.
        let flat_card_upper =
            finite_cardinality_upper(child_attr).or_else(|| finite_cardinality_upper(parent_attr));

        for parent_obj in parent_attr.children.iter().flatten() {
            let parent_id = object_node_id(parent_obj);
            let parent_occ = effective_occurrences(parent_obj, parent_attr, owner_rm, self.rm);
            // Multiple-occurrence parent node: upper is not exactly 1.
            if parent_occ.upper == Some(1) {
                continue;
            }
            // Members = child nodes redefining this parent node.
            let has_members =
                child_attr.children.iter().flatten().any(|c| {
                    AdlCodeDefinitionsData::codes_conformant(object_node_id(c), parent_id)
                });
            if !has_members {
                continue;
            }
            let coll =
                collective_occurrences_of(child_attr, parent_id, parent_occ, flat_card_upper);
            if !bounds_intersect(coll, parent_occ) {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vsonco,
                    format!(
                        "collective occurrences {} of the specialised node set do not intersect the parent occurrences {}",
                        display_bounds_always_range(coll),
                        display_bounds_always_range(parent_occ)
                    ),
                    path,
                );
            }
        }
    }

    /// VSSM: a sibling-order marker's node id must refer to a node found within
    /// the same container in the flat parent, or a node redefined locally
    /// (`master04.5` §`C_OBJECT`, VSSM L391).
    fn check_sibling_order(
        &mut self,
        child_attr: &'a CAttribute,
        parent_attr: &'a CAttribute,
        path: &str,
    ) {
        for child in child_attr.children.iter().flatten() {
            let Some(order) = sibling_order(child) else {
                continue;
            };
            let anchor = &order.sibling_node_id;
            let in_parent = parent_attr.children.iter().flatten().any(|p| {
                AdlCodeDefinitionsData::codes_conformant(anchor, object_node_id(p))
                    || object_node_id(p) == anchor
            });
            let redefined_locally = child_attr
                .children
                .iter()
                .flatten()
                .any(|c| object_node_id(c) == anchor);
            if !in_parent && !redefined_locally {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vssm,
                    format!(
                        "sibling order anchor {anchor:?} is not a node in the same flat-parent container"
                    ),
                    path,
                );
            }
        }
    }

    /// VSUNT / VUNT for a redefinition of a `C_COMPLEX_OBJECT_PROXY` parent node
    /// (`master04.5` §`C_COMPLEX_OBJECT_PROXY`, VSUNT L488 / VUNT L479-480).
    fn check_proxy_redefinition(
        &mut self,
        child: &'a CObject,
        _parent_proxy: &'a CComplexObjectProxy,
        path: &str,
    ) {
        // VSUNT: a proxy may be redefined by another proxy, or by a
        // `C_COMPLEX_OBJECT` that legally redefines the proxy target.
        match child {
            CObject::CComplexObjectProxy(_) | CObject::CComplexObject(_) => {}
            _ => push_issue(
                &mut self.issues,
                ValidationCode::Vsunt,
                "a use_node proxy may only be redefined by a proxy or a C_COMPLEX_OBJECT",
                path,
            ),
        }
    }

    /// A new child node (`master04.5` §`C_OBJECT` VSONIN, VSONPO); primitive leaves,
    /// whose node ids are synthetic, are exempt.
    fn check_new_object(&mut self, child: &'a CObject, path: &str) {
        if aom_type(child).is_primitive() {
            return;
        }
        // VSONPO: a new node's occurrences may not be prohibited (`{0}`) — that
        // only makes sense for an existing node.
        if is_prohibited(child_occurrences(child)) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonpo,
                "a new object node may not have prohibited (occurrences {0}) redefinition",
                path,
            );
        }
        // VSONIN: a new node carrying a node id must use a 'new' node id
        // specialised at the child level (`at0.*`/`id0.*` form). A redefinition-
        // style id with no parent counterpart is invalid.
        let nid = object_node_id(child);
        if !nid.is_empty()
            && !matches!(child, CObject::ArchetypeSlot(_))
            && (specialisation_depth(nid) != Some(self.child_level) || !is_new_at_level(nid))
        {
            // A node id that is a redefinition of a parent code but has no parent
            // counterpart, or is not specialised to the child level.
            push_issue(
                &mut self.issues,
                ValidationCode::Vsonin,
                format!(
                    "new object node id {nid:?} is not a valid new node id at specialisation level {}",
                    self.child_level
                ),
                path,
            );
        }
    }
}

/// Resolve a differential path from `start`, the flat-parent object that owns
/// the attribute carrying it, to its target object, using path congruence
/// (`codes_conformant`) for the node-id predicates.
fn resolve_object<'a>(start: &'a CComplexObject, path: &str) -> Option<&'a CComplexObject> {
    let segments = parse_path(path);
    let mut current = start;
    for seg in &segments {
        let attr = complex_attributes(current)
            .iter()
            .find(|a| a.rm_attribute_name == seg.attribute)?;
        let child = pick_child(attr, seg)?;
        match child {
            CObject::CComplexObject(cco) => current = cco,
            _ => return None,
        }
    }
    Some(current)
}

/// Choose the child object of `attr` matching a path segment's node-id predicate
/// (congruent match), or the sole child if the segment carries no predicate.
fn pick_child<'a>(attr: &'a CAttribute, seg: &PathSegment) -> Option<&'a CObject> {
    match &seg.node_id {
        Some(nid) => attr.children.iter().flatten().find(|c| {
            object_node_id(c) == nid
                || AdlCodeDefinitionsData::codes_conformant(nid, object_node_id(c))
        }),
        None if attr.children.as_ref().map_or(0, Vec::len) == 1 => {
            attr.children.iter().flatten().next()
        }
        None => None,
    }
}

/// Find the flat-parent node under `parent_attr` that `child` redefines
/// (`node_id_conforms_to` — the child id is the same as, or a specialisation of,
/// the parent id).
fn find_congruent<'a>(parent_attr: &'a CAttribute, child: &CObject) -> Option<&'a CObject> {
    let cid = object_node_id(child);
    if cid.is_empty() {
        return None;
    }
    parent_attr
        .children
        .iter()
        .flatten()
        .find(|p| AdlCodeDefinitionsData::codes_conformant(cid, object_node_id(p)))
}

/// Pair a child primitive leaf (which carries only a synthetic node id) with the
/// parent attribute's leaf of the same primitive AOM type, so a leaf value
/// redefinition (VPOV/VUNK) can be compared. Only used when node-id matching
/// ([`find_congruent`]) found no counterpart.
fn pair_primitive_leaf<'a>(parent_attr: &'a CAttribute, child: &CObject) -> Option<&'a CObject> {
    let ct = aom_type(child);
    if !ct.is_primitive() {
        return None;
    }
    if let Some(same) = parent_attr
        .children
        .iter()
        .flatten()
        .find(|p| aom_type(p) == ct)
    {
        return Some(same);
    }
    // A type-changing leaf redefinition pairs with the parent attribute's sole
    // primitive leaf, so VCORMT / VSONT fire.
    let mut prims = parent_attr
        .children
        .iter()
        .flatten()
        .filter(|p| aom_type(p).is_primitive());
    let first = prims.next()?;
    if prims.next().is_none() {
        Some(first)
    } else {
        None
    }
}

/// True if an occurrences interval is prohibited (`{0}` / `{0..0}`).
fn is_prohibited(occ: Option<&MultiplicityInterval>) -> bool {
    occ.is_some_and(MultiplicityInterval::is_prohibited)
}

/// True if two [`Bounds`] intervals intersect (share at least one integer).
fn bounds_intersect(a: Bounds, b: Bounds) -> bool {
    let a_ok = a.upper.is_none_or(|au| au >= b.lower);
    let b_ok = b.upper.is_none_or(|bu| bu >= a.lower);
    a_ok && b_ok
}

/// The full attribute path of a differential-path attribute: the owning
/// object's `base` path, the `diff` prefix and the attribute name.
fn full_attr_path(base: &str, diff: &str, name: &str) -> String {
    format!("{base}{diff}/{name}")
}

// ── the parent-dependent phase-1 header checks (VACSD / VASID / VALC) ─────

/// VACSD: the specialisation depth is one more than the parent's; VASID: the
/// `specialise` id is the immediate parent's (master03 §Validity Rules).
pub(super) fn check_specialisation_depth(
    v: &ArchetypeView<'_>,
    repo: Option<&ArchetypeRepository>,
    out: &mut Vec<ValidationIssue>,
) {
    let level = v.specialisation_level();
    let Some(parent_id) = v.parent_archetype_id else {
        // Not specialised — depth must be 0.
        if level != 0 {
            out.push(ValidationIssue::new(
                ValidationCode::Vacsd,
                format!(
                    "non-specialised archetype has root specialisation depth {level}, expected 0"
                ),
            ));
        }
        return;
    };

    // Specialised — a specialised archetype must be at depth >= 1 regardless.
    if level == 0 {
        out.push(ValidationIssue::new(
            ValidationCode::Vacsd,
            "specialised archetype has a level-0 root code (expected depth >= 1)",
        ));
    }

    let Some(repo) = repo else {
        return; // standalone validation: the parent-dependent half needs a repository
    };
    let Some(parent) = repo.resolve(parent_id, v.archetype_id.namespace.as_deref()) else {
        // An operational template is already the flat form (master08 §Phase 3):
        // its `specialise` clause records lineage, and no phase-2 conformance
        // against a parent applies to it.
        if v.kind == crate::source::ArtefactKind::OperationalTemplate {
            return;
        }
        // The stated id resolves to nothing, so it is not "the identifier of the
        // immediate specialisation parent archetype" (master03 VASID), and
        // phase 2 (master08 §Validation of Specialised Archetype Against Flat
        // Parent) cannot run: the archetype has not validated.
        out.push(ValidationIssue::new(
            ValidationCode::Vasid,
            format!(
                "stated parent archetype {parent_id:?} is not in the repository, so the \
                 specialised archetype cannot be validated against its flat parent"
            ),
        ));
        return;
    };
    let parent_view = view(parent);
    let parent_level = parent_view.specialisation_level();
    if level != parent_level + 1 {
        out.push(ValidationIssue::new(
            ValidationCode::Vacsd,
            format!("specialisation depth {level} is not one greater than the parent depth {parent_level}"),
        ));
    }
    // VASID: the stated parent id must be the immediate parent's id.
    let stated = raw_id_lookup_key(parent_id);
    let actual = hrid_lookup_key(parent_view.archetype_id);
    if stated != actual {
        out.push(ValidationIssue::new(
            ValidationCode::Vasid,
            format!("stated parent id {stated:?} is not the immediate parent id {actual:?}"),
        ));
    }
}

/// VALC: a specialised archetype's languages are a subset of the flat parent's
/// (master03 §Validity Rules; `ADL2/master09.02` §Differential and Flat Forms).
///
/// Without a buildable flat parent the declared parent's own languages are used.
pub(super) fn check_language_conformance(
    v: &ArchetypeView<'_>,
    repo: Option<&ArchetypeRepository>,
    out: &mut Vec<ValidationIssue>,
) {
    let (Some(parent_id), Some(repo)) = (v.parent_archetype_id, repo) else {
        return;
    };
    let Some(parent) = repo.resolve(parent_id, v.archetype_id.namespace.as_deref()) else {
        return;
    };
    let flat_parent = crate::flatten::flat_form(parent, repo).ok();
    let parent_langs = match flat_parent.as_ref() {
        Some(flat) => languages(&view(flat)),
        None => languages(&view(parent)),
    };
    for lang in languages(v) {
        if !parent_langs.contains(&lang) {
            out.push(ValidationIssue::new(
                ValidationCode::Valc,
                format!("language {lang:?} is not present in the flattened parent archetype"),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::parse_artefact;
    use crate::parse::Dialect;
    use crate::validate::rm::ProductionRmModel;

    /// The level-0 parent CLUSTER for the hand-written specialisation cases.
    const PARENT: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.p2_parent.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..*} matches {
\t\t\tELEMENT[id2] occurrences matches {0..1} matches {
\t\t\t\tvalue matches { DV_QUANTITY[id3] }
\t\t\t}
\t\t\tELEMENT[id4] occurrences matches {1..3} matches {
\t\t\t\tvalue matches { DV_TEXT[id5] matches { value matches {\"a\", \"b\", \"c\"} } }
\t\t\t}
\t\t\tallow_archetype CLUSTER[id6] matches {
\t\t\t\tinclude
\t\t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.foo.*\\.v1/}
\t\t\t}
\t\t\tallow_archetype CLUSTER[id9] matches {
\t\t\t\tinclude
\t\t\t\t\tarchetype_id/value matches {\"openEHR-EHR-CLUSTER.baz.v1\"}
\t\t\t}
\t\t\tallow_archetype CLUSTER[id8] closed
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id6\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id8\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id9\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";

    /// Wrap a differential `definition` body into a level-1 child specialising
    /// [`PARENT`].
    fn child(def: &str, terms: &str) -> String {
        format!(
            "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
             \topenEHR-EHR-CLUSTER.p2_child.v1.0.0\n\n\
             specialize\n\topenEHR-EHR-CLUSTER.p2_parent.v1\n\n\
             language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
             description\n\tlifecycle_state = <\"draft\">\n\n\
             definition\n\tCLUSTER[id1.1] matches {{\n{def}\n\t}}\n\n\
             terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n{terms}\t\t>\n\t>\n"
        )
    }

    fn term(code: &str) -> String {
        format!("\t\t\t[\"{code}\"] = <text=<\"x\"> description=<\"x\">>\n")
    }

    /// The error codes phase-2 raises for `def` (a child definition body).
    fn codes(def: &str, terms: &str) -> Vec<String> {
        let parent = parse_artefact(PARENT, Dialect::Adl2).unwrap();
        let child = parse_artefact(&child(def, terms), Dialect::Adl2).unwrap();
        let repo = ArchetypeRepository::default();
        let issues = validate_against_flat_parent(&child, &parent, &ProductionRmModel, &repo);
        issues
            .iter()
            .map(|i| i.code.mnemonic().to_owned())
            .collect()
    }

    fn assert_raises(def: &str, terms: &str, code: &str) {
        let raised = codes(def, terms);
        assert!(
            raised.iter().any(|c| c == code),
            "expected {code}, raised {raised:?}"
        );
    }

    fn assert_not_raised(def: &str, terms: &str, code: &str) {
        let raised = codes(def, terms);
        assert!(
            !raised.iter().any(|c| c == code),
            "expected no {code}, raised {raised:?}"
        );
    }

    #[test]
    fn vsam_cardinality_on_single_valued_attribute() {
        // Redefining the single-valued ELEMENT.value with a cardinality is a
        // multiplicity mismatch (`master04.5` §`C_ATTRIBUTE`, VSAM).
        assert_raises(
            "\t\t/items[id2]/value cardinality matches {0..*} matches { DV_QUANTITY[id3.1] }",
            &term("id3.1"),
            "VSAM",
        );
    }

    #[test]
    fn vsonct_reference_type_non_conformance() {
        // Redefining `DV_QUANTITY`[id3] to `DV_TEXT`: `DV_TEXT` conforms to the
        // attribute type `DATA_VALUE` (so not VCORMT) but not to the parent node
        // type `DV_QUANTITY` (`master04.5` §`C_OBJECT`, VSONCT).
        assert_raises(
            "\t\t/items[id2]/value matches { DV_TEXT[id3.1] }",
            &term("id3.1"),
            "VSONCT",
        );
    }

    #[test]
    fn vsont_meta_type_change() {
        // Redefining the ELEMENT[id2] complex node (which has child attributes)
        // with a `C_ARCHETYPE_ROOT` is an illegal AOM meta-type change (`master04.5`
        // §`C_OBJECT`, VSONT).
        assert_raises(
            "\t\t/items matches { use_archetype ELEMENT[id2.1, openEHR-EHR-CLUSTER.x.v1] }",
            &term("id2.1"),
            "VSONT",
        );
    }

    #[test]
    fn vsonpi_prohibition_wrong_node_id() {
        // Prohibiting (occurrences {0}) with a node id that is a specialisation of
        // (not identical to) the parent node id (`master04.5` §`C_OBJECT`, VSONPI).
        assert_raises(
            "\t\t/items matches { ELEMENT[id2.1] occurrences matches {0} }",
            &term("id2.1"),
            "VSONPI",
        );
    }

    #[test]
    fn vsonpo_new_node_prohibited() {
        // A brand-new node may not be prohibited — prohibition only makes sense
        // for an existing node (`master04.5` §`C_OBJECT`, VSONPO).
        assert_raises(
            "\t\t/items matches { ELEMENT[id0.9] occurrences matches {0} }",
            &term("id0.9"),
            "VSONPO",
        );
    }

    #[test]
    fn vsonpt_prohibition_wrong_aom_type() {
        // Prohibiting the slot node id6 with a `C_COMPLEX_OBJECT` (different AOM
        // type from `ARCHETYPE_SLOT`) (`master04.5` §`C_OBJECT`, VSONPT).
        assert_raises(
            "\t\t/items matches { CLUSTER[id6] occurrences matches {0} }",
            "",
            "VSONPT",
        );
    }

    #[test]
    fn vpov_leaf_value_out_of_parent() {
        // Redefining the `DV_TEXT` value list to include "z", not in the parent
        // list {"a","b","c"} (`master08` §Phase 2 gloss, VPOV, via
        // c_value_conforms_to).
        assert_raises(
            "\t\t/items[id4]/value matches { DV_TEXT[id5.1] matches { value matches {\"a\", \"z\"} } }",
            &term("id5.1"),
            "VPOV",
        );
    }

    #[test]
    fn vdssm_slot_neither_closed_nor_narrowed() {
        // A specialised slot that neither closes nor narrows the parent slot
        // (`master04.5` §`ARCHETYPE_SLOT`, VDSSM).
        assert_raises(
            "\t\t/items matches { allow_archetype CLUSTER[id6] }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vdssm_slot_restatement_is_not_a_proper_narrowing() {
        // Redefining slot id6 with an `include` identical to the parent's is a
        // restatement, not a proper narrowing (`master04.5` §`ARCHETYPE_SLOT`,
        // VDSSM).
        assert_raises(
            "\t\t/items matches { allow_archetype CLUSTER[id6] matches {\n\
             \t\t\tinclude\n\t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.foo.*\\.v1/}\n\t\t} }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vdssm_slot_widening_literal_not_admitted_by_parent() {
        // Redefining slot id6 with an `include` naming a literal archetype id the
        // parent slot does not admit widens the slot (not a subset) —
        // (`master04.5` §`ARCHETYPE_SLOT`, VDSSM).
        assert_raises(
            "\t\t/items matches { allow_archetype CLUSTER[id6] matches {\n\
             \t\t\tinclude\n\t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.bar\\.v1/}\n\t\t} }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vdssm_widening_literal_after_a_non_regex_include_is_still_caught() {
        // A slot admits the union of its includes (`ADL2/master04.3` §Archetype
        // Slots); the second include widens the parent (VDSSM).
        assert_raises(
            "\t\t/items matches { allow_archetype CLUSTER[id6] matches {\n\
             \t\t\tinclude\n\t\t\t\tarchetype_id/value matches {\"openEHR-EHR-CLUSTER.qux.v1\"}\n\
             \t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.bar\\.v1/}\n\t\t} }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vdssm_a_non_regex_include_is_not_itself_a_widening() {
        // The same slot narrowed to `foo1` (admitted by the parent's `foo.*`) plus
        // an include the regex reading cannot express: the unreadable assertion
        // contributes an unknown share of the child's set, which proves nothing —
        // VDSSM refutes only what it can decide (`master04.5` §`ARCHETYPE_SLOT`).
        assert_not_raised(
            "\t\t/items matches { allow_archetype CLUSTER[id6] matches {\n\
             \t\t\tinclude\n\t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.foo1\\.v1/}\n\
             \t\t\t\tarchetype_id/value matches {\"openEHR-EHR-CLUSTER.qux.v1\"}\n\t\t} }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vdssm_an_unreadable_parent_include_refutes_nothing() {
        // Slot id9's parent `include` is a literal value constraint, so the
        // admitted superset is unknown — an unknown superset cannot establish that
        // the child admits something outside it (`master04.5` §`ARCHETYPE_SLOT`,
        // VDSSM: a PROPER SUBSET of the parent's matched set).
        assert_not_raised(
            "\t\t/items matches { allow_archetype CLUSTER[id9] matches {\n\
             \t\t\tinclude\n\t\t\t\tarchetype_id/value matches {/openEHR-EHR-CLUSTER\\.other\\.v1/}\n\t\t} }",
            "",
            "VDSSM",
        );
    }

    #[test]
    fn vsonif_unidentified_new_node_among_identified_siblings() {
        // master04.5 §C_OBJECT VSONIF: a new object node added to a container whose
        // flattened siblings are identified must itself be identified. An
        // unidentified new node is only constructible in the AOM model (ADL2 source
        // requires node ids), so it is built by stripping the id off a parsed node.
        use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
        let parent = parse_artefact(PARENT, Dialect::Adl2).unwrap();
        let mut child = parse_artefact(
            &child("\t\titems matches { ELEMENT[id0.5] }", &term("id0.5")),
            Dialect::Adl2,
        )
        .unwrap();
        // Strip the new ELEMENT's node id to exercise the unidentified case.
        if let Archetype::AuthoredArchetype(inner) = &mut child
            && let AuthoredArchetype::AuthoredArchetype(data) = inner.as_mut()
            && let CComplexObject::CComplexObject(root) = &mut data.definition
        {
            for a in root
                .attributes
                .iter_mut()
                .flatten()
                .filter(|a| a.rm_attribute_name == "items")
            {
                strip_child_node_ids(a);
            }
        }
        let issues = validate_against_flat_parent(
            &child,
            &parent,
            &ProductionRmModel,
            &ArchetypeRepository::default(),
        );
        let raised: Vec<&str> = issues.iter().map(|i| i.code.mnemonic()).collect();
        assert!(raised.contains(&"VSONIF"), "raised {raised:?}");
    }

    /// Clears the node id of every complex-object child of `attr`.
    fn strip_child_node_ids(attr: &mut CAttribute) {
        for c in attr.children.iter_mut().flatten() {
            if let CObject::CComplexObject(CComplexObject::CComplexObject(el)) = c {
                el.node_id.clear();
            }
        }
    }

    #[test]
    fn vdssp_specialise_a_closed_slot() {
        // Specialising the already-closed slot id8 (`master04.5` §`ARCHETYPE_SLOT`,
        // VDSSP).
        assert_raises(
            "\t\t/items matches { allow_archetype CLUSTER[id8] closed }",
            "",
            "VDSSP",
        );
    }

    // ── second-order tuple conformance (master04.5 §C_SECOND_ORDER) ───────
    //
    // The vendored corpus has no VTPNC/VTPIN case, so these fixtures cover them.

    /// The tuple parent: `ELEMENT[id2]`'s `DV_QUANTITY[id3]` carries the
    /// two-row `[magnitude, units]` tuple of `ADL2/master09.05` §Tuple
    /// Redefinition; `ELEMENT[id4]`'s `DV_QUANTITY[id5]` carries none.
    const TUPLE_PARENT: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.p2_tuple_parent.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {0..*} matches {
\t\t\tELEMENT[id2] occurrences matches {0..1} matches {
\t\t\t\tvalue matches {
\t\t\t\t\tDV_QUANTITY[id3] matches {
\t\t\t\t\t\t[magnitude, units] matches {
\t\t\t\t\t\t\t[{|>=50.0|}, {\"mm[Hg]\"}],
\t\t\t\t\t\t\t[{|>=68.0|}, {\"cm[H20]\"}]
\t\t\t\t\t\t}
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t\tELEMENT[id4] occurrences matches {0..1} matches {
\t\t\t\tvalue matches { DV_QUANTITY[id5] }
\t\t\t}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";

    /// The error codes phase-2 raises for a child definition body specialising
    /// [`TUPLE_PARENT`].
    fn tuple_codes(def: &str) -> Vec<String> {
        let src = format!(
            "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
             \topenEHR-EHR-CLUSTER.p2_tuple_child.v1.0.0\n\n\
             specialize\n\topenEHR-EHR-CLUSTER.p2_tuple_parent.v1\n\n\
             language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
             description\n\tlifecycle_state = <\"draft\">\n\n\
             definition\n\tCLUSTER[id1.1] matches {{\n{def}\n\t}}\n\n\
             terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\t\t>\n\t>\n"
        );
        let parent = parse_artefact(TUPLE_PARENT, Dialect::Adl2).unwrap();
        let child = parse_artefact(&src, Dialect::Adl2).unwrap();
        let issues = validate_against_flat_parent(
            &child,
            &parent,
            &ProductionRmModel,
            &ArchetypeRepository::default(),
        );
        issues
            .iter()
            .map(|i| i.code.mnemonic().to_owned())
            .collect()
    }

    /// The child `DV_QUANTITY[id3]` body redefining `ELEMENT[id2]`'s value.
    fn quantity_child(tuple: &str) -> String {
        format!("\t\t/items[id2]/value matches {{ DV_QUANTITY[id3] matches {{\n{tuple}\n\t\t}} }}")
    }

    #[test]
    fn tuple_row_narrowing_conforms() {
        // Dropping a row and narrowing the surviving one is the sanctioned
        // narrowing (`master04.5` §`C_SECOND_ORDER` `c_conforms_to`,
        // `ADL2/master09.05` §Tuple Redefinition).
        let raised = tuple_codes(&quantity_child(
            "\t\t\t[magnitude, units] matches { [{|>=60.0|}, {\"mm[Hg]\"}] }",
        ));
        assert_eq!(raised, Vec::<String>::new());
    }

    #[test]
    fn tuple_over_an_unconstrained_parent_node_conforms() {
        // A child tuple where the parent has none is a new constraint, which
        // `master04.5` §`C_SECOND_ORDER` does not condemn.
        let raised = tuple_codes(
            "\t\t/items[id4]/value matches { DV_QUANTITY[id5] matches {\n\
             \t\t\t[magnitude, units] matches { [{|>=1.0|}, {\"mm\"}] }\n\t\t} }",
        );
        assert_eq!(raised, Vec::<String>::new());
    }

    #[test]
    fn vtpnc_row_narrows_no_parent_row() {
        // `>=20.0` is wider than the `>=50.0` of the only row whose units match,
        // so the row narrows no parent row (`master04.5` §`C_SECOND_ORDER`
        // `C_ATTRIBUTE_TUPLE.c_conforms_to`).
        let raised = tuple_codes(&quantity_child(
            "\t\t\t[magnitude, units] matches { [{|>=20.0|}, {\"mm[Hg]\"}] }",
        ));
        assert_eq!(raised, vec!["VTPNC".to_owned()]);
    }

    #[test]
    fn vtpnc_member_of_a_different_primitive_type() {
        // The `same_type` guard of `C_PRIMITIVE_TUPLE.c_conforms_to`
        // (`master04.5` §`C_SECOND_ORDER` L785): a `C_INTEGER` member cannot
        // narrow the parent's `C_REAL` member.
        let raised = tuple_codes(&quantity_child(
            "\t\t\t[magnitude, units] matches { [{60}, {\"mm[Hg]\"}] }",
        ));
        assert_eq!(raised, vec!["VTPNC".to_owned()]);
    }

    #[test]
    fn vtpin_different_member_attribute_group() {
        // A `[magnitude, units, precision]` tuple over the parent's
        // `[magnitude, units]` group can never satisfy the `count = other.count`
        // precondition (`master04.5` §`C_SECOND_ORDER` L784).
        let raised = tuple_codes(&quantity_child(
            "\t\t\t[magnitude, units, precision] matches { [{|>=50.0|}, {\"mm[Hg]\"}, {2}] }",
        ));
        assert_eq!(raised, vec!["VTPIN".to_owned()]);
    }

    #[test]
    fn vtpin_row_member_count_is_not_the_group_arity() {
        // Each row member corresponds to one member attribute (`master04.5`
        // §`C_PRIMITIVE_TUPLE`), so a one-member row under a two-attribute
        // group is not comparable with the parent's rows.
        let raised = tuple_codes(&quantity_child(
            "\t\t\t[magnitude, units] matches { [{|>=50.0|}] }",
        ));
        assert_eq!(raised, vec!["VTPIN".to_owned()]);
    }
}
