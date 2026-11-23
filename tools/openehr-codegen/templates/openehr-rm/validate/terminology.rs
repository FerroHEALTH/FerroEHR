// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The terminology-backed RM class invariants: openEHR group membership
//! (`has_code_for_group_id`, `terminology_id = "openehr"` only) and code-set
//! membership (`code_set (id).has_code`), against [`openehr_term::bundle`]
//! (TERM 3.1.0).
//!
//! [`slots_for`] is the one binding table; [`validate_rm_terminology`] and the
//! `openehr-its` terminology pass both read it. Each entry cites its
//! `RM/docs/UML/classes/org.openehr.rm.*.adoc` invariant.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use openehr_base::validate::InvariantViolation;
use openehr_term::bundle::{OpenehrTerminology, openehr};
use serde_json::Value;

/// An openEHR terminology group a coded slot must draw from.
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
            Group::CompositionCategory => t.is_valid_code("composition_category", code),
            Group::Setting => t.is_valid_code("setting", code),
            Group::NullFlavour => t.is_valid_code("null_flavours", code),
            Group::InstructionState => t.is_valid_code("instruction_states", code),
            Group::InstructionTransition => t.is_valid_code("instruction_transitions", code),
            Group::ParticipationFunction => t.is_valid_code("participation_function", code),
            Group::ParticipationMode => t.is_valid_code("participation_mode", code),
            Group::EventMathFunction => t.is_valid_code("event_math_function", code),
            Group::TermMappingPurpose => t.is_valid_code("term_mapping_purpose", code),
            Group::AuditChangeType => t.is_valid_code("audit_change_type", code),
            Group::AttestationReason => t.is_valid_code("attestation_reason", code),
            Group::SubjectRelationship => t.is_valid_code("subject_relationship", code),
            Group::VersionLifecycleState => t.is_valid_code("version_lifecycle_state", code),
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

/// A code set a coded slot must draw from, checked without a
/// `terminology_id` guard.
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
            CodeSet::Languages => t.is_valid_external_code("languages", code),
            CodeSet::Countries => t.is_valid_external_code("countries", code),
            CodeSet::CharacterSets => t.is_valid_external_code("character_sets", code),
            CodeSet::MediaTypes => t.is_valid_external_code("media_types", code),
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

/// The coded slots of an RM `_type`, each with its invariant and vocabulary.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "a flat `_type` -> `&[Slot]` terminology-binding table; the length is the size of the RM type set, not logic"
)]
pub fn slots_for(rm_type: &str) -> &'static [Slot] {
    // `language` and `encoding`: RM `entry.adoc` and `dv_text.adoc`
    // Language_valid and Encoding_valid.
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
        // participation.adoc, extract_participation.adoc: Function_valid, Mode_valid.
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
        // dv_encapsulated.adoc and dv_multimedia.adoc.
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
        // attestation.adoc Reason_valid, and the inherited Change_type_valid: the
        // table is keyed by exact `_type`.
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

/// The `(code, terminology_id)` of a coded node, or `None` when uncoded.
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

/// Whether `slot` is satisfied by `node`: `Some(false)` only for a present
/// in-scope code outside the vocabulary.
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

/// Runs the terminology-backed invariants of one node, appending one violation
/// per violated slot.
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
    use openehr_term::codes::AuditChangeType;

    /// An out-of-group `change_type` is refused on `ATTESTATION` too.
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
                out.iter().any(
                    |v| v.message == format!("Invariant Change_type_valid failed on type {ty}")
                ),
                "{ty}: an out-of-group change_type must be refused, got {out:?}",
            );
        }

        // The accepting twin: a code the group does carry raises nothing.
        let valid = serde_json::json!({
            "change_type": {
                "value": AuditChangeType::Creation.rubric(),
                "defining_code": {
                    "terminology_id": { "value": "openehr" },
                    "code_string": AuditChangeType::Creation.code()
                }
            }
        });
        for ty in ["AUDIT_DETAILS", "ATTESTATION"] {
            let mut out = Vec::new();
            validate_rm_terminology(ty, &valid, &mut out);
            assert!(
                out.is_empty(),
                "{ty}: a valid change_type raises nothing, got {out:?}"
            );
        }
    }
}
