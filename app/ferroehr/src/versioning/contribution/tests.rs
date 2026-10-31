// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Unit tests of the parent module.

use openehr_its::rest::generated::ehr::NewContribution;

use super::*;

/// A committer that satisfies `Basic_validity` + `Name_valid`.
fn committer() -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" })
}

/// An `audit_change_type` member in the released `DV_CODED_TEXT` shape.
fn coded(code: &str, value: &str) -> Value {
    json!({ "value": value,
        "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": code } })
}

/// The ruling's own example of a `lifecycle_state`: a `DV_CODED_TEXT`
/// (`schemas/ehr/UpdateVersion.yaml` → `data_types/DvCodedText.yaml`).
fn complete() -> Value {
    coded("532", "complete")
}

/// A one-member `NewContribution` body whose member carries `lifecycle`
/// and `commit_audit`, under the CONTRIBUTION `audit`.
fn body(lifecycle: Value, commit_audit: Value, audit: Value) -> Value {
    let mut member = serde_json::Map::new();
    member.insert("lifecycle_state".to_owned(), lifecycle);
    member.insert("commit_audit".to_owned(), commit_audit);
    member.insert("data".to_owned(), json!({ "_type": "COMPOSITION" }));
    let mut body = serde_json::Map::new();
    body.insert(
        "versions".to_owned(),
        Value::Array(vec![Value::Object(member)]),
    );
    body.insert("audit".to_owned(), audit);
    Value::Object(body)
}

fn update_audit() -> Value {
    json!({ "_type": "UPDATE_AUDIT", "change_type": coded("249", "creation"),
        "committer": committer() })
}

fn decode(body: &Value) -> Result<NewContribution<Value>, ServiceError> {
    decode_new_contribution::<NewContribution<Value>>(body)
}

/// The 400 a shape refusal answers, its message.
fn refused(body: &Value) -> String {
    match decode(body) {
        Err(ServiceError::BadRequest(e)) => e.message,
        other => panic!("{body}: expected the 400 shape refusal, got {other:?}"),
    }
}

/// The first member's commit audit and the CONTRIBUTION audit, each as the
/// plain `UPDATE_AUDIT` [`plain_audit`] admits.
fn audits(decoded: &NewContribution<Value>) -> (&UpdateAuditData, &UpdateAuditData) {
    let member = decoded.versions.first().expect("one member");
    (
        plain_audit(&member.commit_audit, "commit_audit").expect("a plain member audit"),
        plain_audit(&decoded.audit, "CONTRIBUTION.audit").expect("a plain audit"),
    )
}

/// Ruling 1 of #3595: `lifecycle_state` is a `DV_CODED_TEXT`
/// (`schemas/ehr/UpdateVersion.yaml` `$ref: ../data_types/DvCodedText.yaml`,
/// `DvCodedText.yaml` `required: [defining_code]`). The SM `Terminology_code`
/// spelling `{terminology_id, code_string}`, a plain string and a bare
/// `{value}` are none, so each is the 400 of syntactically invalid content.
#[test]
fn lifecycle_state_is_a_dv_coded_text() {
    for (lifecycle, why) in [
        (
            json!({ "terminology_id": { "value": "openehr" }, "code_string": "532" }),
            "the SM Terminology_code spelling",
        ),
        (json!("532"), "a plain code string"),
        (json!("complete"), "a plain rubric string"),
        (json!({ "value": "complete" }), "a DV_TEXT-like bare value"),
    ] {
        let message = refused(&body(lifecycle, update_audit(), update_audit()));
        assert!(message.contains("lifecycle_state"), "{why}: {message}");
    }
    // The accepted spellings: the ruling's example, and the same value with
    // its `_type` stated.
    for lifecycle in [
        complete(),
        json!({ "_type": "DV_CODED_TEXT", "value": "complete",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "532" } }),
    ] {
        let decoded = decode(&body(lifecycle.clone(), update_audit(), update_audit()))
            .unwrap_or_else(|e| panic!("{lifecycle} is a DV_CODED_TEXT: {e:?}"));
        let member = decoded.versions.first().expect("one member");
        assert_eq!(member.lifecycle_state.defining_code.code_string, "532");
    }
}

/// Ruling 1 of #3595: `description` is a `DV_TEXT` or `DV_CODED_TEXT`
/// object (`schemas/common/UpdateAudit.yaml` → `data_types/UDvText.yaml`),
/// so the plain-string spelling is the 400, on a member's `commit_audit`
/// and on the CONTRIBUTION `audit` alike, while both object spellings are
/// accepted and a coded description keeps its `defining_code` (RM common
/// `UML/classes/org.openehr.rm.common.audit_details.adoc` §Attributes).
#[test]
fn description_is_a_dv_text_object() {
    let with_description = |description: Value| {
        let mut audit = update_audit();
        audit
            .as_object_mut()
            .expect("built as an object")
            .insert("description".to_owned(), description);
        audit
    };
    let plain = with_description(json!("free text"));
    assert!(refused(&body(complete(), plain.clone(), update_audit())).contains("description"));
    assert!(refused(&body(complete(), update_audit(), plain)).contains("description"));

    let text = with_description(json!({ "_type": "DV_TEXT", "value": "free text" }));
    let coded_description = with_description(json!({ "_type": "DV_CODED_TEXT",
        "value": "amended after review",
        "defining_code": { "_type": "CODE_PHRASE", "code_string": "at0007",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" } } }));
    let decoded = decode(&body(complete(), coded_description, text))
        .expect("both object spellings are accepted");
    let (member, contribution) = audits(&decoded);
    let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
        .parse()
        .expect("the literal is a valid RFC 3339 instant");
    let rebuilt = AuditInput::from_commit_audit(member, "251".to_owned(), "sys").canonical(&now);
    assert_eq!(
        rebuilt
            .pointer("/description/_type")
            .and_then(Value::as_str),
        Some("DV_CODED_TEXT")
    );
    assert_eq!(
        rebuilt
            .pointer("/description/defining_code/code_string")
            .and_then(Value::as_str),
        Some("at0007")
    );
    assert_eq!(
        rebuilt
            .pointer("/description/defining_code/terminology_id/value")
            .and_then(Value::as_str),
        Some("local")
    );
    let rebuilt =
        AuditInput::from_commit_audit(contribution, "249".to_owned(), "sys").canonical(&now);
    assert_eq!(
        rebuilt
            .pointer("/description/_type")
            .and_then(Value::as_str),
        Some("DV_TEXT")
    );
}

/// Ruling 2 of #3595: `UpdateAudit.yaml` `required: [change_type,
/// committer]`, so a member `commit_audit` or a CONTRIBUTION `audit` without
/// `change_type` is the 400; no change type is inferred from
/// `preceding_version_uid` any more.
#[test]
fn a_missing_change_type_is_refused() {
    let without = json!({ "_type": "UPDATE_AUDIT", "committer": committer() });
    assert!(refused(&body(complete(), without.clone(), update_audit())).contains("change_type"));
    assert!(refused(&body(complete(), update_audit(), without)).contains("change_type"));
}

/// Ruling 4 of #3595: `NewContribution.yaml` `required: [versions, audit]`,
/// and `contribution_create.yaml` declares no 422, so a body without its
/// `audit` (or with a `null` one) is the 400 of syntactically invalid
/// content.
#[test]
fn a_missing_audit_is_refused() {
    let mut no_audit = body(complete(), update_audit(), update_audit());
    let map = no_audit.as_object_mut().expect("built as an object");
    map.remove("audit");
    assert!(refused(&no_audit).contains("audit"));
    let null_audit = body(complete(), update_audit(), Value::Null);
    refused(&null_audit);
}

/// Ruling 3 of #3595: a commit audit is sent as `_type` `UPDATE_AUDIT`,
/// `AUDIT_DETAILS` or no `_type` (`schemas/common/UpdateAudit.yaml`), and
/// each of the three is accepted.
#[test]
fn the_three_audit_spellings_are_accepted() {
    for tag in [Some("UPDATE_AUDIT"), Some("AUDIT_DETAILS"), None] {
        let mut audit = json!({ "change_type": coded("249", "creation"),
            "committer": committer() });
        if let Some(tag) = tag {
            audit
                .as_object_mut()
                .expect("built as an object")
                .insert("_type".to_owned(), json!(tag));
        }
        let decoded = decode(&body(complete(), audit.clone(), audit))
            .unwrap_or_else(|e| panic!("{tag:?} is an accepted audit spelling: {e:?}"));
        let (member, contribution) = audits(&decoded);
        assert_eq!(member.change_type.defining_code.code_string, "249");
        assert_eq!(contribution.change_type.defining_code.code_string, "249");
    }
}

/// Ruling 3 of #3595: `_type` `ATTESTATION` or `UPDATE_ATTESTATION` on a
/// commit audit is refused with the 400, on a member and on the
/// CONTRIBUTION audit; an attestation travels in
/// `versions[].attestations[]` (`schemas/ehr/UpdateVersion.yaml`). Any other
/// class name is refused the same way, never stored as if it were an
/// audit.
#[test]
fn an_attestation_or_unknown_class_as_commit_audit_is_refused() {
    let tagged = |tag: &str| {
        json!({ "_type": tag, "change_type": coded("251", "modification"),
            "committer": committer(),
            "reason": { "_type": "DV_TEXT", "value": "witnessed" },
            "is_pending": false })
    };
    // Outside the generated tag set: the decoder refuses the class.
    for bad in [
        "ATTESTATION",
        "COMPOSITION",
        "REVISION_HISTORY_ITEM",
        "UPDATE_VERSION",
    ] {
        let message = refused(&body(complete(), tagged(bad), update_audit()));
        assert!(message.contains(bad), "{bad}: {message}");
        refused(&body(complete(), update_audit(), tagged(bad)));
    }
    // UPDATE_ATTESTATION decodes as the generated subtype, which
    // `plain_audit` refuses.
    let decoded = decode(&body(
        complete(),
        tagged("UPDATE_ATTESTATION"),
        update_audit(),
    ))
    .expect("the generated UpdateAudit reads its UPDATE_ATTESTATION subtype");
    let member = decoded.versions.first().expect("one member");
    match plain_audit(&member.commit_audit, "commit_audit") {
        Err(ServiceError::BadRequest(e)) => {
            assert!(e.message.contains("versions[].attestations"), "{e:?}");
        }
        other => panic!("expected the 400 refusal, got {other:?}"),
    }
    match plain_audit(&decoded.audit, "CONTRIBUTION.audit") {
        Ok(_) => {}
        Err(e) => panic!("an UPDATE_AUDIT is accepted: {e:?}"),
    }
}

/// A commit audit without `committer` is refused with the 400, never
/// completed with a committer the client did not state: ITS-REST
/// `schemas/common/UpdateAudit.yaml` `required: [change_type, committer]`,
/// SM `UML/classes/update_audit.adoc` (`committer` 1..1). A `null`
/// committer is the same absence, and a committer that does not decode as
/// a canonical `PARTY_PROXY` (`UPartyProxy.yaml`) is refused the same way;
/// one that decodes but breaks its own invariants is the 422 of
/// [`validate_commit_audit`].
#[test]
fn commit_audit_without_a_canonical_committer_is_refused() {
    for bad in [
        None,
        Some(Value::Null),
        Some(json!("Dr Jones")),
        Some(json!({})),
        Some(json!(42)),
        Some(json!({ "_type": "PARTY_RELATED", "name": "Mum" })),
    ] {
        let mut audit = json!({ "change_type": coded("249", "creation") });
        if let Some(bad) = bad.clone() {
            audit
                .as_object_mut()
                .expect("built as an object")
                .insert("committer".to_owned(), bad);
        }
        // The generated decoder names the audit it could not read.
        let message = refused(&body(complete(), audit.clone(), update_audit()));
        assert!(
            message.contains("versions[0].commit_audit"),
            "{bad:?}: {message}"
        );
        refused(&body(complete(), update_audit(), audit));
    }
    // The decodable twin: an identity-less PARTY_IDENTIFIED decodes and is
    // left to the invariant check.
    let identityless = json!({ "change_type": coded("249", "creation"),
        "committer": { "_type": "PARTY_IDENTIFIED" } });
    let decoded = decode(&body(complete(), identityless, update_audit()))
        .expect("a canonical PARTY_PROXY decodes");
    let (member, _) = audits(&decoded);
    match validate_commit_audit(&AuditInput::from_commit_audit(
        member,
        "249".to_owned(),
        "s",
    )) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("AUDIT_DETAILS.committer"));
        }
        other => panic!("expected the 422 invariant refusal, got {other:?}"),
    }
}

/// A commit audit that states its own `committer` keeps it, and one that
/// omits `system_id` takes the CONTRIBUTION audit's (RM common master06
/// §Committal and Audits); one that states it keeps its own.
#[test]
fn commit_audit_keeps_its_committer_and_defaults_its_system_id() {
    let member = json!({ "change_type": coded("251", "modification"),
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "member author" } });
    let decoded = decode(&body(complete(), member, update_audit()))
        .expect("a commit audit stating its committer is accepted");
    let (member, _) = audits(&decoded);
    let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
        .parse()
        .expect("the literal is a valid RFC 3339 instant");
    let rebuilt = AuditInput::from_commit_audit(member, "251".to_owned(), "contribution.system")
        .canonical(&now);
    assert_eq!(
        rebuilt.pointer("/committer/name").and_then(Value::as_str),
        Some("member author")
    );
    assert_eq!(
        rebuilt.get("system_id").and_then(Value::as_str),
        Some("contribution.system")
    );
    let stated = json!({ "change_type": coded("251", "modification"),
        "system_id": "member.system", "committer": committer() });
    let decoded = decode(&body(complete(), stated, update_audit())).expect("accepted");
    let (member, _) = audits(&decoded);
    let audit = AuditInput::from_commit_audit(member, "251".to_owned(), "contribution.system");
    assert_eq!(audit.system_id, "member.system");
}

/// A non-string `system_id` does not match `UpdateAudit.yaml`
/// (`system_id: type: string`), so it is the 400 instead of being replaced
/// by the default system id.
#[test]
fn a_non_string_system_id_is_refused() {
    for bad in [json!(7), json!({ "value": "sys" })] {
        let audit = json!({ "change_type": coded("249", "creation"),
            "system_id": bad, "committer": committer() });
        let message = refused(&body(complete(), audit, update_audit()));
        assert!(
            message.contains("versions[0].commit_audit"),
            "{bad}: {message}"
        );
    }
}

/// An `ATTESTATION` stored with a version's commit audit (an imported
/// version may carry one, RM common master06 §Committal and Audits:
/// "`AUDIT_DETAILS` … or its subtype `ATTESTATION`") round-trips whole from
/// its storage fragment: the coded `reason`, `is_pending`, `proof` and
/// `items` come back on an `ATTESTATION`-typed `AUDIT_DETAILS`.
#[test]
fn a_stored_attestation_commit_audit_round_trips() {
    // 648|witnessed| is an `attestation reason` group member
    // (ATTESTATION.Reason_valid).
    let fragment = json!({
        "reason": { "_type": "DV_CODED_TEXT", "value": "witnessed",
            "defining_code": { "_type": "CODE_PHRASE", "code_string": "648",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } } },
        "is_pending": true,
        "proof": "-----BEGIN PGP SIGNATURE-----",
        "items": [{ "_type": "DV_EHR_URI", "value": "ehr://x/y" }]
    });
    let audit = AuditInput {
        system_id: "sysA.example.org".to_owned(),
        change_type: change_type::MODIFICATION.to_owned(),
        description: None,
        committer: party_proxy(&committer()).expect("a canonical committer"),
        attestation: Some(Box::new(
            AttestationParts::decode(&fragment).expect("a valid stored fragment"),
        )),
    };
    let now: jiff::Timestamp = "2026-08-02T10:11:12Z"
        .parse()
        .expect("the literal is a valid RFC 3339 instant");
    let rebuilt = audit.canonical(&now);
    assert_eq!(
        rebuilt.get("_type").and_then(Value::as_str),
        Some("ATTESTATION")
    );
    assert_eq!(rebuilt.get("is_pending"), Some(&Value::Bool(true)));
    assert_eq!(
        rebuilt.get("proof").and_then(Value::as_str),
        Some("-----BEGIN PGP SIGNATURE-----")
    );
    assert_eq!(
        rebuilt
            .pointer("/reason/defining_code/code_string")
            .and_then(Value::as_str),
        Some("648")
    );
    assert_eq!(
        rebuilt.pointer("/items/0/value").and_then(Value::as_str),
        Some("ehr://x/y")
    );
    assert_eq!(
        rebuilt.get("system_id").and_then(Value::as_str),
        Some("sysA.example.org")
    );
}

/// An `UPDATE_ATTESTATION` in `versions[].attestations[]` missing its
/// required `reason` or `is_pending` (`schemas/common/UpdateAttestation.yaml`
/// `required: [reason, is_pending]`) is the 400 of a schema failure.
#[test]
fn an_attestation_missing_its_required_members_is_refused() {
    for (missing, attestation) in [
        (
            "reason",
            json!({ "_type": "UPDATE_ATTESTATION",
            "change_type": coded("666", "attestation"), "committer": committer(),
            "is_pending": false }),
        ),
        (
            "is_pending",
            json!({ "_type": "UPDATE_ATTESTATION",
            "change_type": coded("666", "attestation"), "committer": committer(),
            "reason": { "_type": "DV_TEXT", "value": "witnessed" } }),
        ),
    ] {
        let mut wire = body(complete(), update_audit(), update_audit());
        wire.pointer_mut("/versions/0")
            .and_then(Value::as_object_mut)
            .expect("one member")
            .insert("attestations".to_owned(), json!([attestation]));
        assert!(refused(&wire).contains(missing), "{missing}");
    }
}

fn classify_err(token: &str, has_preceding: bool, has_data: bool) -> Violation {
    match classify(token, has_preceding, has_data) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => v,
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

fn classify_bad_request(token: &str, has_preceding: bool, has_data: bool) -> String {
    match classify(token, has_preceding, has_data) {
        Err(ServiceError::BadRequest(e)) => e.message,
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn classify_preserves_the_full_change_type_set() {
    for code in ["250", "251", "252", "253", "816", "817"] {
        let (action, kept) = classify(code, true, true).expect(code);
        assert_eq!(action, Action::Modify);
        assert_eq!(kept, code);
    }
    let (action, kept) = classify("amendment", true, true).unwrap();
    assert_eq!((action, kept.as_str()), (Action::Modify, "250"));
    let (action, kept) = classify("creation", false, true).unwrap();
    assert_eq!((action, kept.as_str()), (Action::Create, "249"));
    let (action, kept) = classify("523", true, false).unwrap();
    assert_eq!((action, kept.as_str()), (Action::Delete, "523"));
}

#[test]
fn classify_rejects_spec_invalid_combinations() {
    // A non-creation change type as the FIRST version is THE released
    // `400_CONTRIBUTION` trigger ("the modification type does not match
    // the operation"); creation WITH a preceding is its unassigned mirror,
    // adjudicated 422.
    let creation_with_preceding = classify_err("249", true, true);
    assert_eq!(creation_with_preceding.path(), Some("change_type"));
    assert_eq!(
        creation_with_preceding.invariant(),
        Some("RM change_control §Contributions")
    );
    assert!(classify_bad_request("250", false, true).contains("preceding_version_uid"));
    let deleted_with_data = classify_err("523", true, true);
    assert_eq!(deleted_with_data.path(), Some("data"));
    assert_eq!(
        deleted_with_data.invariant(),
        Some("RM change_control §Contributions")
    );
    assert!(classify_bad_request("523", false, false).contains("preceding_version_uid"));
    let out_of_group = classify_err("999", true, true);
    assert_eq!(out_of_group.path(), Some("change_type"));
    assert_eq!(
        out_of_group.invariant(),
        Some("AUDIT_DETAILS.Change_type_valid")
    );
    assert_eq!(classify_err("251", true, false).path(), Some("data"));
}

#[test]
fn classify_attestation_of_existing_version() {
    let (action, kept) = classify("666", true, false).unwrap();
    assert_eq!((action, kept.as_str()), (Action::Attest, "666"));
    let (action, kept) = classify("attestation", true, false).unwrap();
    assert_eq!((action, kept.as_str()), (Action::Attest, "666"));
    assert!(classify_bad_request("666", false, false).contains("preceding_version_uid"));
    let attest_with_data = classify_err("666", true, true);
    assert_eq!(attest_with_data.path(), Some("data"));
    assert!(
        attest_with_data.detail().contains("adds no content"),
        "{attest_with_data}"
    );
}

/// A delete member states the `523|deleted|` its change type commits, in
/// any accepted spelling; every other state of the group contradicts the
/// deletion procedure and is the 400, and a token outside the group keeps
/// the shared out-of-group 422.
#[test]
fn a_delete_member_states_the_deleted_lifecycle_or_nothing() {
    // The accepting twins: the code, its rubric, and the absent state the
    // required-lifecycle check owns instead.
    for accepted in ["523", "deleted", "Deleted"] {
        assert!(
            reject_contradictory_delete_lifecycle(Some(accepted)).is_ok(),
            "{accepted} is the deleted state"
        );
    }
    assert!(reject_contradictory_delete_lifecycle(None).is_ok());

    // Every other member of the group contradicts the change type.
    for contradicting in [
        state::COMPLETE,
        state::INCOMPLETE,
        state::INACTIVE,
        state::ABANDONED,
        "complete",
    ] {
        match reject_contradictory_delete_lifecycle(Some(contradicting)) {
            Err(ServiceError::BadRequest(e)) => {
                assert!(e.message.contains("Logical Deletion"), "{e:?}");
                assert!(e.message.contains("contradicts change_type"), "{e:?}");
            }
            other => panic!("{contradicting} must be a 400, got {other:?}"),
        }
    }

    // An out-of-group token is the same 422 every other member's state
    // gets (ORIGINAL_VERSION.Lifecycle_state_valid).
    match reject_contradictory_delete_lifecycle(Some("999")) {
        Err(ServiceError::Unprocessable { violation: v, .. }) => {
            assert_eq!(v.path(), Some("lifecycle_state"));
            assert_eq!(
                v.invariant(),
                Some("ORIGINAL_VERSION.Lifecycle_state_valid")
            );
        }
        other => panic!("an out-of-group delete state must be 422, got {other:?}"),
    }
}
