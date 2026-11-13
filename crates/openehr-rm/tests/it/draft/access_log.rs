// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    reason = "test fixtures: a fixture that does not read or write should fail loudly"
)]
//! The draft access log's generated types (#5124): the proposal's example
//! entry reads and writes back through the strict canonical-JSON codec, an
//! undeclared member is refused, and each class invariant of the BMM refuses
//! an entry that breaks it.

use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::invariants::{check_checkpoint, check_record};
use openehr_rm::draft::invariants::{Violation, Violations};
use serde_json::{Value, json};

/// A coded value of `terminology` with the rubric `value`.
fn coded(terminology: &str, value: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "terminology_id": { "value": terminology }, "code_string": code } })
}

/// The example entry of the proposal's appendix, its `<...>` placeholders
/// standing for the codes the terminology editors assign and the digests. The
/// fixture is shared with the access-log store tests of `ferroehr`.
fn example() -> Value {
    serde_json::from_str(include_str!("../../fixtures/draft/access_log_entry.json"))
        .expect("the example entry fixture is JSON")
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

/// An invariant, and the edit of the example entry that breaks it alone.
type Breaking = (&'static str, fn(&mut Value));

#[test]
fn each_entry_invariant_refuses_an_entry_that_breaks_it() {
    let cases: [Breaking; 16] = [
        ("ACCESS_LOG_RECORD.System_id_valid", |e| {
            e["system_id"] = json!("");
        }),
        ("ACCESS_LOG_LINK.Sequence_valid", |e| {
            e["link"]["sequence"] = json!(0);
        }),
        ("ACCESS_LOG_LINK.Algorithm_valid", |e| {
            e["link"]["algorithm"] = json!("");
        }),
        ("ACCESS_LOG_LINK.Digests_valid", |e| {
            e["link"]["digest"] = json!("");
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
            e["outcome"]["delivered"] = json!(-1);
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
    let named = |result: Result<(), Violations>| {
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
