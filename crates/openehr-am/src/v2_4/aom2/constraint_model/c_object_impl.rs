// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_OBJECT` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_object.adoc` §Functions,
//! `AM/docs/AOM2/master07-terminology_package.adoc` §Specialisation Depth, and
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance Semantics: C_OBJECT + §Occurrences inferencing rules.

use crate::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use crate::v2_4::aom2::constraint_model::c_object::CObject;
use crate::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use crate::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_base::v1_3::foundation_types::interval::multiplicity_interval::MultiplicityInterval;

impl CObject {
    /// Returns the `node_id` of this object node.
    #[must_use]
    pub fn node_id(&self) -> &str {
        match self {
            Self::ArchetypeSlot(o) => &o.node_id,
            Self::CBoolean(o) => &o.node_id,
            Self::CComplexObject(o) => o.node_id(),
            Self::CComplexObjectProxy(o) => &o.node_id,
            Self::CDate(o) => &o.node_id,
            Self::CDateTime(o) => &o.node_id,
            Self::CDuration(o) => &o.node_id,
            Self::CInteger(o) => &o.node_id,
            Self::CReal(o) => &o.node_id,
            Self::CString(o) => &o.node_id,
            Self::CTerminologyCode(o) => &o.node_id,
            Self::CTime(o) => &o.node_id,
        }
    }

    /// Returns the `occurrences` of this object node.
    #[must_use]
    pub fn occurrences(&self) -> Option<&MultiplicityInterval> {
        match self {
            Self::ArchetypeSlot(o) => o.occurrences.as_ref(),
            Self::CBoolean(o) => o.occurrences.as_ref(),
            Self::CComplexObject(o) => o.occurrences(),
            Self::CComplexObjectProxy(o) => o.occurrences.as_ref(),
            Self::CDate(o) => o.occurrences.as_ref(),
            Self::CDateTime(o) => o.occurrences.as_ref(),
            Self::CDuration(o) => o.occurrences.as_ref(),
            Self::CInteger(o) => o.occurrences.as_ref(),
            Self::CReal(o) => o.occurrences.as_ref(),
            Self::CString(o) => o.occurrences.as_ref(),
            Self::CTerminologyCode(o) => o.occurrences.as_ref(),
            Self::CTime(o) => o.occurrences.as_ref(),
        }
    }

    /// Returns the specialisation level of this node, from its `node_id`.
    ///
    /// `specialisation_depth` (`org.openehr.am.aom2.c_object.adoc` §Functions):
    /// "The value 0 corresponds to non-specialised, 1 to first-level
    /// specialisation and so on. The level is the same as the number of '.'
    /// characters in the `node_id` code. If `node_id` is not set, the return
    /// value is -1, signifying that the specialisation level should be
    /// determined from the nearest parent `C_OBJECT` node having a `node_id`."
    #[must_use]
    pub fn specialisation_depth(&self) -> i32 {
        specialisation_depth_of(self.node_id())
    }

    /// Returns true if this node is prohibited.
    ///
    /// `is_prohibited` (`org.openehr.am.aom2.c_object.adoc` §Functions),
    /// post-condition `Result = occurrences /= Void and then
    /// occurrences.is_prohibited`, i.e. an `occurrences` of `0..0`.
    #[must_use]
    pub fn is_prohibited(&self) -> bool {
        self.occurrences()
            .is_some_and(MultiplicityInterval::is_prohibited)
    }

    /// Returns the `rm_type_name` of this object node.
    #[must_use]
    pub fn rm_type_name(&self) -> &str {
        match self {
            Self::ArchetypeSlot(o) => &o.rm_type_name,
            Self::CBoolean(o) => &o.rm_type_name,
            Self::CComplexObject(o) => o.rm_type_name(),
            Self::CComplexObjectProxy(o) => &o.rm_type_name,
            Self::CDate(o) => &o.rm_type_name,
            Self::CDateTime(o) => &o.rm_type_name,
            Self::CDuration(o) => &o.rm_type_name,
            Self::CInteger(o) => &o.rm_type_name,
            Self::CReal(o) => &o.rm_type_name,
            Self::CString(o) => &o.rm_type_name,
            Self::CTerminologyCode(o) => &o.rm_type_name,
            Self::CTime(o) => &o.rm_type_name,
        }
    }

    /// Returns the `sibling_order` of this object node.
    #[must_use]
    pub fn sibling_order(&self) -> Option<&SiblingOrder> {
        match self {
            Self::ArchetypeSlot(o) => o.sibling_order.as_ref(),
            Self::CBoolean(o) => o.sibling_order.as_ref(),
            Self::CComplexObject(o) => o.sibling_order(),
            Self::CComplexObjectProxy(o) => o.sibling_order.as_ref(),
            Self::CDate(o) => o.sibling_order.as_ref(),
            Self::CDateTime(o) => o.sibling_order.as_ref(),
            Self::CDuration(o) => o.sibling_order.as_ref(),
            Self::CInteger(o) => o.sibling_order.as_ref(),
            Self::CReal(o) => o.sibling_order.as_ref(),
            Self::CString(o) => o.sibling_order.as_ref(),
            Self::CTerminologyCode(o) => o.sibling_order.as_ref(),
            Self::CTime(o) => o.sibling_order.as_ref(),
        }
    }

    /// The node facts the `master04.5` conformance functions read.
    #[must_use]
    pub(crate) fn facts(&self) -> NodeFacts<'_> {
        NodeFacts {
            node_id: self.node_id(),
            rm_type_name: self.rm_type_name(),
            occurrences: self.occurrences(),
            sibling_order: self.sibling_order(),
        }
    }

    /// Returns true if this node's `node_id` conforms to `other`'s.
    ///
    /// `node_id_conforms_to` (`master04.5` §Conformance Semantics: C_OBJECT):
    /// `Result := codes_conformant (node_id, other.node_id)`.
    #[must_use]
    pub fn node_id_conforms_to(&self, other: &CObject) -> bool {
        node_id_conforms(&self.facts(), &other.facts())
    }

    /// Returns true if this node's occurrences conform to `other`'s.
    ///
    /// `occurrences_conforms_to` (`master04.5` §Conformance Semantics:
    /// C_OBJECT): "only redefinitions of single-occurrence nodes can be dealt
    /// with here; redefinitions of multiply-occurrences nodes must be evaluated
    /// at the owning attribute, according to VSONCO" — so the containment test
    /// applies when this node states occurrences and `other`'s upper is 1.
    #[must_use]
    pub fn occurrences_conforms_to(&self, other: &CObject) -> bool {
        occurrences_conform(&self.facts(), &other.facts())
    }

    /// Returns the effective occurrences of this node, inferred when it states
    /// none of its own.
    ///
    /// `effective_occurrences` (`master04.5` §Occurrences inferencing rules):
    /// a stated `occurrences` wins; else the owning attribute's `cardinality`
    /// upper (open when unbounded); else the reference-model multiplicity of
    /// the owning attribute with the lower forced to 0; else open (`0..*`).
    ///
    /// The Eiffel reaches the owning attribute and its owner through the
    /// `parent` back-references. Those are `0..1` in the model and a parser may
    /// leave them unset, so `owning_attribute` (the Eiffel `parent`) and
    /// `owning_object_rm_type` (the Eiffel `parent.parent.rm_type_name`) are
    /// supplied by the caller, and `rm_prop_mult` is the spec's own
    /// `object_multiplicity` lambda over the reference model.
    ///
    /// NOTE: the Eiffel passes `parent.parent.rm_attribute_path`, but
    /// `rm_attribute_path` is a `C_ATTRIBUTE` function and `parent.parent` is
    /// the owning object, so the owning attribute's path is passed here.
    #[must_use]
    pub fn effective_occurrences(
        &self,
        owning_attribute: Option<&CAttribute>,
        owning_object_rm_type: Option<&str>,
        rm_prop_mult: &dyn Fn(&str, &str) -> MultiplicityInterval,
    ) -> MultiplicityInterval {
        if let Some(occurrences) = self.occurrences() {
            return occurrences.clone();
        }
        let Some(attribute) = owning_attribute else {
            return open();
        };
        if let Some(cardinality) = attribute.cardinality.as_ref() {
            return if cardinality.interval.upper_unbounded {
                open()
            } else {
                bounded(0, cardinality.interval.upper)
            };
        }
        let Some(rm_type) = owning_object_rm_type.filter(|t| !t.is_empty()) else {
            return open();
        };
        let mut inferred = rm_prop_mult(rm_type, &attribute.rm_attribute_path());
        inferred.lower = Some(0);
        inferred.lower_unbounded = false;
        inferred.lower_included = true;
        inferred
    }

    /// Returns true if this node on its own expresses the same or narrower
    /// constraints than `other`.
    ///
    /// `c_conforms_to` (`master04.5` §Conformance Semantics: C_OBJECT):
    /// `node_id_conforms_to (other) and (rm_type_name.is_case_insensitive_equal
    /// (other.rm_type_name) or else rmcc (rm_type_name, other.rm_type_name))
    /// and (is_root or else parent.is_multiple or else parent.is_single and
    /// occurrences_conforms_to (other))`.
    ///
    /// `rmcc` is the spec's reference-model conformance lambda. As with
    /// [`CObject::effective_occurrences`], `owning_attribute` carries the
    /// Eiffel `parent`: `None` means `is_root`.
    #[must_use]
    pub fn c_conforms_to(
        &self,
        other: &CObject,
        rmcc: &dyn Fn(&str, &str) -> bool,
        owning_attribute: Option<&CAttribute>,
    ) -> bool {
        conforms(&self.facts(), &other.facts(), rmcc, owning_attribute)
    }

    /// Returns true if this node expresses no constraints beyond `other`'s,
    /// node-id redefinition aside.
    ///
    /// `c_congruent_to` (`master04.5` §Conformance Semantics: C_OBJECT):
    /// identical `rm_type_name`, `occurrences` unset or identical,
    /// `sibling_order` unset or identical, and `node_reuse_congruent (other)`.
    /// `owning_attribute` carries the Eiffel `parent`, whose
    /// `child_reuse_count` the reuse test reads; `None` means `is_root`.
    ///
    /// NOTE: the BMM declares the parameter as `ARCHETYPE_CONSTRAINT` (the
    /// inherited signature) while `master04.5` states the body over `C_OBJECT`,
    /// and the docs text is the oracle.
    #[must_use]
    pub fn c_congruent_to(&self, other: &CObject, owning_attribute: Option<&CAttribute>) -> bool {
        congruent(&self.facts(), &other.facts(), owning_attribute)
    }

    /// Returns true if this node is the sole re-using node of the corresponding
    /// node in the flat parent.
    ///
    /// `node_reuse_congruent` (`master04.5` §Conformance Semantics: C_OBJECT):
    /// `node_id_conforms_to (other) and (is_root or else attached parent and
    /// then parent.child_reuse_count (other.node_id) = 1)`. It is stated in the
    /// spec text only, so it carries no BMM declaration of its own.
    #[must_use]
    pub fn node_reuse_congruent(
        &self,
        other: &CObject,
        owning_attribute: Option<&CAttribute>,
    ) -> bool {
        node_reuse_congruent(&self.facts(), &other.facts(), owning_attribute)
    }
}

/// The `C_OBJECT` node facts the `master04.5` §Conformance Semantics functions
/// read, so the one implementation serves both the `C_OBJECT` slot and its
/// `C_COMPLEX_OBJECT` refinement.
pub(crate) struct NodeFacts<'a> {
    /// The node's `node_id`.
    pub node_id: &'a str,
    /// The node's `rm_type_name`.
    pub rm_type_name: &'a str,
    /// The node's `occurrences`, if stated.
    pub occurrences: Option<&'a MultiplicityInterval>,
    /// The node's `sibling_order`, if stated.
    pub sibling_order: Option<&'a SiblingOrder>,
}

/// `node_id_conforms_to` (`master04.5` §Conformance Semantics: C_OBJECT).
pub(crate) fn node_id_conforms(child: &NodeFacts<'_>, other: &NodeFacts<'_>) -> bool {
    AdlCodeDefinitionsData::codes_conformant(child.node_id, other.node_id)
}

/// `occurrences_conforms_to` (`master04.5` §Conformance Semantics: C_OBJECT).
///
/// NOTE: the Eiffel reads `other.occurrences.upper` with no Void guard while
/// `occurrences` is `0..1`, so an unset `other.occurrences` is read here as the
/// True branch rather than as a dereference.
pub(crate) fn occurrences_conform(child: &NodeFacts<'_>, other: &NodeFacts<'_>) -> bool {
    let (Some(own), Some(theirs)) = (child.occurrences, other.occurrences) else {
        return true;
    };
    if theirs.upper_unbounded || theirs.upper != Some(1) {
        return true;
    }
    theirs.contains(own)
}

/// `c_conforms_to` (`master04.5` §Conformance Semantics: C_OBJECT).
pub(crate) fn conforms(
    child: &NodeFacts<'_>,
    other: &NodeFacts<'_>,
    rmcc: &dyn Fn(&str, &str) -> bool,
    owning_attribute: Option<&CAttribute>,
) -> bool {
    if !node_id_conforms(child, other) {
        return false;
    }
    if !child.rm_type_name.eq_ignore_ascii_case(other.rm_type_name)
        && !rmcc(child.rm_type_name, other.rm_type_name)
    {
        return false;
    }
    match owning_attribute {
        None => true,
        Some(attribute) if attribute.is_multiple => true,
        Some(_) => occurrences_conform(child, other),
    }
}

/// `c_congruent_to` (`master04.5` §Conformance Semantics: C_OBJECT).
pub(crate) fn congruent(
    child: &NodeFacts<'_>,
    other: &NodeFacts<'_>,
    owning_attribute: Option<&CAttribute>,
) -> bool {
    child.rm_type_name.eq_ignore_ascii_case(other.rm_type_name)
        && child
            .occurrences
            .is_none_or(|own| other.occurrences == Some(own))
        && child
            .sibling_order
            .is_none_or(|own| other.sibling_order == Some(own))
        && node_reuse_congruent(child, other, owning_attribute)
}

/// `node_reuse_congruent` (`master04.5` §Conformance Semantics: C_OBJECT).
pub(crate) fn node_reuse_congruent(
    child: &NodeFacts<'_>,
    other: &NodeFacts<'_>,
    owning_attribute: Option<&CAttribute>,
) -> bool {
    node_id_conforms(child, other)
        && match owning_attribute {
            None => true,
            Some(attribute) => attribute.child_reuse_count(other.node_id) == 1,
        }
}

/// The unbounded multiplicity interval `0..*` (the Eiffel `make_open`).
fn open() -> MultiplicityInterval {
    MultiplicityInterval {
        lower: Some(0),
        upper: None,
        lower_unbounded: false,
        upper_unbounded: true,
        lower_included: true,
        upper_included: false,
    }
}

/// A multiplicity interval with a stated lower bound (the Eiffel
/// `make_bounded`), open above when `upper` is absent.
fn bounded(lower: i32, upper: Option<i32>) -> MultiplicityInterval {
    match upper {
        Some(upper) => MultiplicityInterval {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        },
        None => open(),
    }
}

/// The specialisation depth a `node_id` code encodes, or `-1` when unset.
pub(crate) fn specialisation_depth_of(node_id: &str) -> i32 {
    if node_id.is_empty() {
        return -1;
    }
    i32::try_from(
        node_id
            .matches(AdlCodeDefinitionsData::SPECIALISATION_SEPARATOR)
            .count(),
    )
    .unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::c_complex_object::{
        CComplexObject, CComplexObjectData,
    };

    fn object(node_id: &str, occurrences: Option<MultiplicityInterval>) -> CObject {
        CObject::CComplexObject(CComplexObject::CComplexObject(CComplexObjectData {
            parent: None,
            soc_parent: None,
            rm_type_name: "ELEMENT".to_owned(),
            occurrences,
            node_id: node_id.to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            attributes: None,
            attribute_tuples: None,
        }))
    }

    fn interval(lower: i32, upper: i32) -> MultiplicityInterval {
        MultiplicityInterval {
            lower: Some(lower),
            upper: Some(upper),
            lower_unbounded: false,
            upper_unbounded: false,
            lower_included: true,
            upper_included: true,
        }
    }

    #[test]
    fn the_depth_is_the_separator_count_of_the_node_id() {
        assert_eq!(object("id1", None).specialisation_depth(), 0);
        assert_eq!(object("at0004.1", None).specialisation_depth(), 1);
        assert_eq!(object("at0004.0.1", None).specialisation_depth(), 2);
    }

    #[test]
    fn an_unset_node_id_defers_to_the_nearest_coded_ancestor() {
        assert_eq!(object("", None).specialisation_depth(), -1);
    }

    #[test]
    fn prohibition_is_an_occurrences_of_zero_to_zero() {
        assert!(object("id1", Some(interval(0, 0))).is_prohibited());
        assert!(!object("id1", Some(interval(0, 1))).is_prohibited());
        assert!(!object("id1", None).is_prohibited());
    }
}
