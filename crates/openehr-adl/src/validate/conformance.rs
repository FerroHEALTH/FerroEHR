// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The validator's reading of the AOM2 conformance functions.
//!
//! The conformance functions themselves are AOM2 spec functions and live on the
//! generated classes (`openehr_am::v2_4::aom2::constraint_model`, realized from
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance Semantics). This module is what the phase-2 specialisation
//! validator adds on top of them: the tri-state
//! `ValueConformance`/`TupleConformance` verdicts its issue codes need, the
//! VSONCO collective-occurrences computation, the VSONT meta-type rule, and the
//! ADL 1.4 effective-value accessors.
//!
//! Context the spec functions read through the model's `parent` back-references
//! is passed explicitly (owning attribute, grand-parent RM type), because the
//! assembler leaves those unset.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;

use super::rm::RmModel;
use crate::aom::access::{AomType, aom_type, child_occurrences, object_node_id};
use crate::aom::build::cobject_to_primitive;
use crate::aom::interval::{Bounds, bounds};
use crate::paths::locate;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// `object_multiplicity` (`master04.5` §Occurrences inferencing rules, L219-236):
/// the effective object multiplicity of objects at `attr` within `rm_type` from
/// the reference model — `(0, cardinality.upper)` for a container, else the
/// attribute existence.
#[must_use]
pub(crate) fn object_multiplicity(rm: &dyn RmModel, rm_type: &str, attr: &str) -> Bounds {
    match rm.attribute(rm_type, attr) {
        Some(a) if a.is_multiple => match a.cardinality {
            Some(c) => Bounds::new(0, c.upper),
            None => Bounds::new(0, None),
        },
        Some(a) => a.existence,
        None => Bounds::new(0, None),
    }
}

/// The [`Bounds`] reading of `C_OBJECT.effective_occurrences` for `obj`.
///
/// The spec function is
/// [`CObject::effective_occurrences`]; this supplies its two pieces of context —
/// `owning_attr` (the Eiffel `parent`) and `grandparent_rm_type` (the Eiffel
/// `parent.parent.rm_type_name`, empty when unknown) — and its `rm_prop_mult`
/// lambda, which `master04.5` §Occurrences inferencing rules places in the
/// reference-model schema rather than in the AOM ([`object_multiplicity`]).
#[must_use]
pub(crate) fn effective_occurrences(
    obj: &CObject,
    owning_attr: &CAttribute,
    grandparent_rm_type: &str,
    rm: &dyn RmModel,
) -> Bounds {
    let owner = (!grandparent_rm_type.is_empty()).then_some(grandparent_rm_type);
    bounds(
        &obj.effective_occurrences(Some(owning_attr), owner, &|rm_type, attr| {
            object_multiplicity(rm, rm_type, attr).to_multiplicity_interval()
        }),
    )
}

/// `collective_occurrences_of` (`master04.5` §Conformance Semantics: `C_ATTRIBUTE`,
/// L82-118 + VSONCO L359-379): the collective occurrences of all object nodes
/// under `attr` that redefine the parent node identified by `parent_node_id`.
///
/// - lower = Σ of member lowers;
/// - upper = min(Σ of member uppers [unbounded if any member is unbounded],
///   the owning attribute's flattened cardinality upper).
///
/// A member with no local `occurrences` override inherits the redefined parent
/// node's occurrences (`parent_occ`). `flattened_card_upper` is the finite upper
/// bound of the owning attribute's flattened cardinality (`None` = unbounded).
#[must_use]
pub(crate) fn collective_occurrences_of(
    attr: &CAttribute,
    parent_node_id: &str,
    parent_occ: Bounds,
    flattened_card_upper: Option<i32>,
) -> Bounds {
    let mut lower: i64 = 0;
    let mut upper_sum: Option<i64> = Some(0);
    for child in attr.children.iter().flatten() {
        if !AdlCodeDefinitionsData::codes_conformant(object_node_id(child), parent_node_id) {
            continue;
        }
        let member = child_occurrences(child).map_or(parent_occ, bounds);
        lower += i64::from(member.lower);
        match (upper_sum, member.upper) {
            (Some(sum), Some(u)) => upper_sum = Some(sum + i64::from(u)),
            // any unbounded member makes the sum unbounded.
            (_, None) => upper_sum = None,
            (None, _) => {}
        }
    }
    // Cap by the owning attribute's flattened cardinality upper.
    let card_upper = flattened_card_upper;
    let upper = match (upper_sum, card_upper) {
        (Some(sum), Some(cu)) => Some(i64::from(cu).min(sum)),
        (Some(sum), None) => Some(sum),
        (None, Some(cu)) => Some(i64::from(cu)),
        (None, None) => None,
    };
    Bounds::new(
        i32::try_from(lower).unwrap_or(i32::MAX),
        upper.map(|u| i32::try_from(u).unwrap_or(i32::MAX)),
    )
}

/// The EFFECTIVE `existence` of an attribute in an **ADL 1.4** text: the stated
/// `existence` if there is one, else the 1.4 default `{1..1}`.
///
/// `ADL1.4/master05-cadl.adoc` §Existence L210: "The default existence
/// constraint, if none is shown, is {1..1}." ADL 1.4 states the default in the
/// formalism itself, so it is supplied as an ACCESSOR over the parsed model — the
/// parsed structure is never rewritten, and an absent `existence` stays absent in
/// the AOM (which is what the 1.4→2 converter and the printer must see).
#[must_use]
pub fn effective_existence_adl14(attr: &CAttribute) -> Bounds {
    attr.existence.as_ref().map_or(BOUNDS_ONE, bounds)
}

/// The EFFECTIVE `occurrences` of an object node in an **ADL 1.4** text.
///
/// - A stated `occurrences` wins.
/// - A `use_node` internal reference with none takes the referenced node's:
///   `ADL1.4/master05-cadl.adoc` §Internal References L515 — "Unlike other node
///   types, if no `occurrences` is mentioned, the value of the `occurrences` is
///   set to that of the referenced node (which if not explicitly mentioned will be
///   the default occurrences)". `root` is the archetype's definition root, against
///   which the proxy's target path resolves; a non-specialised 1.4 archetype is
///   its own flat form, so the path resolves locally.
/// - Otherwise the 1.4 default `{1..1}`: `ADL1.4/master05-cadl.adoc`
///   §Occurrences L316 — "The default occurrences, if none is mentioned, is
///   `{1..1}`".
///
/// As with [`effective_existence_adl14`], this is an accessor: nothing is written
/// back into the parsed model.
#[must_use]
pub(crate) fn effective_occurrences_adl14(root: &CComplexObject, obj: &CObject) -> Bounds {
    if let Some(occ) = child_occurrences(obj) {
        return bounds(occ);
    }
    if let CObject::CComplexObjectProxy(proxy) = obj
        && let Some(target) = locate(root, &proxy.target_path)
        && let Some(occ) = child_occurrences(target)
    {
        return bounds(occ);
    }
    BOUNDS_ONE
}

/// The ADL 1.4 default multiplicity `{1..1}`, shared by the existence
/// (`master05` L210) and occurrences (`master05` L316) defaults.
const BOUNDS_ONE: Bounds = Bounds {
    lower: 1,
    upper: Some(1),
};

/// VSONT meta-type conformance (`master04.5` §Validity Rules: `C_OBJECT`, VSONT
/// L342): the child meta-type must equal the parent's, with three exceptions —
/// a childless `C_COMPLEX_OBJECT` parent admits any non-primitive; a
/// `C_COMPLEX_OBJECT_PROXY` parent admits a `C_COMPLEX_OBJECT`; an
/// `ARCHETYPE_SLOT` parent admits a `C_ARCHETYPE_ROOT` (slot filling).
#[must_use]
pub(crate) fn meta_type_conforms(child: &CObject, parent: &CObject) -> bool {
    let (ct, pt) = (aom_type(child), aom_type(parent));
    if ct == pt {
        return true;
    }
    match parent {
        // A childless `C_COMPLEX_OBJECT` may be redefined by any non-primitive.
        CObject::CComplexObject(CComplexObject::CComplexObject(d))
            if d.attributes.as_ref().is_none_or(Vec::is_empty)
                && d.attribute_tuples.as_ref().is_none_or(Vec::is_empty) =>
        {
            !ct.is_primitive()
        }
        CObject::CComplexObjectProxy(_) => ct == AomType::ComplexObject,
        CObject::ArchetypeSlot(_) => ct == AomType::ArchetypeRoot,
        _ => false,
    }
}

/// The outcome of a value-constraint conformance test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueConformance {
    /// The child value constraint is the same as, or narrower than, the parent's.
    Conforms,
    /// The child value constraint is definitely wider than / outside the parent's
    /// (→ VPOV).
    Violates,
    /// Conformance cannot be determined (opaque / incomparable constraint forms,
    /// or mismatched primitive types) (→ VUNK).
    Unknown,
}

/// The validator's tri-state reading of `C_PRIMITIVE_OBJECT.c_value_conforms_to`
/// for two leaf nodes.
///
/// The comparison itself is the spec function
/// ([`CPrimitiveObject::c_value_conforms_to`], with
/// [`CPrimitiveObject::c_value_congruent_to`] restoring the "same as OR narrower
/// than" reading the specialisation rules need — `master04.5` states
/// `c_value_conforms_to` as a STRICT narrowing and treats the equal case as
/// congruence). Two decidability gates sit in front of it, because a `False`
/// from a vacuous test is not evidence of a violation:
///
/// - different primitive AOM types are a meta-type question (VSONT), not a
///   value one;
/// - a child that states no constraint against a constraining parent satisfies
///   the strict-subset test vacuously, which says nothing about the value space.
#[must_use]
fn value_conformance(child: &CPrimitiveObject, parent: &CPrimitiveObject) -> ValueConformance {
    if child.constrained_typename() != parent.constrained_typename() {
        return ValueConformance::Unknown;
    }
    if states_a_constraint(parent) && !states_a_constraint(child) {
        return ValueConformance::Unknown;
    }
    if child.c_value_conforms_to(parent) || child.c_value_congruent_to(parent) {
        ValueConformance::Conforms
    } else {
        ValueConformance::Violates
    }
}

/// Whether a primitive leaf states any value constraint of its own — the
/// negation of `any_allowed` (`master04.5` §Conformance semantics per
/// primitive), read per leaf type because `any_allowed` is redefined down the
/// hierarchy.
fn states_a_constraint(node: &CPrimitiveObject) -> bool {
    match node {
        CPrimitiveObject::CBoolean(c) => !c.any_allowed(),
        CPrimitiveObject::CString(c) => !c.any_allowed(),
        CPrimitiveObject::CTerminologyCode(c) => !c.any_allowed(),
        CPrimitiveObject::CDate(c) => {
            !c.constraint.as_ref().is_none_or(Vec::is_empty)
                || !c.pattern_constraint.as_deref().is_none_or(str::is_empty)
        }
        CPrimitiveObject::CDateTime(c) => {
            !c.constraint.as_ref().is_none_or(Vec::is_empty)
                || !c.pattern_constraint.as_deref().is_none_or(str::is_empty)
        }
        CPrimitiveObject::CDuration(c) => {
            !c.constraint.as_ref().is_none_or(Vec::is_empty)
                || !c.pattern_constraint.as_deref().is_none_or(str::is_empty)
        }
        CPrimitiveObject::CTime(c) => {
            !c.constraint.as_ref().is_none_or(Vec::is_empty)
                || !c.pattern_constraint.as_deref().is_none_or(str::is_empty)
        }
        CPrimitiveObject::CInteger(c) => !c.constraint.as_ref().is_none_or(Vec::is_empty),
        CPrimitiveObject::CReal(c) => !c.constraint.as_ref().is_none_or(Vec::is_empty),
    }
}

/// [`value_conformance`] for two object nodes, `Unknown` unless both are
/// primitive leaves.
#[must_use]
pub(crate) fn c_value_conforms_to(child: &CObject, parent: &CObject) -> ValueConformance {
    match (cobject_to_primitive(child), cobject_to_primitive(parent)) {
        (Some(c), Some(p)) => value_conformance(&c, &p),
        _ => ValueConformance::Unknown,
    }
}

/// The verdict of the second-order tuple conformance functions (`master04.5`
/// §`C_SECOND_ORDER`, L729-804).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TupleConformance {
    /// The child tuple conforms to its counterpart, has no counterpart in the
    /// parent node (a new second-order constraint), or cannot be disproved.
    Conforms,
    /// The child tuple constrains a different member-attribute group than the
    /// counterpart it overlaps, so no row of it can satisfy the
    /// `count = other.count` precondition of `C_PRIMITIVE_TUPLE.c_conforms_to`.
    GroupMismatch,
    /// A child tuple row carries a member count other than the member-attribute
    /// group's — the same precondition, at the row.
    RowArityMismatch,
    /// Comparable, but some child tuple row narrows no parent row —
    /// `C_ATTRIBUTE_TUPLE.c_conforms_to` is False.
    RowViolates,
}

/// The RM attribute names of a tuple's member attributes, in declaration order.
///
/// Each `C_PRIMITIVE_TUPLE` member corresponds positionally to one of these
/// (`master04.5` §`C_PRIMITIVE_TUPLE`).
#[must_use]
pub(crate) fn tuple_member_names(tuple: &CAttributeTuple) -> Vec<&str> {
    tuple
        .members
        .iter()
        .flatten()
        .map(|a| a.rm_attribute_name.as_str())
        .collect()
}

/// `c_conforms_to` for a `C_ATTRIBUTE_TUPLE` against the tuple set of the
/// corresponding flat-parent node (`master04.5` §`C_SECOND_ORDER`, L756-804).
///
/// The counterpart is the parent tuple constraining the same member-attribute
/// group; declaration order may differ, so members are paired by attribute
/// name. A child tuple sharing no attribute with any parent tuple is a new
/// second-order constraint and conforms.
///
/// `C_ATTRIBUTE_TUPLE.c_conforms_to` requires every child row to conform to
/// some parent row (the second `c_congruent_to` disjunct is subsumed: a
/// congruent row conforms); `C_PRIMITIVE_TUPLE.c_conforms_to` requires equal
/// member counts and, position-wise, `same_type` plus member value conformance.
/// A member pair whose value conformance is undecidable
/// ([`ValueConformance::Unknown`]) leaves the row unrefuted.
#[must_use]
pub(crate) fn tuple_conforms_to(
    child: &CAttributeTuple,
    parent_tuples: &[CAttributeTuple],
) -> TupleConformance {
    let names = tuple_member_names(child);
    // The counterpart is the parent tuple over the same group; failing that,
    // one that overlaps it (the group the child then fails to restate).
    let Some(counterpart) = parent_tuples
        .iter()
        .find(|p| member_order(&names, &tuple_member_names(p)).is_some())
        .or_else(|| {
            parent_tuples
                .iter()
                .find(|p| tuple_member_names(p).iter().any(|n| names.contains(n)))
        })
    else {
        return TupleConformance::Conforms;
    };
    let Some(order) = member_order(&names, &tuple_member_names(counterpart)) else {
        return TupleConformance::GroupMismatch;
    };
    // Rows of the counterpart carrying one member per member attribute; a
    // malformed parent tuple leaves the child unrefuted.
    let parent_rows: Vec<&CPrimitiveTuple> = counterpart
        .tuples
        .iter()
        .flatten()
        .filter(|r| r.members.len() == order.len())
        .collect();
    if parent_rows.is_empty() {
        return TupleConformance::Conforms;
    }
    let mut verdict = TupleConformance::Conforms;
    for row in child.tuples.iter().flatten() {
        if row.members.len() != order.len() {
            return TupleConformance::RowArityMismatch;
        }
        let refuted = parent_rows
            .iter()
            .all(|p| row_conforms_to(row, p, &order) == ValueConformance::Violates);
        if refuted {
            verdict = TupleConformance::RowViolates;
        }
    }
    verdict
}

/// The child→parent member position map of two tuple member-attribute lists, or
/// `None` if the two do not constrain the same attribute group.
fn member_order(names: &[&str], parent_names: &[&str]) -> Option<Vec<usize>> {
    if names.len() != parent_names.len() {
        return None;
    }
    names
        .iter()
        .map(|n| parent_names.iter().position(|p| p == n))
        .collect()
}

/// `c_conforms_to` for one `C_PRIMITIVE_TUPLE` row against a parent row
/// (`master04.5` §`C_SECOND_ORDER`, L783-792), with `order` mapping each child
/// member position to the parent row position of the same attribute.
fn row_conforms_to(
    row: &CPrimitiveTuple,
    parent_row: &CPrimitiveTuple,
    order: &[usize],
) -> ValueConformance {
    let mut worst = ValueConformance::Conforms;
    for (i, member) in row.members.iter().enumerate() {
        let Some(other) = order.get(i).and_then(|j| parent_row.members.get(*j)) else {
            return ValueConformance::Violates;
        };
        // `same_type` (L785): a member of a different primitive type never
        // conforms.
        if member.constrained_typename() != other.constrained_typename() {
            return ValueConformance::Violates;
        }
        match value_conformance(member, other) {
            ValueConformance::Conforms => {}
            ValueConformance::Violates => return ValueConformance::Violates,
            ValueConformance::Unknown => worst = ValueConformance::Unknown,
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aom::access::complex_attributes;
    use crate::assemble::parse_artefact;
    use crate::parse::Dialect;
    use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
    use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
    use openehr_base::prelude::MultiplicityInterval;

    fn mi(lower: i32, upper: Option<i32>) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(lower),
            upper,
            lower_unbounded: false,
            upper_unbounded: upper.is_none(),
            lower_included: true,
            upper_included: upper.is_some(),
        }
    }

    /// An attribute carrying only an `existence`, for the VSANCE assertions.
    fn attr_with_existence(existence: Option<MultiplicityInterval>) -> CAttribute {
        CAttribute {
            parent: None,
            soc_parent: None,
            rm_attribute_name: "items".to_owned(),
            existence,
            children: None,
            differential_path: None,
            is_multiple: false,
            cardinality: None,
        }
    }

    /// An object node carrying only an `occurrences`, for the VSONCO
    /// assertions.
    fn object_with_occurrences(occurrences: Option<MultiplicityInterval>) -> CObject {
        CObject::CComplexObject(CComplexObject::CComplexObject(
            openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObjectData {
                parent: None,
                soc_parent: None,
                rm_type_name: "ELEMENT".to_owned(),
                occurrences,
                node_id: "id2".to_owned(),
                alternative_ids: None,
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                attributes: None,
                attribute_tuples: None,
            },
        ))
    }

    #[test]
    fn existence_conformance_is_containment() {
        // {0} does not conform to {1} (VSANCE example: redefining {1} → {0}).
        assert!(
            !attr_with_existence(Some(mi(0, Some(0))))
                .existence_conforms_to(&attr_with_existence(Some(mi(1, Some(1)))))
        );
        // {1} conforms to {0..1}.
        assert!(
            attr_with_existence(Some(mi(1, Some(1))))
                .existence_conforms_to(&attr_with_existence(Some(mi(0, Some(1)))))
        );
        // unset always conforms.
        assert!(
            attr_with_existence(None)
                .existence_conforms_to(&attr_with_existence(Some(mi(1, Some(1)))))
        );
    }

    #[test]
    fn occurrences_single_occurrence_containment() {
        // parent {0..1} (upper 1): child {1..*} is NOT contained → non-conform.
        assert!(
            !object_with_occurrences(Some(mi(1, None)))
                .occurrences_conforms_to(&object_with_occurrences(Some(mi(0, Some(1)))))
        );
        // parent {0..1}: child {0..1} contained → conform.
        assert!(
            object_with_occurrences(Some(mi(0, Some(1))))
                .occurrences_conforms_to(&object_with_occurrences(Some(mi(0, Some(1)))))
        );
        // parent upper > 1 (multiple): the single-occurrence rule does not apply
        // here (VSONCO handles it) → always True.
        assert!(
            object_with_occurrences(Some(mi(5, Some(9))))
                .occurrences_conforms_to(&object_with_occurrences(Some(mi(0, Some(3)))))
        );
    }

    /// Build the definition root of a tiny 1.4 archetype, for the ADL 1.4
    /// effective-value tests.
    fn definition_of_adl14(src: &str) -> CComplexObject {
        let art = parse_artefact(src, Dialect::Adl14).unwrap();
        match art {
            Archetype::AuthoredArchetype(a) => match *a {
                AuthoredArchetype::AuthoredArchetype(d) => d.definition,
                AuthoredArchetype::Template(t) => t.definition,
                AuthoredArchetype::OperationalTemplate(o) => o.definition,
            },
            Archetype::TemplateOverlay(t) => t.definition,
        }
    }

    /// The ADL 1.4 defaults are EFFECTIVE values, applied by the accessor and
    /// never written back: `ADL1.4/master05-cadl.adoc` §Existence L210 ("The
    /// default existence constraint, if none is shown, is {1..1}") and
    /// §Occurrences L316 ("The default occurrences, if none is mentioned, is
    /// `{1..1}`").
    #[test]
    fn adl14_effective_defaults_are_one_to_one() {
        let root = definition_of_adl14(ADL14_USE_NODE_SRC);
        let items = complex_attributes(&root)
            .iter()
            .find(|a| a.rm_attribute_name == "items")
            .unwrap()
            .clone();
        // The attribute states no existence ⇒ effective {1..1} (L210); the parsed
        // structure keeps it absent.
        assert!(items.existence.is_none());
        assert_eq!(effective_existence_adl14(&items), Bounds::new(1, Some(1)));

        // at0001 states no occurrences ⇒ effective {1..1} (L316).
        let plain = &items.children.as_deref().unwrap_or_default()[0];
        assert!(child_occurrences(plain).is_none());
        assert_eq!(
            effective_occurrences_adl14(&root, plain),
            Bounds::new(1, Some(1))
        );
        // at0002 states {0..2} ⇒ that wins.
        assert_eq!(
            effective_occurrences_adl14(&root, &items.children.as_deref().unwrap_or_default()[1]),
            Bounds::new(0, Some(2))
        );
    }

    /// `ADL1.4/master05-cadl.adoc` §Internal References L515: a `use_node` with no
    /// stated `occurrences` takes the REFERENCED node's — here at0002's {0..2},
    /// not the {1..1} default.
    #[test]
    fn adl14_use_node_inherits_the_referenced_node_occurrences() {
        let root = definition_of_adl14(ADL14_USE_NODE_SRC);
        let items = complex_attributes(&root)
            .iter()
            .find(|a| a.rm_attribute_name == "items")
            .unwrap()
            .clone();
        let proxy = &items.children.as_deref().unwrap_or_default()[2];
        assert!(child_occurrences(proxy).is_none());
        assert_eq!(
            effective_occurrences_adl14(&root, proxy),
            Bounds::new(0, Some(2))
        );
    }

    const ADL14_USE_NODE_SRC: &str = "\
archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.effective.v1

concept
\t[at0000]

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"AuthorDraft\">

definition
\tCLUSTER[at0000] matches {
\t\titems cardinality matches {0..*; unordered} matches {
\t\t\tELEMENT[at0001] matches {*}
\t\t\tELEMENT[at0002] occurrences matches {0..2} matches {*}
\t\t\tuse_node ELEMENT /items[at0002]
\t\t}
\t}

ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"\"> description=<\"\">>
\t\t\t\t[\"at0001\"] = <text=<\"\"> description=<\"\">>
\t\t\t\t[\"at0002\"] = <text=<\"\"> description=<\"\">>
\t\t\t>
\t\t>
\t>
";

    #[test]
    fn node_id_conformance_uses_codes_conformant() {
        assert!(AdlCodeDefinitionsData::codes_conformant("id3.1", "id3"));
        assert!(!AdlCodeDefinitionsData::codes_conformant("id4", "id3"));
    }

    /// Build a single-attribute `C_ATTRIBUTE` from a tiny archetype's root, for
    /// the collective-occurrences tests.
    fn attr_of(src: &str, attr_name: &str) -> CAttribute {
        let art = parse_artefact(src, Dialect::Adl2).unwrap();
        let def = match art {
            Archetype::AuthoredArchetype(a) => match *a {
                AuthoredArchetype::AuthoredArchetype(d) => d.definition,
                AuthoredArchetype::Template(t) => t.definition,
                AuthoredArchetype::OperationalTemplate(o) => o.definition,
            },
            Archetype::TemplateOverlay(t) => t.definition,
        };
        complex_attributes(&def)
            .iter()
            .find(|a| a.rm_attribute_name == attr_name)
            .unwrap()
            .clone()
    }

    #[test]
    fn collective_occurrences_sums_and_caps() {
        // Two children redefining id3 with occurrences {0..2} and {1..3}, under a
        // container with cardinality {0..*}: collective lower = 0+1 = 1, upper =
        // 2+3 = 5 (VSONCO L370-375).
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.coll.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1] matches {
\t\tdata cardinality matches {0..*} matches {
\t\t\tHISTORY[id3.1] occurrences matches {0..2}
\t\t\tHISTORY[id3.2] occurrences matches {1..3}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3.1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3.2\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        let attr = attr_of(src, "data");
        let coll = collective_occurrences_of(&attr, "id3", Bounds::new(0, Some(1)), None);
        assert_eq!(coll.lower, 1);
        assert_eq!(coll.upper, Some(5));
    }

    #[test]
    fn collective_occurrences_unbounded_member_caps_to_cardinality() {
        // A member with {1..*} makes the summed upper unbounded; the owning
        // attribute cardinality upper (4) caps it (VSONCO L373-379 — "min of …
        // upper bound of the flattened cardinality").
        let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-OBSERVATION.coll2.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tOBSERVATION[id1] matches {
\t\tdata cardinality matches {0..4} matches {
\t\t\tHISTORY[id3.1] occurrences matches {1..*}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3.1\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
        let attr = attr_of(src, "data");
        // The child attribute restates cardinality {0..4}: the flattened
        // cardinality upper (4) caps the unbounded member sum.
        let coll = collective_occurrences_of(&attr, "id3", Bounds::new(0, Some(1)), Some(4));
        assert_eq!(coll.lower, 1);
        assert_eq!(coll.upper, Some(4));
    }

    /// A `C_TERMINOLOGY_CODE` leaf carrying a constraint and a status.
    fn terminology_leaf(constraint: &str, status: i32) -> CPrimitiveObject {
        CPrimitiveObject::CTerminologyCode(
            openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode {
                parent: None,
                soc_parent: None,
                rm_type_name: "DV_CODED_TEXT".to_owned(),
                occurrences: None,
                node_id: "at9999".to_owned(),
                alternative_ids: None,
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint: constraint.to_owned(),
                constraint_status: Some(constraint_status_of(status)),
            },
        )
    }

    /// The `CONSTRAINT_STATUS` value for an effective status integer
    /// (`master04.5` §`C_TERMINOLOGY_NODE`: required 0, extensible 1,
    /// preferred 2, example 3).
    fn constraint_status_of(
        status: i32,
    ) -> openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus
    {
        use openehr_am::v2_4::aom2::constraint_model::primitive::constraint_status::ConstraintStatus;
        match status {
            1 => ConstraintStatus::Extensible,
            2 => ConstraintStatus::Preferred,
            3 => ConstraintStatus::Example,
            _ => ConstraintStatus::Required,
        }
    }

    /// A `C_DATE` leaf carrying a pattern and, optionally, a range constraint.
    fn date_leaf(pattern: Option<&str>, ranged: bool) -> CPrimitiveObject {
        use openehr_base::v1_3::foundation_types::interval::interval::Interval;
        use openehr_base::v1_3::foundation_types::interval::point_interval::PointInterval;
        use openehr_base::v1_3::foundation_types::time::iso8601_date::Iso8601Date;
        let constraint = ranged.then(|| {
            vec![Interval::PointInterval(PointInterval {
                lower: Some(Iso8601Date {
                    value: "2004-05-20".to_owned(),
                }),
                upper: Some(Iso8601Date {
                    value: "2004-05-20".to_owned(),
                }),
                lower_unbounded: false,
                upper_unbounded: false,
                lower_included: true,
                upper_included: true,
            })]
        });
        CPrimitiveObject::CDate(
            openehr_am::v2_4::aom2::constraint_model::primitive::c_date::CDate {
                parent: None,
                soc_parent: None,
                rm_type_name: "DV_DATE".to_owned(),
                occurrences: None,
                node_id: "at9999".to_owned(),
                alternative_ids: None,
                is_deprecated: None,
                sibling_order: None,
                default_value: None,
                assumed_value: None,
                is_enumerated_type_constraint: None,
                constraint,
                pattern_constraint: pattern.map(str::to_owned),
            },
        )
    }

    #[test]
    fn terminology_status_ordering() {
        // child example(3) vs parent required(0): 3 > 0 → violates.
        assert_eq!(
            value_conformance(
                &terminology_leaf("at0004", 3),
                &terminology_leaf("at0004", 0)
            ),
            ValueConformance::Violates
        );
        // parent non-required (extensible=1): child auto-conforms.
        assert_eq!(
            value_conformance(&terminology_leaf("ac2", 0), &terminology_leaf("ac1", 1)),
            ValueConformance::Conforms
        );
        // both required, child code conforms lexically.
        assert_eq!(
            value_conformance(
                &terminology_leaf("at0004.1", 0),
                &terminology_leaf("at0004", 0)
            ),
            ValueConformance::Conforms
        );
        // both required, child code does not conform.
        assert_eq!(
            value_conformance(
                &terminology_leaf("at0005", 0),
                &terminology_leaf("at0004", 0)
            ),
            ValueConformance::Violates
        );
    }

    /// The temporal pattern rules, through `C_TEMPORAL.c_value_conforms_to` and
    /// the `C_DATE` replacement table (`c_temporal_definitions.adoc`
    /// §Attributes).
    ///
    /// The last case is a child stating a date RANGE under a parent stating only
    /// a PATTERN: `C_TEMPORAL.any_allowed` is False for a patterned parent, so
    /// the `C_ORDERED` precursor finds no parent interval containing the child's
    /// and the spec answer is a violation.
    #[test]
    fn temporal_pattern_replacement() {
        // Parent `yyyy-??-??` (month/day optional): child `yyyy-mm-dd` narrows
        // both to mandatory → valid replacement (master04.5 §C_TEMPORAL).
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm-dd"), false),
                &date_leaf(Some("yyyy-??-??"), false)
            ),
            ValueConformance::Conforms
        );
        // Child may also exclude an optional field.
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm-XX"), false),
                &date_leaf(Some("yyyy-??-??"), false)
            ),
            ValueConformance::Conforms
        );
        // Parent mandatory day cannot be loosened to optional.
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm-??"), false),
                &date_leaf(Some("yyyy-mm-dd"), false)
            ),
            ValueConformance::Violates
        );
        // Separator / layout mismatch is not a valid replacement.
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm"), false),
                &date_leaf(Some("yyyy-mm-dd"), false)
            ),
            ValueConformance::Violates
        );
        // An unconstrained parent admits any pattern.
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm-dd"), false),
                &date_leaf(None, false)
            ),
            ValueConformance::Conforms
        );
        assert_eq!(
            value_conformance(
                &date_leaf(Some("yyyy-mm-dd"), true),
                &date_leaf(Some("yyyy-??-??"), false)
            ),
            ValueConformance::Violates
        );
    }
}
