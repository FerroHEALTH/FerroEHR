// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The draft access log's generated types (#5124): the proposal's example
//! entry reads and writes back through the strict canonical-JSON codec, an
//! undeclared member is refused, and each class invariant of the BMM refuses
//! an entry that breaks it.

use ferroehr::access_log_draft::access_log::access_log_checkpoint::AccessLogCheckpoint;
use ferroehr::access_log_draft::access_log::access_log_record::AccessLogRecord;
use ferroehr::access_log_invariants::{Violation, check_checkpoint, check_record};
use serde_json::{Value, json};

/// A coded value of `terminology` with the rubric `value`.
fn coded(terminology: &str, value: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "terminology_id": { "value": terminology }, "code_string": code } })
}

/// The example entry of the proposal's appendix, its `<...>` placeholders
/// standing for the codes the terminology editors assign and the digests.
fn example() -> Value {
    json!({
        "_type": "ACCESS_LOG_ENTRY",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193f1a2-7c1e-7b4a-9f00-2c6d1e8b5a01" },
        "system_id": "cdr.example.org",
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "action": coded("openehr", "read", "<access action: read>"),
        "operation": coded("openehr_its_rest_1.1.0", "getComposition", "getComposition"),
        "outcome": { "_type": "ACCESS_OUTCOME",
            "result": coded("openehr", "granted", "<access outcome: granted>"),
            "delivered": 1 },
        "accessor": { "_type": "ACCESSOR",
            "kind": coded("openehr", "person", "<accessor kind: person>"),
            "party": { "_type": "PARTY_IDENTIFIED",
                "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5" } } },
            "provider": { "_type": "PARTY_IDENTIFIED", "name": "Example Hospital",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "example national register",
                    "id": "00001234", "type": "provider" }] },
            "credential": { "_type": "ACCESS_CREDENTIAL", "issuer": "https://login.example.org",
                "subject": "a1b2c3", "token_id": "jti-7f3e" } },
        "ehr_id": { "_type": "HIER_OBJECT_ID", "value": "7d44b88c-4199-4bad-97dc-d78268e01398" },
        "items": [{ "_type": "ACCESSED_ITEM",
            "version_id": { "_type": "OBJECT_VERSION_ID",
                "value": "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1" },
            "rm_type": "COMPOSITION",
            "template_id": { "_type": "TEMPLATE_ID", "value": "Example discharge summary" },
            "origins": [{ "_type": "ACCESS_ORIGIN", "system_id": "cdr.example.org" }],
            "was_delivered": true }],
        "purposes": [coded("http://terminology.hl7.org/CodeSystem/v3-ActReason", "treatment", "TREAT")],
        "is_emergency": false,
        "channel": { "_type": "ACCESS_CHANNEL", "request_id": "01JC2Q7N3X" },
        "retention": { "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": "2031-11-07" },
            "grounds": [coded("local", "national retention period", "nl-logging-5y")] },
        "link": { "_type": "ACCESS_LOG_LINK", "sequence": 48213, "algorithm": "SHA-256",
            "previous_digest": "<64 hex>", "content_digest": "<64 hex>", "digest": "<64 hex>" }
    })
}

/// Reads `value` as an access-log record.
fn read(value: Value) -> AccessLogRecord {
    serde_json::from_value(value).expect("the record reads")
}

/// The invariants `value` breaks, as `CLASS.Invariant`.
fn broken(value: Value) -> Vec<String> {
    match check_record(&read(value)) {
        Ok(()) => Vec::new(),
        Err(violations) => violations.0.iter().map(Violation::to_string).collect(),
    }
}

#[test]
fn the_example_entry_reads_writes_back_and_holds_every_invariant() {
    let record = read(example());
    assert!(matches!(record, AccessLogRecord::AccessLogEntry(_)));
    let written = serde_json::to_value(&record).expect("the record writes");
    assert_eq!(written["_type"], "ACCESS_LOG_ENTRY", "{written}");
    assert_eq!(
        read(written),
        record,
        "the written form reads back as the same record"
    );
    assert_eq!(check_record(&record), Ok(()));
}

#[test]
fn a_tombstone_reads_and_writes_back() {
    let tombstone = json!({
        "_type": "ACCESS_LOG_TOMBSTONE",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193f1a2-7c1e-7b4a-9f00-2c6d1e8b5a01" },
        "system_id": "cdr.example.org",
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "link": example()["link"].clone(),
        "time_removed": { "_type": "DV_DATE_TIME", "value": "2031-11-08T00:00:00Z" },
        "grounds": [coded("local", "national retention period", "nl-logging-5y")]
    });
    let record = read(tombstone);
    assert!(matches!(record, AccessLogRecord::AccessLogTombstone(_)));
    let written = serde_json::to_value(&record).expect("the tombstone writes");
    assert_eq!(read(written), record);
    assert_eq!(check_record(&record), Ok(()));
}

#[test]
fn the_reader_is_strict() {
    let mut undeclared = example();
    undeclared["not_in_the_model"] = json!(true);
    assert!(serde_json::from_value::<AccessLogRecord>(undeclared).is_err());
    let mut missing = example();
    missing
        .as_object_mut()
        .expect("an object")
        .remove("retention");
    assert!(serde_json::from_value::<AccessLogRecord>(missing).is_err());
    let mut empty_grounds = example();
    empty_grounds["retention"]["grounds"] = json!([]);
    assert!(
        serde_json::from_value::<AccessLogRecord>(empty_grounds).is_err(),
        "grounds is 1..*"
    );
}

#[test]
fn each_entry_invariant_refuses_an_entry_that_breaks_it() {
    let cases: [(&str, fn(&mut Value)); 16] = [
        ("ACCESS_LOG_RECORD.System_id_valid", |e| {
            e["system_id"] = json!("")
        }),
        ("ACCESS_LOG_LINK.Sequence_valid", |e| {
            e["link"]["sequence"] = json!(0)
        }),
        ("ACCESS_LOG_LINK.Algorithm_valid", |e| {
            e["link"]["algorithm"] = json!("")
        }),
        ("ACCESS_LOG_LINK.Digests_valid", |e| {
            e["link"]["digest"] = json!("")
        }),
        ("ACCESS_LOG_ENTRY.Action_valid", |e| {
            e["action"] = coded("openehr", "browse", "1");
        }),
        ("ACCESS_LOG_ENTRY.Items_need_ehr", |e| {
            e.as_object_mut().expect("an object").remove("ehr_id");
        }),
        ("ACCESS_OUTCOME.Result_valid", |e| {
            e["outcome"]["result"] = coded("local", "granted", "1");
        }),
        ("ACCESS_OUTCOME.Delivered_valid", |e| {
            e["outcome"]["delivered"] = json!(-1)
        }),
        ("ACCESSOR.Kind_valid", |e| {
            e["accessor"]["kind"] = coded("openehr", "robot", "1");
        }),
        ("ACCESS_CREDENTIAL.Issuer_valid", |e| {
            e["accessor"]["credential"]["issuer"] = json!("");
        }),
        ("ACCESS_CREDENTIAL.Subject_valid", |e| {
            e["accessor"]["credential"]["subject"] = json!("");
        }),
        ("ACCESSED_ITEM.Item_named", |e| {
            e["items"][0]
                .as_object_mut()
                .expect("an object")
                .remove("version_id");
        }),
        ("ACCESS_ORIGIN.System_id_valid", |e| {
            e["items"][0]["origins"][0]["system_id"] = json!("");
        }),
        ("ACCESS_QUERY.Query_named", |e| {
            e["query"] = json!({ "_type": "ACCESS_QUERY", "language": "AQL" });
        }),
        ("ACCESS_QUERY.Rows_valid", |e| {
            e["query"] =
                json!({ "_type": "ACCESS_QUERY", "stored_query": "org.example::q", "rows": -1 });
        }),
        ("ACCESS_AUTHORISATION.Kind_valid", |e| {
            e["authorisations"] = json!([{ "_type": "ACCESS_AUTHORISATION",
                "kind": coded("openehr", "hunch", "1") }]);
        }),
    ];
    for (invariant, breaking) in cases {
        let mut entry = example();
        breaking(&mut entry);
        assert_eq!(broken(entry), vec![invariant.to_owned()], "{invariant}");
    }
}

#[test]
fn each_checkpoint_invariant_refuses_a_checkpoint_that_breaks_it() {
    let checkpoint = |sequence: i64, proof: &str| -> AccessLogCheckpoint {
        serde_json::from_value(json!({
            "_type": "ACCESS_LOG_CHECKPOINT", "system_id": "cdr.example.org",
            "sequence": sequence, "digest": "<64 hex>", "algorithm": "SHA-256",
            "time_sealed": { "_type": "DV_DATE_TIME", "value": "2026-11-07T10:00:00Z" },
            "signer": { "_type": "PARTY_IDENTIFIED", "name": "Example Hospital" },
            "proof": proof
        }))
        .expect("the checkpoint reads")
    };
    assert_eq!(check_checkpoint(&checkpoint(48213, "<jws>")), Ok(()));
    let named = |result: Result<(), ferroehr::access_log_invariants::Violations>| {
        result.err().map(|violations| {
            violations
                .0
                .iter()
                .map(Violation::to_string)
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        named(check_checkpoint(&checkpoint(0, "<jws>"))),
        Some(vec!["ACCESS_LOG_CHECKPOINT.Sequence_valid".to_owned()])
    );
    assert_eq!(
        named(check_checkpoint(&checkpoint(1, ""))),
        Some(vec!["ACCESS_LOG_CHECKPOINT.Proof_valid".to_owned()])
    );
}
