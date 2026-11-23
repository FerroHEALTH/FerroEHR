// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Occurrences and cardinality reconciliation for the 1.4→2 conversion: the 1.4
//! default `occurrences` is materialised, then RM-default multiplicity elided,
//! in that order.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion: our own design.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

use crate::adl14::walk::cco_data_mut;

/// Materialise the ADL 1.4 default `occurrences` on container children.
///
/// ADL 1.4 reads an absent occurrences as `{1..1}` (`ADL1.4/master05-cadl.adoc`
/// §Occurrences L316), ADL 2 as `0..cardinality.upper` (`AOM2/master04.5`
/// §Occurrences inferencing rules). Only container children are affected
/// (master05 L308), and a `use_node` takes its target's (master05 L515).
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
pub(super) fn elide_multiplicity(cco: &mut CComplexObject) {
    let Some(d) = cco_data_mut(cco) else { return };
    for attr in d.attributes.iter_mut().flatten() {
        elide_attr(attr);
        for child in attr.children.iter_mut().flatten() {
            if let CObject::CComplexObject(c) = child {
                elide_multiplicity(c);
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

/// The `occurrences` of a non-primitive `C_OBJECT`, for exactly the subtypes
/// [`clear_occurrences`] clears.
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
