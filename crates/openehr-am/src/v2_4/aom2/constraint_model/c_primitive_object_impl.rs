// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written AOM2 `C_PRIMITIVE_OBJECT` spec functions.
//!
//! Spec sources (vendored):
//! `AM/docs/UML/classes/org.openehr.am.aom2.c_primitive_object.adoc` §Functions,
//! each concrete descendant's own class page, whose `constraint` attribute
//! names the native type it constrains (e.g. `c_duration.adoc`:
//! `List<Interval<Iso8601_duration>>`), and
//! `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Conformance Semantics: C_PRIMITIVE_OBJECT.

use crate::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use crate::v2_4::aom2::constraint_model::c_object_impl::{NodeFacts, conforms};
use crate::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use crate::v2_4::aom2::constraint_model::primitive::c_ordered::COrdered;
use crate::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_base::v1_3::foundation_types::interval::multiplicity_interval::MultiplicityInterval;

impl CPrimitiveObject {
    /// Returns true if this constraint carries an assumed value.
    ///
    /// `has_assumed_value` (`org.openehr.am.aom2.c_primitive_object.adoc`
    /// §Functions): "True if there is an assumed value", read against the
    /// `0..1` `assumed_value` attribute every concrete descendant carries.
    #[must_use]
    pub fn has_assumed_value(&self) -> bool {
        match self {
            Self::CBoolean(c) => c.assumed_value.is_some(),
            Self::CDate(c) => c.assumed_value.is_some(),
            Self::CDateTime(c) => c.assumed_value.is_some(),
            Self::CDuration(c) => c.assumed_value.is_some(),
            Self::CInteger(c) => c.assumed_value.is_some(),
            Self::CReal(c) => c.assumed_value.is_some(),
            Self::CString(c) => c.assumed_value.is_some(),
            Self::CTerminologyCode(c) => c.assumed_value.is_some(),
            Self::CTime(c) => c.assumed_value.is_some(),
        }
    }

    /// Returns the name of the native type this constraint constrains.
    ///
    /// `constrained_typename` (`org.openehr.am.aom2.c_primitive_object.adoc`
    /// §Functions): for most types the constrainer typename without its `C_`
    /// prefix (`C_INTEGER` → `Integer`), while "for the date/time types the
    /// mapping is different" — the different mapping being each temporal
    /// descendant's own declared `constraint` element type (`Iso8601_date`,
    /// `Iso8601_time`, `Iso8601_date_time`, `Iso8601_duration`) and
    /// `C_TERMINOLOGY_CODE`'s `Terminology_code` `assumed_value` type.
    #[must_use]
    pub fn constrained_typename(&self) -> &'static str {
        match self {
            Self::CBoolean(_) => "Boolean",
            Self::CInteger(_) => "Integer",
            Self::CReal(_) => "Real",
            Self::CString(_) => "String",
            Self::CDate(_) => "Iso8601_date",
            Self::CTime(_) => "Iso8601_time",
            Self::CDateTime(_) => "Iso8601_date_time",
            Self::CDuration(_) => "Iso8601_duration",
            Self::CTerminologyCode(_) => "Terminology_code",
        }
    }

    /// The node facts the `master04.5` conformance functions read.
    fn facts(&self) -> NodeFacts<'_> {
        match self {
            Self::CBoolean(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CDate(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CDateTime(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CDuration(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CInteger(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CReal(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CString(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CTerminologyCode(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
            Self::CTime(c) => leaf_facts(
                &c.node_id,
                &c.rm_type_name,
                c.occurrences.as_ref(),
                c.sibling_order.as_ref(),
            ),
        }
    }

    /// Returns true if this node on its own expresses the same or narrower
    /// constraints than `other`.
    ///
    /// `c_conforms_to` (`master04.5` §Conformance Semantics:
    /// C_PRIMITIVE_OBJECT): `precursor (other, rmcc) and c_value_conforms_to
    /// (other)`, the precursor being the `C_OBJECT` body. `owning_attribute`
    /// carries the Eiffel `parent` as it does on
    /// [`crate::v2_4::aom2::constraint_model::c_object::CObject`].
    #[must_use]
    pub fn c_conforms_to(
        &self,
        other: &CPrimitiveObject,
        rmcc: &dyn Fn(&str, &str) -> bool,
        owning_attribute: Option<&CAttribute>,
    ) -> bool {
        conforms(&self.facts(), &other.facts(), rmcc, owning_attribute)
            && self.c_value_conforms_to(other)
    }

    /// Returns true if this node expresses no constraints beyond `other`'s.
    ///
    /// `c_congruent_to` (`master04.5` §Conformance Semantics:
    /// C_PRIMITIVE_OBJECT): `constrained_typename.is_case_insensitive_equal
    /// (other.constrained_typename) and c_value_congruent_to (other)`.
    #[must_use]
    pub fn c_congruent_to(&self, other: &CPrimitiveObject) -> bool {
        self.constrained_typename()
            .eq_ignore_ascii_case(other.constrained_typename())
            && self.c_value_congruent_to(other)
    }

    /// Returns true if this node expresses a value constraint that conforms to
    /// `other`'s.
    ///
    /// `c_value_conforms_to` (`master04.5` §Conformance Semantics:
    /// C_PRIMITIVE_OBJECT) is deferred there and effected on each descendant, so
    /// this dispatches to the descendant body. The Eiffel types the parameter
    /// `like Current`, so a pair of different primitive types never conforms.
    #[must_use]
    pub fn c_value_conforms_to(&self, other: &CPrimitiveObject) -> bool {
        match (self, other) {
            (Self::CBoolean(own), Self::CBoolean(theirs)) => own.c_value_conforms_to(theirs),
            (Self::CString(own), Self::CString(theirs)) => own.c_value_conforms_to(theirs),
            (Self::CTerminologyCode(own), Self::CTerminologyCode(theirs)) => {
                own.c_value_conforms_to(theirs)
            }
            _ => match (self.as_ordered(), other.as_ordered()) {
                (Some(own), Some(theirs)) => own.c_value_conforms_to(&theirs),
                _ => false,
            },
        }
    }

    /// Returns true if this node's value constraint is the same as `other`'s.
    ///
    /// `c_value_congruent_to` (`master04.5` §Conformance Semantics:
    /// C_PRIMITIVE_OBJECT) — deferred there and effected on each descendant, as
    /// with [`CPrimitiveObject::c_value_conforms_to`].
    #[must_use]
    pub fn c_value_congruent_to(&self, other: &CPrimitiveObject) -> bool {
        match (self, other) {
            (Self::CBoolean(own), Self::CBoolean(theirs)) => own.c_value_congruent_to(theirs),
            (Self::CString(own), Self::CString(theirs)) => own.c_value_congruent_to(theirs),
            (Self::CTerminologyCode(own), Self::CTerminologyCode(theirs)) => {
                own.c_value_congruent_to(theirs)
            }
            _ => match (self.as_ordered(), other.as_ordered()) {
                (Some(own), Some(theirs)) => own.c_value_congruent_to(&theirs),
                _ => false,
            },
        }
    }

    /// This node as a `C_ORDERED`, when it is one of that subtree's leaves.
    fn as_ordered(&self) -> Option<COrdered> {
        match self {
            Self::CDate(c) => Some(COrdered::CDate(c.clone())),
            Self::CDateTime(c) => Some(COrdered::CDateTime(c.clone())),
            Self::CDuration(c) => Some(COrdered::CDuration(c.clone())),
            Self::CInteger(c) => Some(COrdered::CInteger(c.clone())),
            Self::CReal(c) => Some(COrdered::CReal(c.clone())),
            Self::CTime(c) => Some(COrdered::CTime(c.clone())),
            Self::CBoolean(_) | Self::CString(_) | Self::CTerminologyCode(_) => None,
        }
    }
}

/// The `NodeFacts` of a primitive leaf.
fn leaf_facts<'a>(
    node_id: &'a str,
    rm_type_name: &'a str,
    occurrences: Option<&'a MultiplicityInterval>,
    sibling_order: Option<&'a SiblingOrder>,
) -> NodeFacts<'a> {
    NodeFacts {
        node_id,
        rm_type_name,
        occurrences,
        sibling_order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2_4::aom2::constraint_model::primitive::c_date::CDate;
    use crate::v2_4::aom2::constraint_model::primitive::c_integer::CInteger;

    fn integer(assumed_value: Option<f64>) -> CPrimitiveObject {
        CPrimitiveObject::CInteger(CInteger {
            parent: None,
            soc_parent: None,
            rm_type_name: "Integer".to_owned(),
            occurrences: None,
            node_id: "at9999".to_owned(),
            alternative_ids: None,
            is_deprecated: None,
            sibling_order: None,
            default_value: None,
            assumed_value,
            is_enumerated_type_constraint: None,
            constraint: None,
        })
    }

    fn date() -> CPrimitiveObject {
        CPrimitiveObject::CDate(CDate {
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
            constraint: None,
            pattern_constraint: None,
        })
    }

    #[test]
    fn an_absent_assumed_value_is_no_assumed_value() {
        assert!(!integer(None).has_assumed_value());
        assert!(integer(Some(0.0)).has_assumed_value());
    }

    #[test]
    fn the_simple_types_drop_the_c_prefix() {
        assert_eq!(integer(None).constrained_typename(), "Integer");
    }

    #[test]
    fn the_temporal_types_name_their_iso8601_constraint_type() {
        assert_eq!(date().constrained_typename(), "Iso8601_date");
    }
}
