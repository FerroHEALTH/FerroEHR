// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Occurrences / cardinality reconciliation for the 1.4→2 conversion.
//!
//! Two rewrites, applied in this order by [`crate::adl14::convert`]: the 1.4
//! default `occurrences` is MATERIALISED (ADL 2 would infer a different one),
//! then RM-default multiplicity is ELIDED. Order matters — the materialisation
//! reads the 1.4 cardinality, which the elision may drop.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` module is
//! our own design (see the [`crate::adl14`] flag); the two default rules
//! reconciled here are the spec-cited ones named below.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use crate::adl14::walk::cco_data_mut;

/// Materialise the ADL 1.4 default `occurrences` on container children before the
/// definition is emitted as ADL 2.
///
/// The two formalisms give an ABSENT `occurrences` different meanings, so the
/// default cannot be carried across implicitly:
///
/// - ADL 1.4 — `ADL1.4/master05-cadl.adoc` §Occurrences L316: "The default
///   occurrences, if none is mentioned, is `{1..1}`".
/// - ADL 2 — `AOM2/master04.5-constraint_model-class_definitions.adoc`
///   §Occurrences inferencing rules: an absent `occurrences` is inferred from the
///   owning attribute's cardinality upper (lower forced to 0), i.e.
///   `0..cardinality.upper`.
///
/// So an unstated 1.4 occurrences on a container child means "exactly once", and
/// leaving it unstated in the ADL 2 output would silently widen it to "none to
/// many". It is written out explicitly here.
///
/// Restricted to CONTAINER attributes because master05 L308 restricts the rule's
/// significance to them ("It only has significance for objects which are children
/// of a container attribute, since by definition, the occurrences of an object
/// which is the value of a single valued attribute can only be `0..1` or `1..1`,
/// and this is already defined by the attribute `existence`"). A `use_node`
/// internal reference is exempt: master05 L515 gives it the REFERENCED node's
/// occurrences, which is exactly what leaving it unstated means in ADL 2 once the
/// proxy is expanded.
pub(super) fn materialise_adl14_occurrences(def: &mut CComplexObject) {
    let Some(d) = cco_data_mut(def) else { return };
    for attr in d.attributes.iter_mut().flatten() {
        let is_container = attr.cardinality.is_some();
        for child in attr.children.iter_mut().flatten() {
            if is_container
                && complex_occurrences(child).is_none()
                && !matches!(child, CObject::CComplexObjectProxy(_))
            {
                set_occurrences(child, one_to_one());
            }
            if let CObject::CComplexObject(c) = child {
                materialise_adl14_occurrences(c);
            }
        }
    }
}

/// The ADL 1.4 default multiplicity `{1..1}` (`ADL1.4/master05-cadl.adoc`
/// §Occurrences L316).
fn one_to_one() -> openehr_base::prelude::MultiplicityInterval {
    openehr_base::prelude::MultiplicityInterval {
        lower: Some(1),
        upper: Some(1),
        lower_unbounded: false,
        upper_unbounded: false,
        lower_included: true,
        upper_included: true,
    }
}

fn set_occurrences(obj: &mut CObject, occ: openehr_base::prelude::MultiplicityInterval) {
    match obj {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.occurrences = Some(occ),
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r.occurrences = Some(occ),
        CObject::CComplexObjectProxy(p) => p.occurrences = Some(occ),
        CObject::ArchetypeSlot(s) => s.occurrences = Some(occ),
        CObject::CBoolean(o) => o.occurrences = Some(occ),
        CObject::CInteger(o) => o.occurrences = Some(occ),
        CObject::CReal(o) => o.occurrences = Some(occ),
        CObject::CString(o) => o.occurrences = Some(occ),
        CObject::CTerminologyCode(o) => o.occurrences = Some(occ),
        CObject::CDate(o) => o.occurrences = Some(occ),
        CObject::CTime(o) => o.occurrences = Some(occ),
        CObject::CDateTime(o) => o.occurrences = Some(occ),
        CObject::CDuration(o) => o.occurrences = Some(occ),
    }
}

/// Drop the RM-default cardinality / occurrences the ADL 2 output leaves unstated.
pub(super) fn elide_multiplicity(def: &mut CComplexObject) {
    elide_cco(def);
}

fn elide_cco(cco: &mut CComplexObject) {
    let Some(d) = cco_data_mut(cco) else { return };
    for attr in d.attributes.iter_mut().flatten() {
        elide_attr(attr);
        for child in attr.children.iter_mut().flatten() {
            if let CObject::CComplexObject(c) = child {
                elide_cco(c);
            }
        }
    }
}

fn elide_attr(attr: &mut CAttribute) {
    // Drop a container cardinality equal to the RM default `{0..*}` (the
    // fixtures elide `cardinality matches {0..*; unordered}`); keep any narrower
    // bound. Drop `occurrences matches {0..*}` on children likewise.
    if let Some(card) = &attr.cardinality
        && is_zero_unbounded(&card.interval)
    {
        attr.cardinality = None;
        attr.is_multiple = false;
    }
    for child in attr.children.iter_mut().flatten() {
        if let Some(occ) = complex_occurrences(child)
            && is_zero_unbounded(occ)
        {
            clear_occurrences(child);
        }
    }
}

fn is_zero_unbounded(mi: &openehr_base::prelude::MultiplicityInterval) -> bool {
    mi.lower == Some(0) && mi.upper_unbounded
}

/// The `occurrences` of a NON-PRIMITIVE `C_OBJECT`, if it carries one.
///
/// Deliberately narrower than [`crate::aom::access::child_occurrences`]: it
/// answers only for the four subtypes whose `occurrences` its mutable partner
/// [`clear_occurrences`] can clear, so the read/clear pair covers exactly the
/// same arms.
fn complex_occurrences(obj: &CObject) -> Option<&openehr_base::prelude::MultiplicityInterval> {
    match obj {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.occurrences.as_ref(),
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r.occurrences.as_ref(),
        CObject::CComplexObjectProxy(p) => p.occurrences.as_ref(),
        CObject::ArchetypeSlot(s) => s.occurrences.as_ref(),
        _ => None,
    }
}

fn clear_occurrences(obj: &mut CObject) {
    match obj {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.occurrences = None,
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r.occurrences = None,
        CObject::CComplexObjectProxy(p) => p.occurrences = None,
        CObject::ArchetypeSlot(s) => s.occurrences = None,
        _ => {}
    }
}
