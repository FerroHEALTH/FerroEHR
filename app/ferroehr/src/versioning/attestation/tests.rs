// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Unit tests of the parent module.

use serde_json::json;

use super::*;

fn now() -> jiff::Timestamp {
    "2026-07-07T10:11:12Z"
        .parse()
        .expect("the literal is a valid RFC 3339 instant")
}

/// Decode a wire attestation payload through the generated
/// `UpdateAttestation` and complete it, as both arrival routes do,
/// returning the canonical form the store and the wire carry. A payload
/// the generated type refuses is the 400 the CONTRIBUTION door answers
/// ([`crate::versioning::contribution::decode_new_contribution`]).
///
/// The fixture states the change type and committer every
/// `UPDATE_ATTESTATION` requires (`PARTY_IDENTIFIED` with a name satisfies
/// `Basic_validity` + `Name_valid`), so each test exercises only the
/// attribute it names; [`missing_committer_is_refused`] pins the refusal
/// of a partial without one.
fn complete(partial: &Value) -> Result<Value, ServiceError> {
    let mut stated = json!({
        "change_type": { "value": "attestation", "defining_code": {
            "terminology_id": { "value": "openehr" }, "code_string": "666" } },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
    });
    for (k, v) in partial.as_object().expect("the fixture is an object") {
        stated
            .as_object_mut()
            .expect("built as an object above")
            .insert(k.clone(), v.clone());
    }
    let input = AttestationInput::from_update(&decode_wire(&stated)?)?;
    Ok(openehr_its::json::to_canonical_value(
        &complete_attestation(&input, "ferroehr.local", now()),
    ))
}

/// The generated `UpdateAttestation` of a wire payload, a refusal being the
/// 400 of a schema failure.
fn decode_wire(
    wire: &Value,
) -> Result<openehr_its::rest::generated::common::UpdateAttestation, ServiceError> {
    openehr_its::json::from_canonical_value(wire)
        .map_err(|e| ServiceError::bad_request(format!("not an UPDATE_ATTESTATION: {e}"), e))
}

/// The 400 a payload the generated type cannot read answers, its message.
fn shape_refusal(result: Result<Value, ServiceError>) -> String {
    match result {
        Err(ServiceError::BadRequest(e)) => e.message,
        other => panic!("expected the 400 shape refusal, got {other:?}"),
    }
}

/// An `UPDATE_ATTESTATION` without a `committer` is refused as the
/// schema-invalid body it is (400), never completed with a committer the
/// client did not state: ITS-REST `schemas/common/UpdateAttestation.yaml`
/// inherits `required: [change_type, committer]` from `UpdateAudit.yaml`
/// through `allOf`. A `null` committer is the same absence.
#[test]
fn missing_committer_is_refused() {
    let base = json!({
        "change_type": { "value": "attestation", "defining_code": {
            "terminology_id": { "value": "openehr" }, "code_string": "666" } },
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false
    });
    let mut null_committer = base.clone();
    null_committer
        .as_object_mut()
        .expect("built as an object above")
        .insert("committer".to_owned(), Value::Null);
    for partial in [base, null_committer] {
        match decode_wire(&partial) {
            Err(ServiceError::BadRequest(e)) => {
                assert!(e.message.contains("committer"), "{e:?}");
            }
            other => panic!("expected the 400 refusal, got {other:?}"),
        }
    }
}

/// A stated attestation `system_id` is kept, and one left out takes the
/// default the caller passes, the audit of the version it attests
/// (`schemas/common/UpdateAudit.yaml`: "when omitted the server sets its own
/// value"; RM common master06 §Committal and Audits). An empty one is
/// refused (`AUDIT_DETAILS.System_id_valid`).
#[test]
fn a_stated_system_id_is_kept() {
    let stated = complete(&json!({
        "system_id": "attesting.system",
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false
    }))
    .expect("a stated system_id is accepted");
    assert_eq!(
        stated.get("system_id").and_then(Value::as_str),
        Some("attesting.system")
    );
    let defaulted = complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false
    }))
    .expect("an omitted system_id is accepted");
    assert_eq!(
        defaulted.get("system_id").and_then(Value::as_str),
        Some("ferroehr.local")
    );
    match complete(&json!({
        "system_id": "",
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false
    })) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.invariant(), Some("AUDIT_DETAILS.System_id_valid"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

/// The attestation committer is held to its own `PARTY_PROXY` invariants
/// like every commit audit's (`PARTY_IDENTIFIED.Basic_validity`, RM common
/// `UML/classes/org.openehr.rm.common.party_identified.adoc` §Invariants).
#[test]
fn an_invalid_attestation_committer_is_refused() {
    let wire = json!({
        "change_type": { "value": "attestation", "defining_code": {
            "terminology_id": { "value": "openehr" }, "code_string": "666" } },
        "committer": { "_type": "PARTY_IDENTIFIED" },
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false
    });
    let update = decode_wire(&wire).expect("an identity-less PARTY_IDENTIFIED decodes");
    match AttestationInput::from_update(&update) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("AUDIT_DETAILS.committer"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

/// The completed `ATTESTATION` is the canonical serialization of the
/// generated RM type: `_type` first, then the BMM's own attribute order
/// (the inherited `AUDIT_DETAILS` attributes, then `attested_view`,
/// `proof`, `items`, `reason`, `is_pending` — RM common
/// `UML/classes/org.openehr.rm.common.attestation.adoc` §Attributes).
#[test]
fn completed_attestation_is_in_bmm_attribute_order() {
    let att = complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "proof": "signed-by-hand",
        "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }]
    }))
    .expect("a complete, well-typed UPDATE_ATTESTATION partial");
    let keys: Vec<&str> = att
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(
        keys,
        vec![
            "_type",
            "system_id",
            "time_committed",
            "change_type",
            "committer",
            "proof",
            "items",
            "reason",
            "is_pending",
        ]
    );
    assert_eq!(
        att.get("_type").and_then(Value::as_str),
        Some("ATTESTATION")
    );
}

/// A `DV_CODED_TEXT` `description` survives completion whole. The
/// inherited `AUDIT_DETAILS.description` is typed `DV_TEXT` at 0..1 (RM
/// common `UML/classes/org.openehr.rm.common.audit_details.adoc`
/// §Attributes, inherited by `…org.openehr.rm.common.attestation.adoc`)
/// and `DV_CODED_TEXT` is a substitutable subtype — so the `_type`, the
/// display `value` and the whole `defining_code` come back unchanged
/// rather than being flattened to the plain text they share.
#[test]
fn coded_description_round_trips() {
    let description = json!({
        "_type": "DV_CODED_TEXT",
        "value": "amended after multidisciplinary review",
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
            "code_string": "at0004"
        }
    });
    let att = complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "description": description.clone()
    }))
    .expect("a coded description is a valid DV_TEXT");
    assert_eq!(att.get("description"), Some(&description));
}

/// Ruling 1 of #3595: a plain string is not the wire shape of `description`,
/// which is a `DV_TEXT` or `DV_CODED_TEXT` object
/// (`schemas/common/UpdateAudit.yaml` → `data_types/UDvText.yaml`), so a
/// plain-string `description` is the 400.
#[test]
fn string_description_is_refused() {
    let message = shape_refusal(complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "description": "countersigned"
    })));
    assert!(message.contains("description"), "{message}");
}

/// A `description` that is not a canonical `DV_TEXT` is refused instead of
/// silently dropped: the generated type cannot read it, so it is the 400
/// naming the member (ruling 4 of #3595: a schema failure is
/// syntactically invalid content).
#[test]
fn malformed_description_is_refused() {
    let message = shape_refusal(complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "description": { "_type": "DV_CODED_TEXT", "value": "no defining_code" }
    })));
    assert!(message.contains("description"), "{message}");
}

/// `ATTESTATION.attested_view` is a `DV_MULTIMEDIA` (0..1). A value that
/// does not decode as one is refused naming the attribute, rather than
/// stored verbatim; the generated type cannot read it, so it is the 400
/// (ruling 4 of #3595).
#[test]
fn malformed_attested_view_is_refused() {
    // DV_MULTIMEDIA.media_type is 1..1 (attestation.adoc §Attributes →
    // dv_multimedia.adoc), so this object cannot be one.
    for bad in [json!({ "_type": "DV_MULTIMEDIA" }), json!("a screenshot")] {
        let message = shape_refusal(complete(&json!({
            "reason": { "_type": "DV_TEXT", "value": "witness" },
            "is_pending": false,
            "attested_view": bad
        })));
        assert!(message.contains("attested_view"), "{message}");
    }
}

/// `ATTESTATION.proof` is a `String` (0..1): a non-string is the 400.
#[test]
fn non_string_proof_is_refused() {
    let message = shape_refusal(complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "proof": { "value": "not-a-string" }
    })));
    assert!(message.contains("proof"), "{message}");
}

/// `ATTESTATION.items` is a `List<DV_EHR_URI>`: a member that is not one
/// is the 400, naming its index.
#[test]
fn malformed_items_member_is_refused() {
    let message = shape_refusal(complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }, 7]
    })));
    assert!(message.contains("items[1]"), "{message}");
}

/// A coded `reason` whose `defining_code` is outside the openEHR
/// `attestation reason` group is refused (`ATTESTATION.Reason_valid`,
/// `org.openehr.rm.common.attestation.adoc` §Invariants).
#[test]
fn out_of_group_coded_reason_is_refused() {
    let err = complete(&json!({
        "reason": {
            "_type": "DV_CODED_TEXT",
            "value": "signed",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                // 249 is `creation` (audit change type), not an
                // `attestation reason` member ({240 signed, 648 witnessed}).
                "code_string": "249"
            }
        },
        "is_pending": false
    }))
    .expect_err("an out-of-group coded reason must be refused");
    match err {
        ServiceError::Unprocessable { violation: v, .. } => {
            assert_eq!(v.path(), Some("ATTESTATION.reason.defining_code"));
            assert_eq!(v.invariant(), Some("ATTESTATION.Reason_valid"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

/// A JSON `null` `items` is the ABSENT encoding of the 0..1 attribute, so
/// it is accepted and yields no list — `ATTESTATION.Items_valid` (`items
/// /= Void implies not items.is_empty`) constrains a PRESENT list, and
/// Void is exactly what a null spells. This is the shape the native
/// `UPDATE_VERSION.attestations` DTO emits for `Option::None`, so a
/// rejection here would make the at-committal attestation route
/// unreachable through the typed API.
#[test]
fn null_items_is_absent_not_invalid() {
    let att = complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "items": Value::Null,
        "proof": Value::Null,
        "attested_view": Value::Null,
        "description": Value::Null
    }))
    .expect("a null optional is absent, not invalid");
    assert_eq!(att.get("items"), None, "no items list is serialized");
}

/// The native `UPDATE_VERSION.attestations` partial, as the direct-write
/// route supplies it.
///
/// The inherited `change_type` is a `DV_CODED_TEXT`: ITS-REST
/// `schemas/common/UpdateAudit.yaml` `$ref`s `DvCodedText` for it, and the
/// ITS-REST docs text is silent on the member's shape, so the released OAS
/// grounds it (`.claude/rules/spec-adherence.md` §the ITS-REST
/// wire-oracle order). The `CODE_PHRASE` shape `{terminology_id, code_string}`
/// this fixture used to send is not that shape — see
/// [`flat_terminology_code_change_type_is_refused`] for the twin that
/// pins its refusal.
fn update_attestation(body: &Value) -> openehr_its::rest::generated::common::UpdateAttestation {
    let mut wire = json!({
        "change_type": {
            "value": "attestation",
            "defining_code": {
                "terminology_id": { "value": "openehr" },
                "code_string": "666"
            }
        },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
    });
    for (k, v) in body.as_object().expect("the fixture is an object") {
        wire.as_object_mut()
            .expect("built as an object above")
            .insert(k.clone(), v.clone());
    }
    openehr_its::json::from_canonical_value(&wire)
        .expect("the fixture is a well-typed UPDATE_ATTESTATION")
}

/// The invalid twin of [`update_attestation`]'s `change_type`: the `CODE_PHRASE`
/// shape `{terminology_id, code_string}` is NOT
/// the released wire shape of `UPDATE_AUDIT.change_type`
/// (`schemas/common/UpdateAudit.yaml` → `DvCodedText`), so the strict
/// canonical reader refuses it — the PARSE class, answered `400` on the
/// wire. A reader that started accepting it would silently admit a member
/// with no `defining_code`, which is the attribute
/// `AUDIT_DETAILS.Change_type_valid` is stated over.
#[test]
fn flat_terminology_code_change_type_is_refused() {
    let refused = openehr_its::json::from_canonical_value::<
        openehr_its::rest::generated::common::UpdateAttestation,
    >(&json!({
        "change_type": { "terminology_id": "openehr", "code_string": "666" },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" },
        "reason": { "_type": "DV_TEXT", "value": "signed" },
        "is_pending": false
    }));
    let err = refused.expect_err("a flat TERMINOLOGY_CODE change_type must be refused");
    assert!(
        err.to_string().contains("terminology_id"),
        "the refusal names the offending member: {err}"
    );
}

/// The native route reaches the SAME invariants as the body route: a coded
/// `reason` outside the openEHR `attestation reason` group is refused
/// (`ATTESTATION.Reason_valid`).
#[test]
fn native_partial_refuses_out_of_group_coded_reason() {
    let update = update_attestation(&json!({
        "reason": {
            "_type": "DV_CODED_TEXT", "value": "signed",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "249"
            }
        },
        "is_pending": false
    }));
    match AttestationInput::from_update(&update) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("ATTESTATION.reason.defining_code"));
            assert_eq!(v.invariant(), Some("ATTESTATION.Reason_valid"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

/// `ATTESTATION.Items_valid` on the native route: a present-but-empty list
/// is refused, an absent one is not.
#[test]
fn native_partial_enforces_items_valid() {
    let base = json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": true
    });
    let mut empty = base.clone();
    empty
        .as_object_mut()
        .expect("built as an object above")
        .insert("items".to_owned(), json!([]));
    match AttestationInput::from_update(&update_attestation(&empty)) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("ATTESTATION.items"));
            assert_eq!(v.invariant(), Some("ATTESTATION.Items_valid"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
    let absent = AttestationInput::from_update(&update_attestation(&base))
        .expect("an absent items list is legal");
    assert_eq!(absent.parts.items, Vec::<DvEhrUri>::new());
}

/// A native partial completes into the same `ATTESTATION` shape the body
/// route produces: the committer is the one the partial stated, and the
/// description object is kept whole.
#[test]
fn native_partial_completes_into_an_attestation() {
    // `description` is an OBJECT on the typed wire: ITS-REST
    // `schemas/common/UpdateAudit.yaml` `$ref`s `DvText` (whose
    // `DV_CODED_TEXT` subtype substitutes for it). A plain string is
    // refused — [`string_description_is_refused`] pins that twin.
    let update = update_attestation(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "description": { "_type": "DV_TEXT", "value": "countersigned" }
    }));
    let input = AttestationInput::from_update(&update).expect("a well-typed partial is accepted");
    let att = openehr_its::json::to_canonical_value(&complete_attestation(
        &input,
        "ferroehr.local",
        now(),
    ));
    assert_eq!(
        att.get("description"),
        Some(&json!({ "_type": "DV_TEXT", "value": "countersigned" }))
    );
    // The committer the partial stated.
    assert_eq!(
        att.pointer("/committer/_type").and_then(Value::as_str),
        Some("PARTY_IDENTIFIED")
    );
    assert_eq!(att.get("items"), None);
}

/// A present-but-empty `items` list is refused (`ATTESTATION.Items_valid`:
/// `items /= Void implies not items.is_empty`,
/// `org.openehr.rm.common.attestation.adoc` §Invariants).
#[test]
fn present_but_empty_items_is_refused() {
    let err = complete(&json!({
        "reason": { "_type": "DV_TEXT", "value": "witness" },
        "is_pending": false,
        "items": []
    }))
    .expect_err("a present-but-empty items list must be refused");
    match err {
        ServiceError::Unprocessable { violation: v, .. } => {
            assert_eq!(v.path(), Some("ATTESTATION.items"));
            assert_eq!(v.invariant(), Some("ATTESTATION.Items_valid"));
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}
