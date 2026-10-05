// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions/diagnostics/fixtures"
)]
//! Enforcement-reach instrumentation:
//! every terminology slot in the RM binding table is PROVABLY reachable —
//! a violating instance is detected and its valid twin is clean — so a
//! never-exercised enforcement site is a failing test instead of a latent
//! gap (`EVENT_CONTEXT.setting` had zero wire exercise until the TERM
//! program added it, and the escape behind it survived precisely because
//! nothing measured reach).
//!
//! The slot inventory is DERIVED (`openehr_rm::v1_2::model::classes()` ×
//! `validate::terminology::slots_for`), never a second hand list; the total is
//! pinned so an arm for a type outside the static model cannot vanish
//! silently. The invariant-core dimension (the generated cores in
//! `openehr_rm::v1_2::validate::generated`) is deliberately OUT of constructive
//! scope: cores are arbitrary predicates whose violating instances are not
//! mechanically derivable from the model — their reach is carried by the
//! fast-vs-typed corpus equivalence battery and the per-invariant unit tests
//! beside each `*_impl.rs`, and the boundary is recorded here rather than
//! silently skipped.

use openehr_rm::v1_2::validate::terminology::{
    Binding, CodeSet, Group, Slot, slot_is_violated, slots_for,
};
use serde_json::{Value, json};

/// A known-member code for each vocabulary binding (bundle facts, pinned the
/// same way the `openehr-term` bundle tests pin them) and a code that is a
/// member of NO openEHR group or code set.
fn valid_code(binding: Binding) -> (&'static str, &'static str) {
    // (terminology_id, code_string)
    match binding {
        Binding::Group(g) => (
            "openehr",
            match g {
                Group::CompositionCategory => "433",
                Group::Setting => "228",
                Group::NullFlavour => "271",
                Group::InstructionState => "245",
                Group::InstructionTransition => "535",
                Group::ParticipationFunction => "253",
                Group::ParticipationMode => "216",
                Group::EventMathFunction => "146",
                Group::TermMappingPurpose => "669",
                Group::AuditChangeType => "249",
                Group::AttestationReason => "240",
                Group::SubjectRelationship => "0",
                Group::VersionLifecycleState => "532",
            },
        ),
        Binding::CodeSet(cs) => match cs {
            CodeSet::Languages => ("ISO_639-1", "en"),
            CodeSet::Countries => ("ISO_3166-1", "UY"),
            CodeSet::CharacterSets => ("IANA_character-sets", "UTF-8"),
            CodeSet::MediaTypes => ("IANA_media-types", "text/plain"),
            CodeSet::NormalStatuses => ("openehr_normal_statuses", "N"),
            CodeSet::CompressionAlgorithms => ("openehr_compression_algorithms", "gzip"),
            CodeSet::IntegrityCheckAlgorithms => ("openehr_integrity_check_algorithms", "SHA-256"),
        },
    }
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT",
        "value": "x",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
            "code_string": code,
        },
    })
}

/// Every (type, slot) pair of the binding table, derived from the model.
fn all_slots() -> Vec<(&'static str, &'static Slot)> {
    let mut out = Vec::new();
    for class in openehr_rm::v1_2::model::classes() {
        for slot in slots_for(class.name) {
            out.push((class.name, slot));
        }
    }
    out
}

/// **Constructive slot reach:** for EVERY slot, an out-of-vocabulary code is
/// detected — at the slot predicate itself and through the full per-type
/// entry point — and the valid twin is clean at the slot.
#[test]
fn every_terminology_slot_detects_a_violation_and_passes_its_valid_twin() {
    let slots = all_slots();
    // The pinned table size: an arm can only leave this table together with a
    // deliberate register/test change, never silently. (Types outside the
    // static model would silently drop out of `all_slots` — the pin catches
    // that shrinkage too; every match arm of `slots_for` names a static-model
    // class.) 56 since ATTESTATION gained the `change_type` slot it inherits
    // from AUDIT_DETAILS: the table is keyed on the exact wire `_type`, so an
    // ancestor's slot reaches a descendant only by being repeated.
    assert_eq!(
        slots.len(),
        56,
        "the slot table changed size — re-pin deliberately"
    );

    for (ty, slot) in slots {
        // Group bindings are guarded on the openehr terminology; code-set
        // bindings judge every terminology. An out-of-set code under the
        // binding's own terminology id must violate in BOTH families.
        let (valid_terminology, valid) = valid_code(slot.binding);
        let bad = coded(valid_terminology, "no-such-code-999");
        assert!(
            slot_is_violated(slot, &bad),
            "{ty}.{} ({}): the out-of-vocabulary twin was not detected",
            slot.field,
            slot.invariant,
        );
        let good = coded(valid_terminology, valid);
        assert!(
            !slot_is_violated(slot, &good),
            "{ty}.{} ({}): the valid twin was rejected",
            slot.field,
            slot.invariant,
        );

        // Full binding-table reach: the same violation surfaces through
        // validate_rm_terminology for the owning type.
        let node = json!({ "_type": ty, slot.field: coded(valid_terminology, "no-such-code-999") });
        let mut out = Vec::new();
        openehr_rm::v1_2::validate::terminology::validate_rm_terminology(ty, &node, &mut out);
        assert!(
            out.iter().any(|iv| iv.message.contains(slot.invariant)),
            "{ty}.{}: validate_rm_terminology did not surface {} (got {out:?})",
            slot.field,
            slot.invariant,
        );
    }
}
