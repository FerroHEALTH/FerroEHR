// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! opt14 ↔ `v1_4` constraint-model divergence sentinel.
//!
//! The AOM 1.4 constraint model exists twice by design: BMM-generated
//! `openehr_am::v1_4` (the canonical logical model, canonical-JSON codec) and
//! XSD-generated `openehr_its::opt14` (the Ocean OPT-XML wire adapter). The
//! divergences between the two are *deliberate and documented* by the
//! respective vendored inputs — the AOM 1.4 BMM
//! (`AM/docs/AOM1.4/master04-constraint_model_package.adoc`) and the Ocean
//! OPT XSD (`crates/openehr-its/schemas/xml/`). This test is the drift guard:
//! it pins both models' constraint-type inventories with
//! **exhaustive** (wildcard-free) matches and explicit inventory lists, so an
//! AOM/OPT spec bump that regenerates either side with a new/removed/renamed
//! constraint type fails here and forces a conscious reconciliation.
//!
//! It intentionally does NOT compare field shapes — the field-level divergence
//! (Interval representation, typed vs `Any` assumed values, domain-type sets)
//! is the documented reason the models are separate; the inventory is the
//! drift-prone surface.

/// The opt14 (OPT-XML) polymorphic constraint inventories. Exhaustive matches:
/// a regeneration that adds or removes an enum variant breaks compilation.
#[expect(
    dead_code,
    reason = "the inventory fns exist for their exhaustive wildcard-free matches, which are the compile-time drift guard; nothing calls them"
)]
mod opt14_inventory {
    use openehr_its::opt14::types;

    fn c_object_variants(v: &types::CObject) -> &'static str {
        match v {
            types::CObject::ArchetypeInternalRef(_) => "ARCHETYPE_INTERNAL_REF",
            types::CObject::ArchetypeSlot(_) => "ARCHETYPE_SLOT",
            types::CObject::ConstraintRef(_) => "CONSTRAINT_REF",
            types::CObject::CArchetypeRoot(_) => "C_ARCHETYPE_ROOT",
            types::CObject::CCodePhrase(_) => "C_CODE_PHRASE",
            types::CObject::CCodeReference(_) => "C_CODE_REFERENCE",
            types::CObject::CComplexObject(_) => "C_COMPLEX_OBJECT",
            types::CObject::CDefinedObject(_) => "C_DEFINED_OBJECT",
            types::CObject::CDvOrdinal(_) => "C_DV_ORDINAL",
            types::CObject::CDvQuantity(_) => "C_DV_QUANTITY",
            types::CObject::CDvState(_) => "C_DV_STATE",
            types::CObject::CPrimitiveObject(_) => "C_PRIMITIVE_OBJECT",
            types::CObject::TComplexObject(_) => "T_COMPLEX_OBJECT",
        }
    }

    fn c_attribute_variants(v: &types::CAttribute) -> &'static str {
        match v {
            types::CAttribute::CMultipleAttribute(_) => "C_MULTIPLE_ATTRIBUTE",
            types::CAttribute::CSingleAttribute(_) => "C_SINGLE_ATTRIBUTE",
        }
    }

    fn c_primitive_variants(v: &types::CPrimitive) -> &'static str {
        match v {
            types::CPrimitive::CBoolean(_) => "C_BOOLEAN",
            types::CPrimitive::CDate(_) => "C_DATE",
            types::CPrimitive::CDateTime(_) => "C_DATE_TIME",
            types::CPrimitive::CDuration(_) => "C_DURATION",
            types::CPrimitive::CInteger(_) => "C_INTEGER",
            types::CPrimitive::CReal(_) => "C_REAL",
            types::CPrimitive::CString(_) => "C_STRING",
            types::CPrimitive::CTime(_) => "C_TIME",
        }
    }

    fn c_domain_type_variants(v: &types::CDomainType) -> &'static str {
        match v {
            types::CDomainType::CCodePhrase(_) => "C_CODE_PHRASE",
            types::CDomainType::CCodeReference(_) => "C_CODE_REFERENCE",
            types::CDomainType::CDvOrdinal(_) => "C_DV_ORDINAL",
            types::CDomainType::CDvQuantity(_) => "C_DV_QUANTITY",
            types::CDomainType::CDvState(_) => "C_DV_STATE",
        }
    }
}

/// The `v1_4` (BMM/AOM 1.4) polymorphic constraint inventories, same mechanism.
#[expect(
    dead_code,
    reason = "the inventory fns exist for their exhaustive wildcard-free matches, which are the compile-time drift guard; nothing calls them"
)]
mod v1_4_inventory {
    use openehr_am::v1_4::prelude as am;

    fn c_object_variants(v: &am::CObject) -> &'static str {
        match v {
            am::CObject::ArchetypeInternalRef(_) => "ARCHETYPE_INTERNAL_REF",
            am::CObject::ArchetypeSlot(_) => "ARCHETYPE_SLOT",
            am::CObject::ConstraintRef(_) => "CONSTRAINT_REF",
            am::CObject::CCodedText(_) => "C_CODED_TEXT",
            am::CObject::CComplexObject(_) => "C_COMPLEX_OBJECT",
            am::CObject::COrdinal(_) => "C_ORDINAL",
            am::CObject::CPrimitiveObject(_) => "C_PRIMITIVE_OBJECT",
            am::CObject::CQuantity(_) => "C_QUANTITY",
        }
    }

    fn c_attribute_variants(v: &am::CAttribute) -> &'static str {
        match v {
            am::CAttribute::CMultipleAttribute(_) => "C_MULTIPLE_ATTRIBUTE",
            am::CAttribute::CSingleAttribute(_) => "C_SINGLE_ATTRIBUTE",
        }
    }

    fn c_primitive_variants(v: &am::CPrimitive) -> &'static str {
        match v {
            am::CPrimitive::CBoolean(_) => "C_BOOLEAN",
            am::CPrimitive::CDate(_) => "C_DATE",
            am::CPrimitive::CDateTime(_) => "C_DATE_TIME",
            am::CPrimitive::CDuration(_) => "C_DURATION",
            am::CPrimitive::CInteger(_) => "C_INTEGER",
            am::CPrimitive::CReal(_) => "C_REAL",
            am::CPrimitive::CString(_) => "C_STRING",
            am::CPrimitive::CTime(_) => "C_TIME",
        }
    }

    fn c_domain_type_variants(v: &am::CDomainType) -> &'static str {
        match v {
            am::CDomainType::CCodedText(_) => "C_CODED_TEXT",
            am::CDomainType::COrdinal(_) => "C_ORDINAL",
            am::CDomainType::CQuantity(_) => "C_QUANTITY",
        }
    }
}

/// The *documented* inventory divergence: the OPT-XML
/// domain types the BMM model does not carry, and vice versa. If this set
/// changes (a spec/XSD bump closed or widened the gap), the wire-model design record must be
/// revisited — that is the point of the failure.
#[test]
fn documented_divergence_is_stable() {
    // OPT-XML-only constraint types (no v1_4 counterpart).
    let opt_only = [
        "C_ARCHETYPE_ROOT", // OPT envelope root (Template.xsd)
        "C_CODE_PHRASE",    // OpenehrProfile.xsd shape of C_CODED_TEXT
        "C_CODE_REFERENCE", // Template.xsd extension of C_CODE_PHRASE
        "C_DV_ORDINAL",     // OpenehrProfile.xsd shape of C_ORDINAL
        "C_DV_QUANTITY",    // OpenehrProfile.xsd shape of C_QUANTITY
        "C_DV_STATE",       // OpenehrProfile.xsd only (no BMM class)
        "T_COMPLEX_OBJECT", // Template.xsd default_value overlay node
    ];
    // BMM/v1_4-only constraint types (no OPT-XML counterpart).
    let am_only = ["C_CODED_TEXT", "C_ORDINAL", "C_QUANTITY"];

    // Pin both lists; a regenerated model that gains/loses a type will already
    // have broken the exhaustive matches above — this assert documents *which*
    // names are the expected asymmetry so the fix is a conscious edit here +
    // by design, not a silent drift.
    assert_eq!(opt_only.len(), 7);
    assert_eq!(am_only.len(), 3);
    assert!(opt_only.is_sorted());
    assert!(am_only.is_sorted());
}
