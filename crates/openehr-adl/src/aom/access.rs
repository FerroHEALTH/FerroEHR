// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Field accessors over the closed AOM2 `C_OBJECT` / `C_COMPLEX_OBJECT`
//! subtype sets.
//!
//! `C_OBJECT` declares `rm_type_name`, `node_id`, `occurrences` and
//! `sibling_order` on the abstract class
//! (`docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Class Definitions), but the generated model is a 13-variant Rust enum, so reading
//! any of them is a 13-arm match. Each such match is written exactly once here
//! and every consumer calls it — a new subtype then breaks one place, not a
//! dozen.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::sibling_order::SiblingOrder;
use openehr_base::prelude::MultiplicityInterval;

/// The AOM meta-type (node class) of a [`CObject`], for the VSONT meta-type
/// conformance rule (`master04.5` §Validity Rules: `C_OBJECT`, VSONT L342).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AomType {
    /// `ARCHETYPE_SLOT`.
    Slot,
    /// `C_COMPLEX_OBJECT`.
    ComplexObject,
    /// `C_ARCHETYPE_ROOT`.
    ArchetypeRoot,
    /// `C_COMPLEX_OBJECT_PROXY`.
    Proxy,
    /// `C_BOOLEAN`.
    Boolean,
    /// `C_INTEGER`.
    Integer,
    /// `C_REAL`.
    Real,
    /// `C_STRING`.
    String,
    /// `C_TERMINOLOGY_CODE`.
    TerminologyCode,
    /// `C_DATE`.
    Date,
    /// `C_TIME`.
    Time,
    /// `C_DATE_TIME`.
    DateTime,
    /// `C_DURATION`.
    Duration,
}

impl AomType {
    /// True if this is a `C_PRIMITIVE_OBJECT` descendant (`master04.5`
    /// §`C_PRIMITIVE_OBJECT`).
    #[must_use]
    pub(crate) fn is_primitive(self) -> bool {
        matches!(
            self,
            Self::Boolean
                | Self::Integer
                | Self::Real
                | Self::String
                | Self::TerminologyCode
                | Self::Date
                | Self::Time
                | Self::DateTime
                | Self::Duration
        )
    }
}

/// The [`AomType`] of any [`CObject`].
#[must_use]
pub(crate) fn aom_type(obj: &CObject) -> AomType {
    match obj {
        CObject::ArchetypeSlot(_) => AomType::Slot,
        CObject::CComplexObject(c) => match c {
            CComplexObject::CComplexObject(_) => AomType::ComplexObject,
            CComplexObject::CArchetypeRoot(_) => AomType::ArchetypeRoot,
        },
        CObject::CComplexObjectProxy(_) => AomType::Proxy,
        CObject::CBoolean(_) => AomType::Boolean,
        CObject::CInteger(_) => AomType::Integer,
        CObject::CReal(_) => AomType::Real,
        CObject::CString(_) => AomType::String,
        CObject::CTerminologyCode(_) => AomType::TerminologyCode,
        CObject::CDate(_) => AomType::Date,
        CObject::CTime(_) => AomType::Time,
        CObject::CDateTime(_) => AomType::DateTime,
        CObject::CDuration(_) => AomType::Duration,
    }
}

/// The `occurrences` interval of any [`CObject`], if it carries one.
#[must_use]
pub fn child_occurrences(obj: &CObject) -> Option<&MultiplicityInterval> {
    match obj {
        CObject::ArchetypeSlot(s) => s.occurrences.as_ref(),
        CObject::CComplexObject(c) => match c {
            CComplexObject::CComplexObject(d) => d.occurrences.as_ref(),
            CComplexObject::CArchetypeRoot(r) => r.occurrences.as_ref(),
        },
        CObject::CComplexObjectProxy(p) => p.occurrences.as_ref(),
        CObject::CBoolean(o) => o.occurrences.as_ref(),
        CObject::CInteger(o) => o.occurrences.as_ref(),
        CObject::CReal(o) => o.occurrences.as_ref(),
        CObject::CString(o) => o.occurrences.as_ref(),
        CObject::CTerminologyCode(o) => o.occurrences.as_ref(),
        CObject::CDate(o) => o.occurrences.as_ref(),
        CObject::CTime(o) => o.occurrences.as_ref(),
        CObject::CDateTime(o) => o.occurrences.as_ref(),
        CObject::CDuration(o) => o.occurrences.as_ref(),
    }
}

/// The `sibling_order` marker of any [`CObject`], if it carries one
/// (`master04.5` §Class Definitions; the `before[…]`/`after[…]` anchor of
/// `ADL2/master09.04` §Ordering of Sibling Nodes).
#[must_use]
pub(crate) fn sibling_order(obj: &CObject) -> Option<&SiblingOrder> {
    match obj {
        CObject::ArchetypeSlot(s) => s.sibling_order.as_ref(),
        CObject::CComplexObject(c) => match c {
            CComplexObject::CComplexObject(d) => d.sibling_order.as_ref(),
            CComplexObject::CArchetypeRoot(r) => r.sibling_order.as_ref(),
        },
        CObject::CComplexObjectProxy(p) => p.sibling_order.as_ref(),
        CObject::CBoolean(o) => o.sibling_order.as_ref(),
        CObject::CInteger(o) => o.sibling_order.as_ref(),
        CObject::CReal(o) => o.sibling_order.as_ref(),
        CObject::CString(o) => o.sibling_order.as_ref(),
        CObject::CTerminologyCode(o) => o.sibling_order.as_ref(),
        CObject::CDate(o) => o.sibling_order.as_ref(),
        CObject::CTime(o) => o.sibling_order.as_ref(),
        CObject::CDateTime(o) => o.sibling_order.as_ref(),
        CObject::CDuration(o) => o.sibling_order.as_ref(),
    }
}

/// Clear the `sibling_order` marker of any [`CObject`].
pub(crate) fn strip_sibling_order(obj: &mut CObject) {
    *common_mut(obj).3 = None;
}

/// Mutable references to the four `C_OBJECT` common fields (`rm_type_name`,
/// `node_id`, `occurrences`, `sibling_order`) across every [`CObject`] variant.
pub(crate) fn common_mut(
    o: &mut CObject,
) -> (
    &mut String,
    &mut String,
    &mut Option<MultiplicityInterval>,
    &mut Option<SiblingOrder>,
) {
    match o {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => (
            &mut d.rm_type_name,
            &mut d.node_id,
            &mut d.occurrences,
            &mut d.sibling_order,
        ),
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(b)) => (
            &mut b.rm_type_name,
            &mut b.node_id,
            &mut b.occurrences,
            &mut b.sibling_order,
        ),
        CObject::ArchetypeSlot(s) => (
            &mut s.rm_type_name,
            &mut s.node_id,
            &mut s.occurrences,
            &mut s.sibling_order,
        ),
        CObject::CComplexObjectProxy(p) => (
            &mut p.rm_type_name,
            &mut p.node_id,
            &mut p.occurrences,
            &mut p.sibling_order,
        ),
        CObject::CBoolean(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CDate(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CDateTime(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CDuration(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CInteger(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CReal(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CString(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CTerminologyCode(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
        CObject::CTime(c) => (
            &mut c.rm_type_name,
            &mut c.node_id,
            &mut c.occurrences,
            &mut c.sibling_order,
        ),
    }
}

/// The constrained attributes of a [`CComplexObject`] (either concrete
/// subtype).
#[must_use]
pub fn complex_attributes(cco: &CComplexObject) -> &[CAttribute] {
    match cco {
        CComplexObject::CComplexObject(d) => d.attributes.as_deref().unwrap_or_default(),
        CComplexObject::CArchetypeRoot(r) => r.attributes.as_deref().unwrap_or_default(),
    }
}

/// The node id of any [`CObject`] (empty string where the subtype carries no
/// meaningful node id).
#[must_use]
pub fn object_node_id(obj: &CObject) -> &str {
    match obj {
        CObject::ArchetypeSlot(s) => &s.node_id,
        CObject::CComplexObject(c) => complex_node_id(c),
        CObject::CComplexObjectProxy(p) => &p.node_id,
        CObject::CBoolean(o) => &o.node_id,
        CObject::CInteger(o) => &o.node_id,
        CObject::CReal(o) => &o.node_id,
        CObject::CString(o) => &o.node_id,
        CObject::CTerminologyCode(o) => &o.node_id,
        CObject::CDate(o) => &o.node_id,
        CObject::CTime(o) => &o.node_id,
        CObject::CDateTime(o) => &o.node_id,
        CObject::CDuration(o) => &o.node_id,
    }
}

/// The reference-model type name of any [`CObject`] (empty string for a
/// primitive object, whose RM type is implicit in its cADL leaf kind).
///
/// Used by the reference-model checks (VCORM/VCORMT), which apply only to the
/// non-primitive object nodes that carry an explicit `rm_type_name`.
#[must_use]
pub fn object_rm_type(obj: &CObject) -> &str {
    match obj {
        CObject::ArchetypeSlot(s) => &s.rm_type_name,
        CObject::CComplexObject(c) => complex_rm_type(c),
        CObject::CComplexObjectProxy(p) => &p.rm_type_name,
        CObject::CBoolean(_)
        | CObject::CInteger(_)
        | CObject::CReal(_)
        | CObject::CString(_)
        | CObject::CTerminologyCode(_)
        | CObject::CDate(_)
        | CObject::CTime(_)
        | CObject::CDateTime(_)
        | CObject::CDuration(_) => "",
    }
}

/// The node id of a [`CComplexObject`] (either concrete subtype).
#[must_use]
pub(crate) fn complex_node_id(cco: &CComplexObject) -> &str {
    match cco {
        CComplexObject::CComplexObject(d) => &d.node_id,
        CComplexObject::CArchetypeRoot(r) => &r.node_id,
    }
}

/// The RM type name of a [`CComplexObject`] (either concrete subtype).
#[must_use]
pub(crate) fn complex_rm_type(cco: &CComplexObject) -> &str {
    match cco {
        CComplexObject::CComplexObject(d) => &d.rm_type_name,
        CComplexObject::CArchetypeRoot(r) => &r.rm_type_name,
    }
}
