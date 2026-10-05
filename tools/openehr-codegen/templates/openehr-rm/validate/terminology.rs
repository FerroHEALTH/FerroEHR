// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The **terminology-backed RM class invariants**.
//!
//! These are the RM invariants whose
//! evaluation needs a terminology-group or code-set membership lookup, which
//! the generated invariant cores cannot express mechanically (the BMM
//! assertion dialect calls out to `terminology (…)` / `code_set (…)`).
//!
//! Two families of terminology binding are enforced, both properties of the RM
//! instance value alone (independent of any archetype / template):
//!
//! 1. **openEHR terminology *group* codes** (`has_code_for_group_id`) — the
//!    `defining_code` of a coded slot must be a member of the openEHR group
//!    fixed by the owning RM type. These carry `terminology_id = "openehr"`; a
//!    non-`openehr` terminology binding is out of scope for the openEHR-group
//!    check and is skipped (matching `terminology (Terminology_id_openehr)…`).
//! 2. **openEHR / ISO / IANA code-set codes** (`code_set (id).has_code`) — a
//!    `CODE_PHRASE` (or bare code) whose code value must be a member of the
//!    external or internal code set fixed by the invariant. The RM invariant
//!    here has **no** `terminology_id` guard, so the code value is validated
//!    against the set directly.
//!
//! This module is the single source of the slot → vocabulary binding table
//! ([`slots_for`]) plus the membership decisions ([`Group`]/[`CodeSet`], backed
//! by [`openehr_term::bundle`], TERM 3.1.0). Two presentation adapters consume
//! it: [`validate_rm_terminology`] (the core form, emitting
//! [`InvariantViolation`]s — what the `openehr-its` wire-boundary dispatcher
//! runs as a post-core check) and the `openehr-its` RM-instance terminology
//! pass (its own `ValidationKind::Terminology` message rendering). Neither
//! re-derives the bindings.
//!
//! # Spec
//!
//! Each entry cites the RM class-invariant source
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.*.adoc`) and the
//! openEHR terminology group / code-set id it resolves against (defined in the
//! openEHR Terminology, `docs/specs/openehr/TERM/docs/SupportTerminology/`).
//! The bundle codes themselves come from the vendored TERM assets in
//! [`openehr_term::bundle`].

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use openehr_base::validate::InvariantViolation;
use openehr_term::bundle::{OpenehrTerminology, openehr};
use serde_json::Value;

/// An openEHR terminology *group* a coded slot must draw from.
///
/// The membership check is guarded by `terminology_id == "openehr"` (the
/// `has_code_for_group_id` invariants bind against the openEHR terminology
/// only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// `composition_category` — `COMPOSITION.category`.
    CompositionCategory,
    /// `setting` — `EVENT_CONTEXT.setting`.
    Setting,
    /// `null_flavours` — `ELEMENT.null_flavour`.
    NullFlavour,
    /// `instruction_states` — `ISM_TRANSITION.current_state`.
    InstructionState,
    /// `instruction_transitions` — `ISM_TRANSITION.transition`.
    InstructionTransition,
    /// `participation_function` — `PARTICIPATION`/`EXTRACT_PARTICIPATION.function`.
    ParticipationFunction,
    /// `participation_mode` — `PARTICIPATION`/`EXTRACT_PARTICIPATION.mode`.
    ParticipationMode,
    /// `event_math_function` — `INTERVAL_EVENT.math_function`.
    EventMathFunction,
    /// `term_mapping_purpose` — `TERM_MAPPING.purpose`.
    TermMappingPurpose,
    /// `audit_change_type` — `AUDIT_DETAILS.change_type`.
    AuditChangeType,
    /// `attestation_reason` — `ATTESTATION.reason`.
    AttestationReason,
    /// `subject_relationship` — `PARTY_RELATED.relationship`.
    SubjectRelationship,
    /// `version_lifecycle_state` — `VERSION.lifecycle_state`.
    VersionLifecycleState,
}

impl Group {
    /// Whether `code` is a member of this openEHR terminology group.
    #[must_use]
    pub fn is_valid(self, t: &OpenehrTerminology, code: &str) -> bool {
        match self {
            Group::CompositionCategory => t.is_valid_composition_category(code),
            Group::Setting => t.is_valid_setting(code),
            Group::NullFlavour => t.is_valid_null_flavour(code),
            Group::InstructionState => t.is_valid_instruction_state(code),
            Group::InstructionTransition => t.is_valid_instruction_transition(code),
            Group::ParticipationFunction => t.is_valid_participation_function(code),
            Group::ParticipationMode => t.is_valid_participation_mode(code),
            Group::EventMathFunction => t.is_valid_event_math_function(code),
            Group::TermMappingPurpose => t.is_valid_term_mapping_purpose(code),
            Group::AuditChangeType => t.is_valid_audit_change_type(code),
            Group::AttestationReason => t.is_valid_attestation_reason(code),
            Group::SubjectRelationship => t.is_valid_subject_relationship(code),
            Group::VersionLifecycleState => t.is_valid_version_lifecycle_state(code),
        }
    }

    /// A human-readable label for the group (the `openehr-its` RM-instance
    /// terminology pass's message text).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Group::CompositionCategory => "composition category",
            Group::Setting => "setting",
            Group::NullFlavour => "null flavour",
            Group::InstructionState => "instruction state",
            Group::InstructionTransition => "instruction transition",
            Group::ParticipationFunction => "participation function",
            Group::ParticipationMode => "participation mode",
            Group::EventMathFunction => "event math function",
            Group::TermMappingPurpose => "term mapping purpose",
            Group::AuditChangeType => "audit change type",
            Group::AttestationReason => "attestation reason",
            Group::SubjectRelationship => "subject relationship",
            Group::VersionLifecycleState => "version lifecycle state",
        }
    }
}

/// A code set (openEHR-internal or ISO/IANA-external) a coded slot must draw
/// from. Checked **without** a `terminology_id` guard — the RM invariant is
/// `code_set (id).has_code (code)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSet {
    /// `languages` (ISO 639-1).
    Languages,
    /// `countries` (ISO 3166-1).
    Countries,
    /// `character_sets` (IANA).
    CharacterSets,
    /// `media_types` (IANA).
    MediaTypes,
    /// `normal_statuses` (openEHR-internal).
    NormalStatuses,
    /// `compression_algorithms` (openEHR-internal).
    CompressionAlgorithms,
    /// `integrity_check_algorithms` (openEHR-internal).
    IntegrityCheckAlgorithms,
}

impl CodeSet {
    /// Whether `code` is a member of this code set.
    #[must_use]
    pub fn is_valid(self, t: &OpenehrTerminology, code: &str) -> bool {
        match self {
            CodeSet::Languages => t.is_valid_language(code),
            CodeSet::Countries => t.is_valid_country(code),
            CodeSet::CharacterSets => t.is_valid_character_set(code),
            CodeSet::MediaTypes => t.is_valid_media_type(code),
            CodeSet::NormalStatuses => t.is_valid_normal_status(code),
            // No dedicated convenience method: check the internal code set directly.
            CodeSet::CompressionAlgorithms => t
                .code_set("compression_algorithms")
                .is_some_and(|cs| cs.codes.iter().flatten().any(|c| c.value == code)),
            CodeSet::IntegrityCheckAlgorithms => t
                .code_set("integrity_check_algorithms")
                .is_some_and(|cs| cs.codes.iter().flatten().any(|c| c.value == code)),
        }
    }

    /// A human-readable label for the code set (the `openehr-its` RM-instance
    /// terminology pass's message text).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            CodeSet::Languages => "language (ISO 639-1)",
            CodeSet::Countries => "country (ISO 3166-1)",
            CodeSet::CharacterSets => "character set (IANA)",
            CodeSet::MediaTypes => "media type (IANA)",
            CodeSet::NormalStatuses => "normal status",
            CodeSet::CompressionAlgorithms => "compression algorithm",
            CodeSet::IntegrityCheckAlgorithms => "integrity check algorithm",
        }
    }
}

/// The binding of a coded slot: an openEHR group (guarded by `terminology_id ==
/// "openehr"`) or a code set (unguarded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// An openEHR terminology group (`has_code_for_group_id`).
    Group(Group),
    /// A code set (`code_set (id).has_code`).
    CodeSet(CodeSet),
}

impl Binding {
    /// The vocabulary label for this binding.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Binding::Group(g) => g.label(),
            Binding::CodeSet(cs) => cs.label(),
        }
    }
}

/// One coded slot of an RM type, with the BMM invariant name it realizes and the
/// vocabulary its code must be drawn from.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    /// The RM attribute name carrying the coded value (`category`, `language`, …).
    pub field: &'static str,
    /// The BMM class-invariant name this slot enforces (`Category_validity`, …).
    pub invariant: &'static str,
    /// The vocabulary the code must belong to.
    pub binding: Binding,
}

/// The coded slots fixed by an owning RM `_type`, with the invariant +
/// vocabulary each realizes.
///
/// The single source of the terminology binding table; both
/// [`validate_rm_terminology`] and the `openehr-its` RM-instance terminology
/// pass resolve against it.
///
/// Scoping follows the BMM: `null_flavour` is `ELEMENT`-scoped and `normal_status`
/// is scoped to the concrete `DV_ORDERED` descendants (the invariants are declared
/// on `ELEMENT` / `DV_ORDERED`). `math_function` covers all `EVENT` subtypes; the
/// field is present only on `INTERVAL_EVENT`, so the other arms are inert (the
/// missing field skips).
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "a flat `_type` -> `&[Slot]` terminology-binding table; the length is the size of the RM type set, not logic"
)]
pub fn slots_for(rm_type: &str) -> &'static [Slot] {
    // Reused vocab-per-field slot vectors.
    // ENTRY subtypes + DV_TEXT family share `language` (ISO 639-1) + `encoding`
    // (IANA character sets): RM `entry.adoc` Language_valid/Encoding_valid,
    // `dv_text.adoc` Language_valid/Encoding_valid.
    const LANG_ENCODING: &[Slot] = &[
        Slot {
            field: "language",
            invariant: "Language_valid",
            binding: Binding::CodeSet(CodeSet::Languages),
        },
        Slot {
            field: "encoding",
            invariant: "Encoding_valid",
            binding: Binding::CodeSet(CodeSet::CharacterSets),
        },
    ];
    match rm_type {
        // composition.adoc: Category_validity (composition_category group),
        // Language_valid (languages code set), Territory_valid (countries code set).
        "COMPOSITION" => &[
            Slot {
                field: "category",
                invariant: "Category_validity",
                binding: Binding::Group(Group::CompositionCategory),
            },
            Slot {
                field: "language",
                invariant: "Language_valid",
                binding: Binding::CodeSet(CodeSet::Languages),
            },
            Slot {
                field: "territory",
                invariant: "Territory_valid",
                binding: Binding::CodeSet(CodeSet::Countries),
            },
        ],
        // event_context.adoc: Setting_valid (setting group).
        "EVENT_CONTEXT" => &[Slot {
            field: "setting",
            invariant: "Setting_valid",
            binding: Binding::Group(Group::Setting),
        }],
        // element.adoc: Inv_null_flavour_valid (null_flavours group).
        "ELEMENT" => &[Slot {
            field: "null_flavour",
            invariant: "Inv_null_flavour_valid",
            binding: Binding::Group(Group::NullFlavour),
        }],
        // ism_transition.adoc: Current_state_valid + Transition_valid
        // (instruction_states / instruction_transitions groups).
        "ISM_TRANSITION" => &[
            Slot {
                field: "current_state",
                invariant: "Current_state_valid",
                binding: Binding::Group(Group::InstructionState),
            },
            Slot {
                field: "transition",
                invariant: "Transition_valid",
                binding: Binding::Group(Group::InstructionTransition),
            },
        ],
        // participation.adoc + extract_participation.adoc: Function_valid +
        // Mode_valid (both bind the same participation_function / participation_mode
        // groups). The per-type message takes the concrete `_type`.
        "PARTICIPATION" | "EXTRACT_PARTICIPATION" => &[
            Slot {
                field: "function",
                invariant: "Function_valid",
                binding: Binding::Group(Group::ParticipationFunction),
            },
            Slot {
                field: "mode",
                invariant: "Mode_valid",
                binding: Binding::Group(Group::ParticipationMode),
            },
        ],
        // interval_event.adoc: Math_function_validity (event_math_function group).
        // The field lives on INTERVAL_EVENT; the EVENT/POINT_EVENT arms are inert.
        "EVENT" | "POINT_EVENT" | "INTERVAL_EVENT" => &[Slot {
            field: "math_function",
            invariant: "Math_function_validity",
            binding: Binding::Group(Group::EventMathFunction),
        }],
        // entry.adoc / dv_text.adoc: language + encoding.
        "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY"
        | "GENERIC_ENTRY" | "DV_TEXT" | "DV_CODED_TEXT" => LANG_ENCODING,
        // term_mapping.adoc: Purpose_valid (term_mapping_purpose group).
        "TERM_MAPPING" => &[Slot {
            field: "purpose",
            invariant: "Purpose_valid",
            binding: Binding::Group(Group::TermMappingPurpose),
        }],
        // dv_encapsulated.adoc: Charset_valid / Language_valid (character_sets /
        // languages) + dv_multimedia.adoc: Media_type_valid (media_types),
        // Compression_algorithm_validity (compression_algorithms),
        // Integrity_check_algorithm_validity (integrity_check_algorithms).
        "DV_MULTIMEDIA" => &[
            Slot {
                field: "media_type",
                invariant: "Media_type_valid",
                binding: Binding::CodeSet(CodeSet::MediaTypes),
            },
            Slot {
                field: "charset",
                invariant: "Charset_valid",
                binding: Binding::CodeSet(CodeSet::CharacterSets),
            },
            Slot {
                field: "language",
                invariant: "Language_valid",
                binding: Binding::CodeSet(CodeSet::Languages),
            },
            Slot {
                field: "compression_algorithm",
                invariant: "Compression_algorithm_validity",
                binding: Binding::CodeSet(CodeSet::CompressionAlgorithms),
            },
            Slot {
                field: "integrity_check_algorithm",
                invariant: "Integrity_check_algorithm_validity",
                binding: Binding::CodeSet(CodeSet::IntegrityCheckAlgorithms),
            },
        ],
        // dv_encapsulated.adoc: Charset_valid / Language_valid on DV_PARSABLE.
        "DV_PARSABLE" => &[
            Slot {
                field: "charset",
                invariant: "Charset_valid",
                binding: Binding::CodeSet(CodeSet::CharacterSets),
            },
            Slot {
                field: "language",
                invariant: "Language_valid",
                binding: Binding::CodeSet(CodeSet::Languages),
            },
        ],
        // audit_details.adoc: Change_type_valid (audit_change_type group).
        "AUDIT_DETAILS" => &[Slot {
            field: "change_type",
            invariant: "Change_type_valid",
            binding: Binding::Group(Group::AuditChangeType),
        }],
        // attestation.adoc: Reason_valid (attestation_reason group), PLUS the
        // Change_type_valid it inherits from AUDIT_DETAILS — this table is keyed
        // on the exact wire `_type`, so an ancestor's slot reaches a descendant
        // only by being repeated here.
        "ATTESTATION" => &[
            Slot {
                field: "reason",
                invariant: "Reason_valid",
                binding: Binding::Group(Group::AttestationReason),
            },
            Slot {
                field: "change_type",
                invariant: "Change_type_valid",
                binding: Binding::Group(Group::AuditChangeType),
            },
        ],
        // party_related.adoc: Relationship_valid (subject_relationship group).
        "PARTY_RELATED" => &[Slot {
            field: "relationship",
            invariant: "Relationship_valid",
            binding: Binding::Group(Group::SubjectRelationship),
        }],
        // dv_ordered.adoc: Normal_status_validity (normal_statuses code set),
        // declared on DV_ORDERED — applied to each concrete descendant.
        "DV_QUANTITY" | "DV_COUNT" | "DV_PROPORTION" | "DV_ORDINAL" | "DV_SCALE" | "DV_DATE"
        | "DV_TIME" | "DV_DATE_TIME" | "DV_DURATION" => &[Slot {
            field: "normal_status",
            invariant: "Normal_status_validity",
            binding: Binding::CodeSet(CodeSet::NormalStatuses),
        }],
        // version.adoc: Lifecycle_state_valid (version_lifecycle_state group).
        "ORIGINAL_VERSION" | "IMPORTED_VERSION" => &[Slot {
            field: "lifecycle_state",
            invariant: "Lifecycle_state_valid",
            binding: Binding::Group(Group::VersionLifecycleState),
        }],
        // authored_resource.adoc: Original_language_valid (languages code set).
        "AUTHORED_RESOURCE" => &[Slot {
            field: "original_language",
            invariant: "Original_language_valid",
            binding: Binding::CodeSet(CodeSet::Languages),
        }],
        // resource_description_item.adoc + translation_details.adoc: Language_valid
        // (languages code set) — same `language` slot binding.
        "RESOURCE_DESCRIPTION_ITEM" | "TRANSLATION_DETAILS" => &[Slot {
            field: "language",
            invariant: "Language_valid",
            binding: Binding::CodeSet(CodeSet::Languages),
        }],
        _ => &[],
    }
}

/// The `(code, terminology_id)` of a coded value node — a
/// `DV_CODED_TEXT`/state (via `defining_code`) or a bare `CODE_PHRASE`.
///
/// `None` when the node is not coded (e.g. a plain `DV_TEXT` participation
/// function, or an absent optional slot), which is skipped: the `has_code…`
/// invariants are guarded by a `generating_type = DV_CODED_TEXT` / `/= Void`
/// antecedent.
#[must_use]
pub fn openehr_code(node: &Value) -> Option<(&str, &str)> {
    let code_phrase = node.get("defining_code").unwrap_or(node);
    let code = code_phrase.get("code_string").and_then(Value::as_str)?;
    let terminology = code_phrase
        .get("terminology_id")
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some((code, terminology))
}

/// Whether `slot` is satisfied by `node`'s coded value.
///
/// Returns `Some(false)` only for a present, in-scope code that is NOT a
/// member of the bound vocabulary; `None` when the slot is absent, uncoded,
/// or (for a group binding) carries a non-`openehr` terminology (out of scope
/// for the openEHR-group check).
#[must_use]
pub fn slot_is_violated(slot: &Slot, node: &Value) -> bool {
    let Some((code, terminology)) = openehr_code(node) else {
        return false;
    };
    match slot.binding {
        Binding::Group(group) => {
            // `terminology (Terminology_id_openehr).has_code_for_group_id (…)`:
            // a non-openEHR terminology binding is out of scope for the check.
            if terminology != "openehr" {
                return false;
            }
            !group.is_valid(openehr(), code)
        }
        // `code_set (id).has_code (code)`: unconditional on the code value.
        Binding::CodeSet(cs) => !cs.is_valid(openehr(), code),
    }
}

/// Run the terminology-backed RM class invariants for a single canonical-JSON
/// node, dispatching on its `_type`.
///
/// Appends one [`InvariantViolation`] per violated coded slot, in the uniform
/// `Invariant <Name> failed on type <RM_TYPE>` form and keyed to the offending
/// attribute path. A node whose `_type` binds no coded slot appends nothing.
///
/// This is the core form of the RM terminology/code-set invariants; the
/// `openehr-its` wire-boundary dispatcher runs it as a post-core check, and the
/// `openehr-its` RM-instance terminology pass resolves the same [`slots_for`]
/// table for its own message rendering.
pub fn validate_rm_terminology(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    for slot in slots_for(ty) {
        let Some(node) = value.get(slot.field) else {
            continue;
        };
        if slot_is_violated(slot, node) {
            out.push(InvariantViolation::at(
                slot.field,
                format!("Invariant {} failed on type {ty}", slot.invariant),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This table is keyed on the exact wire `_type`, so an invariant declared
    /// on an ancestor reaches a descendant only by being repeated. `ATTESTATION`
    /// inherits `AUDIT_DETAILS`, so an out-of-group `change_type` must be
    /// refused on both — it was accepted on `ATTESTATION` while the identical
    /// `AUDIT_DETAILS` node was refused.
    #[test]
    fn an_inherited_coded_invariant_reaches_the_descendant() {
        let node = serde_json::json!({
            "change_type": {
                "value": "creation",
                "defining_code": {
                    "terminology_id": { "value": "openehr" },
                    "code_string": "9999"
                }
            }
        });
        for ty in ["AUDIT_DETAILS", "ATTESTATION"] {
            let mut out = Vec::new();
            validate_rm_terminology(ty, &node, &mut out);
            assert!(
                out.iter()
                    .any(|v| v.message == format!("Invariant Change_type_valid failed on type {ty}")),
                "{ty}: an out-of-group change_type must be refused, got {out:?}",
            );
        }

        // The accepting twin: a code the group does carry raises nothing.
        let valid = serde_json::json!({
            "change_type": {
                "value": "creation",
                "defining_code": {
                    "terminology_id": { "value": "openehr" },
                    "code_string": "249"
                }
            }
        });
        for ty in ["AUDIT_DETAILS", "ATTESTATION"] {
            let mut out = Vec::new();
            validate_rm_terminology(ty, &valid, &mut out);
            assert!(out.is_empty(), "{ty}: a valid change_type raises nothing, got {out:?}");
        }
    }
}
