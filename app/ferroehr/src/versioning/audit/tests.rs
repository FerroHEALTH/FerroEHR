// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Unit tests of the parent module.

use serde_json::json;

use super::*;

/// ITS-REST overview §"openehr-version and openehr-audit-details": a
/// caller-supplied `change_type` "MUST be merged" — legal divergent
/// values are honoured; out-of-group tokens are 422; operation
/// mismatches are 400 (mirroring the CONTRIBUTION path).
#[test]
fn merged_change_type_honours_legal_client_codes() {
    // Absent/empty and restated defaults resolve to the operation's code.
    assert_eq!(
        merged_change_type("", change_type::MODIFICATION).unwrap(),
        change_type::MODIFICATION
    );
    assert_eq!(
        merged_change_type(change_type::MODIFICATION, change_type::MODIFICATION).unwrap(),
        change_type::MODIFICATION
    );
    // 250|amendment| is a legal divergent value on an update.
    assert_eq!(
        merged_change_type("250", change_type::MODIFICATION).unwrap(),
        "250"
    );
    // Rubric tokens resolve to their numeric group code.
    assert_eq!(
        merged_change_type("amendment", change_type::MODIFICATION).unwrap(),
        "250"
    );
    // Creates carry 249 only; deletes carry 523 only.
    assert_eq!(
        merged_change_type(change_type::CREATION, change_type::CREATION).unwrap(),
        change_type::CREATION
    );
    assert_eq!(
        merged_change_type(change_type::DELETED, change_type::DELETED).unwrap(),
        change_type::DELETED
    );
}

#[test]
fn merged_change_type_rejects_mismatch_as_bad_request() {
    for (supplied, operation) in [
        (change_type::CREATION, change_type::MODIFICATION),
        (change_type::DELETED, change_type::MODIFICATION),
        (change_type::ATTESTATION, change_type::MODIFICATION),
        ("250", change_type::CREATION),
        ("250", change_type::DELETED),
        (change_type::CREATION, change_type::DELETED),
    ] {
        let err = merged_change_type(supplied, operation).unwrap_err();
        assert!(
            matches!(err, ServiceError::BadRequest(_)),
            "{supplied} on op {operation}: {err:?}"
        );
    }
}

#[test]
fn merged_change_type_rejects_out_of_group_as_unprocessable() {
    for token in ["999", "banana", "creation-x"] {
        match merged_change_type(token, change_type::MODIFICATION) {
            // The refusal is asserted as DATA: the attribute path and the
            // named RM invariant, not a fragment of the sentence.
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("change_type"), "{token}");
                assert_eq!(
                    v.invariant(),
                    Some("AUDIT_DETAILS.Change_type_valid"),
                    "{token}"
                );
            }
            other => panic!("{token}: expected Unprocessable, got {other:?}"),
        }
    }
}

/// Every named constant is a real group member, and the constants cover
/// the COMPLETE `audit_change_type` group — a code added to the TERM
/// bundle without a constant here fails this test.
#[test]
fn change_type_constants_are_the_complete_group() {
    let all = [
        change_type::CREATION,
        change_type::AMENDMENT,
        change_type::MODIFICATION,
        change_type::SYNTHESIS,
        change_type::UNKNOWN,
        change_type::DELETED,
        change_type::ATTESTATION,
        change_type::RESTORATION,
        change_type::FORMAT_CONVERSION,
    ];
    let t = openehr();
    for code in all {
        assert!(t.is_valid_audit_change_type(code), "code {code}");
        // code_string must be numeric (AUDIT_DETAILS.Change_type_valid).
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
    let mut group: Vec<String> = t
        .concepts_in_group(AUDIT_CHANGE_TYPE)
        .iter()
        .map(|c| c.id.clone())
        .collect();
    group.sort();
    let mut named: Vec<String> = all.iter().map(|c| (*c).to_owned()).collect();
    named.sort();
    assert_eq!(group, named, "constants must mirror the full TERM group");
}

#[test]
fn change_type_code_accepts_code_or_rubric_and_rejects_non_members() {
    assert_eq!(change_type_code("249").as_deref(), Some("249"));
    assert_eq!(change_type_code("creation").as_deref(), Some("249"));
    assert_eq!(change_type_code("Deleted").as_deref(), Some("523"));
    assert_eq!(change_type_code("amendment").as_deref(), Some("250"));
    assert_eq!(change_type_code("synthesis").as_deref(), Some("252"));
    assert_eq!(change_type_code("unknown").as_deref(), Some("253"));
    assert_eq!(change_type_code("666").as_deref(), Some("666"));
    // Out-of-group tokens are rejected, not passed through
    // (AUDIT_DETAILS.Change_type_valid).
    assert_eq!(change_type_code("not-a-change-type"), None);
    assert_eq!(change_type_code("532"), None); // lifecycle code, wrong group
}

#[test]
fn rubric_round_trips() {
    assert_eq!(change_type_rubric(change_type::CREATION), "creation");
    assert_eq!(
        change_type_rubric(change_type::MODIFICATION),
        "modification"
    );
    assert_eq!(change_type_rubric(change_type::DELETED), "deleted");
}

/// A commit audit whose committer is the canonical `PARTY_PROXY` the
/// fragment denotes. The decode is `expect`ed because every caller below
/// passes a structurally well-formed proxy; a committer that is NOT
/// structurally a `PARTY_PROXY` is refused by [`party_proxy`] itself, which
/// [`committer_decode_refuses_structurally_invalid_party`] pins.
fn audit_input(system_id: &str, committer: &Value) -> AuditInput {
    AuditInput {
        system_id: system_id.to_owned(),
        change_type: change_type::CREATION.to_owned(),
        description: None,
        committer: party_proxy(committer).expect("the fixture is a canonical PARTY_PROXY"),
        attestation: None,
    }
}

/// Asserts the refusal is the `422` of `AUDIT_DETAILS.System_id_valid`.
fn assert_system_id_refused<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("AUDIT_DETAILS.system_id"));
            assert_eq!(v.invariant(), Some("AUDIT_DETAILS.System_id_valid"));
        }
        other => panic!("expected Unprocessable(System_id_valid), got {other:?}"),
    }
}

/// The causes of a `422` refusal of the committer, which it names as its path.
fn committer_causes<T: std::fmt::Debug>(
    result: Result<T, ServiceError>,
) -> Vec<openehr_base::validate::InvariantViolation> {
    match result {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("AUDIT_DETAILS.committer"));
            v.causes().to_vec()
        }
        other => panic!("expected Unprocessable on the committer, got {other:?}"),
    }
}

#[test]
fn commit_audit_rejects_empty_system_id() {
    let audit = audit_input(
        "",
        &json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }),
    );
    assert_system_id_refused(validate_commit_audit(&audit));
}

#[test]
fn commit_audit_rejects_committer_without_identity() {
    let audit = audit_input("ferroehr.local", &json!({ "_type": "PARTY_IDENTIFIED" }));
    // The nested dispatcher violations survive as DATA: the causes list is
    // asserted, not a substring of the rendered message.
    let causes = committer_causes(validate_commit_audit(&audit));
    assert!(
        causes.iter().any(|c| c.message.contains("Basic_validity")),
        "causes must carry the PARTY_IDENTIFIED invariant, got {causes:?}"
    );
}

#[test]
fn commit_audit_rejects_empty_committer_name() {
    let audit = audit_input(
        "ferroehr.local",
        &json!({ "_type": "PARTY_IDENTIFIED", "name": "" }),
    );
    match validate_commit_audit(&audit) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => assert!(
            v.causes().iter().any(|c| c.message.contains("Name_valid")),
            "causes must carry Name_valid, got {:?}",
            v.causes()
        ),
        other => panic!("expected Unprocessable(Name_valid), got {other:?}"),
    }
}

/// `PARTY_RELATED.relationship` is a mandatory `DV_CODED_TEXT` (1..1 — RM
/// `UML/classes/org.openehr.rm.common.party_related.adoc` §Attributes), so
/// a committer that omits it, or supplies an uncoded `DV_TEXT`, is not a
/// `PARTY_PROXY` at all: the strict reader refuses it before any invariant
/// runs, naming the attribute. The refusal is asserted as DATA (the
/// committer path plus the decode failure's own JSON path).
#[test]
fn committer_decode_refuses_structurally_invalid_party() {
    for bad in [
        json!({ "_type": "PARTY_RELATED", "name": "Mum" }),
        json!({ "_type": "PARTY_RELATED", "name": "Mum",
                "relationship": { "_type": "DV_TEXT", "value": "mother" } }),
    ] {
        let causes = committer_causes(party_proxy(&bad));
        assert!(
            causes
                .iter()
                .any(|c| c.message.contains("relationship") || c.path.contains("relationship")),
            "{bad}: causes must name the offending attribute, got {causes:?}"
        );
    }
}

#[test]
fn commit_audit_party_related_relationship_is_enforced() {
    // Group member 10 = "mother"; a non-member relationship code is a 422
    // (Relationship_valid, party proxies).
    let related = |relationship: Value| {
        let mut c = json!({ "_type": "PARTY_RELATED", "name": "Mum" });
        c.as_object_mut()
            .unwrap()
            .insert("relationship".into(), relationship);
        audit_input("sys", &c)
    };
    let bad = related(json!({
        "_type": "DV_CODED_TEXT", "value": "colleague",
        "defining_code": { "_type": "CODE_PHRASE", "code_string": "99999",
                           "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } }
    }));
    // The refusal is produced by the generated `PARTY_RELATED` core, whose
    // violation message names the invariant, not the rejected code.
    match validate_commit_audit(&bad) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert!(
                v.causes()
                    .iter()
                    .any(|c| c.message.contains("Relationship_valid")),
                "causes must carry Relationship_valid, got {:?}",
                v.causes()
            );
        }
        other => panic!("non-member relationship code must be 422, got {other:?}"),
    }
    validate_commit_audit(&related(json!({
        "_type": "DV_CODED_TEXT", "value": "mother",
        "defining_code": { "_type": "CODE_PHRASE", "code_string": "10",
                           "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } }
    })))
    .expect("subject_relationship group member accepted");
}

#[test]
fn commit_audit_accepts_valid_committers() {
    validate_commit_audit(&audit_input(
        "sys",
        &json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }),
    ))
    .expect("named committer");
    validate_commit_audit(&audit_input(
        "sys",
        &json!({ "_type": "PARTY_IDENTIFIED", "identifiers": [
            { "_type": "DV_IDENTIFIER", "id": "42", "issuer": "x", "type": "id" }
        ] }),
    ))
    .expect("identifier-only committer");
    validate_commit_audit(&audit_input("sys", &json!({ "_type": "PARTY_SELF" })))
        .expect("PARTY_SELF committer");
}

/// A wire `UPDATE_VERSION.commit_audit` with a `251|modification|` code and
/// the given extra members, read through the generated envelope.
fn wire_audit(extra: &Value) -> UpdateAudit {
    let mut audit = json!({
        "change_type": {
            "value": "modification",
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "251" }
        },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
    });
    for (k, v) in extra.as_object().expect("the extra members are an object") {
        audit
            .as_object_mut()
            .expect("built as an object above")
            .insert(k.clone(), v.clone());
    }
    openehr_its::json::from_canonical_value(&audit).expect("the fixture is an UPDATE_AUDIT")
}

/// ITS-REST `schemas/common/UpdateAudit.yaml` names what a server accepts for
/// a commit audit (`UPDATE_AUDIT`, `AUDIT_DETAILS` or no `_type`), so the direct
/// writes refuse an `UPDATE_ATTESTATION` with `400` as the CONTRIBUTION path
/// does.
#[test]
fn a_direct_write_refuses_an_attestation_commit_audit() {
    let attestation = wire_audit(&json!({
        "_type": "UPDATE_ATTESTATION",
        "reason": { "_type": "DV_TEXT", "value": "signed" },
        "is_pending": false
    }));
    match AuditInput::from_update(&attestation, change_type::MODIFICATION, "update", "sys") {
        Err(ServiceError::BadRequest(e)) => {
            assert!(e.message.contains("UPDATE_ATTESTATION"), "{}", e.message);
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

/// `UpdateAudit.yaml`: a `system_id` "When provided it will be validated", so a
/// stated empty one is refused (`AUDIT_DETAILS.System_id_valid`, `422`) and
/// never replaced by the server's own.
#[test]
fn a_direct_write_refuses_a_stated_empty_system_id() {
    let audit = wire_audit(&json!({ "system_id": "" }));
    assert_system_id_refused(AuditInput::from_update(
        &audit,
        change_type::MODIFICATION,
        "update",
        "sys",
    ));
}

/// An omitted `system_id` is the server's own; a stated one is kept.
#[test]
fn a_direct_write_merges_the_system_id() {
    let omitted = AuditInput::from_update(
        &wire_audit(&json!({})),
        change_type::MODIFICATION,
        "update",
        "server.sys",
    )
    .expect("an omitted system_id is accepted");
    assert_eq!(omitted.system_id, "server.sys");
    let stated = AuditInput::from_update(
        &wire_audit(&json!({ "system_id": "client.sys" })),
        change_type::MODIFICATION,
        "update",
        "server.sys",
    )
    .expect("a stated system_id is accepted");
    assert_eq!(stated.system_id, "client.sys");
    assert!(stated.attestation.is_none());
}

/// `UpdateAudit.yaml` types `description` as `UDvText.yaml`, whose members both
/// require `_type`, so the generated audit refuses an untagged description.
#[test]
fn an_untagged_audit_description_is_refused() {
    let untagged = json!({
        "change_type": {
            "value": "modification",
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "251" }
        },
        "description": { "value": "no tag" },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
    });
    let err = openehr_its::json::from_canonical_value::<UpdateAudit>(&untagged)
        .expect_err("an untagged description is refused");
    assert!(err.to_string().contains("_type"), "{err}");
    let tagged = wire_audit(&json!({ "description": { "_type": "DV_TEXT", "value": "tagged" } }));
    assert!(
        crate::service::version_update::audit_base(&tagged)
            .description
            .is_some()
    );
}
