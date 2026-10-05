// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM subtype relation for type-conformance checks.
//!
//! Backed by the BMM-generated static RM model ([`openehr_rm::v1_2::model`]) —
//! the spec-pinned type hierarchy, regenerated on any spec bump. [`conforms`] is
//! the single seam the validator walk calls.
//!
//! Conformance is spec-correct where both types are known to the model and stays
//! PERMISSIVE where the CONSTRAINT type is unknown, so a future or foreign
//! constraint type cannot cause a false positive. A known constraint type paired
//! with an unknown instance type IS rejected: a known concrete slot must be
//! filled by a known conforming type.

/// Strip a generic argument (`DV_INTERVAL<DV_QUANTITY>` → `DV_INTERVAL`).
fn base(t: &str) -> &str {
    t.split('<').next().unwrap_or(t).trim()
}

/// Whether an instance's concrete RM type conforms to a `WebTemplate` constraint
/// type (the instance type is the constraint type or a spec descendant of it).
///
/// - Equal (modulo generics) always conforms.
/// - When the constraint type is **known** to the RM model, the model decides:
///   a known instance type conforms iff [`openehr_rm::v1_2::model::is_a`] holds; an
///   unknown/bogus instance type is rejected (a known slot needs a known filler).
/// - When the constraint type is **unknown** to the RM model, stay permissive
///   (never over-reject on a type the spec model does not carry).
#[must_use]
pub(crate) fn conforms(instance_type: &str, wt_type: &str) -> bool {
    let (inst, wt) = (base(instance_type), base(wt_type));
    if inst == wt {
        return true;
    }
    // Only judge when the constraint type is a recognised RM type; otherwise stay
    // permissive to avoid over-rejecting on a type the model does not model.
    if openehr_rm::v1_2::model::class(wt).is_none() {
        return true;
    }
    // The constraint type is known. If the instance type is also known, the RM
    // model's transitive is-a relation decides; a known concrete/abstract slot
    // filled by an unknown/bogus instance type is a violation.
    if openehr_rm::v1_2::model::class(inst).is_some() {
        openehr_rm::v1_2::model::is_a(inst, wt)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::conforms;

    #[test]
    fn equal_and_generic() {
        assert!(conforms("DV_QUANTITY", "DV_QUANTITY"));
        assert!(conforms("DV_INTERVAL", "DV_INTERVAL<DV_QUANTITY>"));
    }

    #[test]
    fn coded_text_conforms_to_text_slot() {
        assert!(conforms("DV_CODED_TEXT", "DV_TEXT"));
        // but a plain text does not satisfy a coded-text slot
        assert!(!conforms("DV_TEXT", "DV_CODED_TEXT"));
    }

    #[test]
    fn event_and_entry_families() {
        assert!(conforms("POINT_EVENT", "EVENT"));
        assert!(conforms("INTERVAL_EVENT", "EVENT"));
        assert!(conforms("OBSERVATION", "CARE_ENTRY"));
        assert!(conforms("ACTION", "CARE_ENTRY"));
        assert!(conforms("EVALUATION", "ENTRY"));
        assert!(conforms("ADMIN_ENTRY", "ENTRY"));
        assert!(conforms("SECTION", "CONTENT_ITEM"));
        assert!(conforms("PARTY_SELF", "PARTY_PROXY"));
        assert!(conforms("PARTY_RELATED", "PARTY_PROXY"));
    }

    #[test]
    fn item_and_structure_families() {
        assert!(conforms("ITEM_TREE", "ITEM_STRUCTURE"));
        assert!(conforms("ITEM_LIST", "DATA_STRUCTURE"));
        assert!(conforms("CLUSTER", "ITEM"));
        assert!(conforms("ELEMENT", "ITEM"));
    }

    #[test]
    fn demographic_party_family() {
        assert!(conforms("PERSON", "PARTY"));
        assert!(conforms("ORGANISATION", "PARTY"));
        assert!(conforms("ROLE", "PARTY"));
    }

    #[test]
    fn quantity_ordered_families() {
        assert!(conforms("DV_QUANTITY", "DV_ORDERED"));
        assert!(conforms("DV_QUANTITY", "DV_QUANTIFIED"));
        assert!(conforms("DV_QUANTITY", "DV_AMOUNT"));
        assert!(conforms("DV_ORDINAL", "DV_ORDERED"));
        assert!(conforms("DV_COUNT", "DV_ORDERED"));
        assert!(conforms("DV_DATE", "DV_TEMPORAL"));
        // any concrete DATA_VALUE conforms to the DATA_VALUE root
        assert!(conforms("DV_CODED_TEXT", "DATA_VALUE"));
        assert!(conforms("DV_QUANTITY", "DATA_VALUE"));
    }

    #[test]
    fn confident_concrete_mismatch_is_rejected() {
        assert!(!conforms("DV_TEXT", "DV_QUANTITY"));
        assert!(!conforms("DV_QUANTITY", "DV_CODED_TEXT"));
    }

    #[test]
    fn paragraph_is_not_a_text() {
        // RM 1.2.0: DV_PARAGRAPH inherits DATA_VALUE, not DV_TEXT (its `items`
        // is a List<DV_TEXT>).
        assert!(!conforms("DV_PARAGRAPH", "DV_TEXT"));
        assert!(conforms("DV_PARAGRAPH", "DATA_VALUE"));
    }

    #[test]
    fn unknown_pairing_is_permissive() {
        // A constraint type this model does not carry never rejects.
        assert!(conforms("SOME_FUTURE_TYPE", "ANOTHER_UNKNOWN"));
        assert!(conforms("DV_QUANTITY", "ANOTHER_UNKNOWN"));
    }

    #[test]
    fn bogus_type_where_concrete_expected_is_rejected() {
        // A known concrete/abstract constraint type rejects an unknown instance.
        assert!(!conforms("PLACEHOLDER", "COMPOSITION"));
        assert!(!conforms("NONSENSE", "DV_QUANTITY"));
        assert!(!conforms("PLACEHOLDER", "DATA_VALUE"));
    }
}
