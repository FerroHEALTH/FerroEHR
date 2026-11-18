// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parts written straight from a record are the canonical forms of the
//! parts [`super::super::split`] makes of its JSON value, over generated
//! entry-shaped values and over entries built from generated audit events
//! (#5849).

#![expect(
    clippy::disallowed_types,
    reason = "the reference the parts are held to is computed over the record's ITS-JSON value \
              (#1694 boundary class: canonical form)"
)]

use proptest::prelude::*;
use serde_json::{Map, Value};

use super::{Parts, parts};
use crate::access_log::entry::entry_of;
use crate::access_log::retention::stated;
use crate::access_log::stored_form::{SHARED, SHARED_DETAILS, split};
use crate::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass, QueryNaming,
};
use crate::system_log::retention::LogRetention;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::{
    canonical_json, canonical_object,
};

/// The member the chain fills and the stored parts leave out.
const LINK: &str = "link";

/// The parts as the `Value` path made them: `record`'s JSON value without
/// `link`, split, each part canonicalised.
fn through_value(record: &Value) -> Parts {
    let Value::Object(members) = record else {
        panic!("a record is an object");
    };
    let mut members = members.clone();
    members.remove(LINK);
    let whole = canonical_json(&Value::Object(members.clone())).expect("the whole canonicalises");
    let form = split(members);
    Parts {
        whole,
        row: canonical_json(&form.row).expect("the row canonicalises"),
        shared: form
            .shared
            .map(|shared| canonical_json(&shared).expect("the shared part canonicalises")),
    }
}

/// A JSON leaf.
fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter_map("a finite double", serde_json::Number::from_f64)
            .prop_map(Value::Number),
        "[ -~é→\"\\\\\n]{0,12}".prop_map(Value::String),
    ]
}

/// A JSON value of a few levels.
fn value() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::btree_map("[a-z_]{1,6}", inner, 0..4)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

/// An object whose members are named from `names`.
fn object_of(names: &'static [&'static str]) -> impl Strategy<Value = Value> {
    proptest::collection::btree_map(proptest::sample::select(names), value(), 0..names.len())
        .prop_map(|members| {
            Value::Object(
                members
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value))
                    .collect(),
            )
        })
}

/// An element of the access details: a shared or own at-code, a code that
/// is no string, or none.
fn element() -> impl Strategy<Value = Value> {
    let code = prop_oneof![
        proptest::sample::select(SHARED_DETAILS.to_vec()).prop_map(Value::from),
        Just(Value::from("at0012")),
        Just(Value::from("at0018")),
        any::<i64>().prop_map(Value::from),
        Just(Value::Null),
    ];
    (proptest::option::of(code), value(), any::<bool>()).prop_map(|(code, payload, as_array)| {
        if as_array {
            return payload;
        }
        let mut element = Map::new();
        element.insert("_type".to_owned(), Value::from("ELEMENT"));
        if let Some(code) = code {
            element.insert("archetype_node_id".to_owned(), code);
        }
        element.insert("value".to_owned(), payload);
        Value::Object(element)
    })
}

/// The details tree: its envelope and its elements, or another shape.
fn details() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => (
            proptest::collection::vec(element(), 0..8),
            any::<bool>(),
            value()
        )
            .prop_map(|(items, with_items, extra)| {
                let mut tree = Map::new();
                tree.insert("_type".to_owned(), Value::from("ITEM_TREE"));
                tree.insert("archetype_node_id".to_owned(), Value::from("at0000"));
                tree.insert("extra".to_owned(), extra);
                if with_items {
                    tree.insert("items".to_owned(), Value::Array(items));
                }
                Value::Object(tree)
            }),
        1 => value(),
    ]
}

/// A record shaped like an access-log entry: a top-level member of every
/// kind the split routes, each present or not, of the expected shape or not.
fn record() -> impl Strategy<Value = Value> {
    let shared = proptest::collection::btree_map(
        proptest::sample::select(SHARED.to_vec()),
        value(),
        0..SHARED.len(),
    );
    let own = proptest::collection::btree_map(
        proptest::sample::select(vec![
            "_type",
            "uid",
            "time_recorded",
            "ehr_id",
            "items",
            "channel",
        ]),
        value(),
        0..6,
    );
    let outcome = prop_oneof![object_of(&["result", "delivered", "_type"]), value()];
    let query = prop_oneof![object_of(&["language", "rows", "text_digest"]), value()];
    (
        shared,
        own,
        proptest::option::of(outcome),
        proptest::option::of(query),
        proptest::option::of(details()),
        proptest::option::of(value()),
    )
        .prop_map(|(shared, own, outcome, query, details, link)| {
            let mut record = Map::new();
            for (name, member) in shared.into_iter().chain(own) {
                record.insert(name.to_owned(), member);
            }
            for (name, member) in [
                ("outcome", outcome),
                ("query", query),
                ("other_details", details),
                (LINK, link),
            ] {
                if let Some(member) = member {
                    record.insert(name.to_owned(), member);
                }
            }
            Value::Object(record)
        })
}

/// An audit event of every kind an entry is written for.
pub(crate) fn event() -> impl Strategy<Value = AuditEvent> {
    let action = proptest::sample::select(vec![
        EventActionCode::Create,
        EventActionCode::Read,
        EventActionCode::Update,
        EventActionCode::Delete,
        EventActionCode::Execute,
    ]);
    let object = proptest::sample::select(vec![
        ObjectClass::Ehr,
        ObjectClass::Composition,
        ObjectClass::Contribution,
        ObjectClass::Directory,
        ObjectClass::Query,
    ]);
    let outcome = proptest::sample::select(vec![
        EventOutcome::Success,
        EventOutcome::MinorFailure,
        EventOutcome::SeriousFailure,
    ]);
    let text = || "[ -~é→]{1,16}";
    (
        (action, object, outcome),
        (
            text(),
            proptest::option::of(text()),
            proptest::option::of(text()),
        ),
        (
            proptest::option::of(text()),
            proptest::option::of(text()),
            proptest::option::of(any::<u64>()),
            proptest::option::of(prop_oneof![
                Just("8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1".to_owned()),
                text(),
            ]),
        ),
        (
            proptest::collection::vec(text(), 0..3),
            proptest::collection::vec(text(), 0..3),
            proptest::option::of(text()),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                (action, object, outcome),
                (user, issuer, token),
                (purpose, request, count, object_id),
                more,
            )| {
                let (roles, origins, query, emergency) = more;
                let mut event = AuditEvent::new(action, object, outcome);
                event.event_type = Some(EventType::rest("composition_get"));
                event.user_id = user;
                event.issuer = issuer;
                event.token_id = token;
                event.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
                event.object_id = object_id;
                event.purpose = purpose;
                event.request_id = request;
                event.result_count = count;
                event.roles = roles;
                event.origins = origins;
                event.emergency_access = emergency;
                event.query = query.map(|text| QueryNaming::of("AQL", &text, None));
                event
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    #[test]
    fn the_parts_of_a_generated_record_are_those_its_value_splits_into(record in record()) {
        let written = parts(&record, &[LINK]).expect("the parts write");
        prop_assert_eq!(written, through_value(&record));
    }

    #[test]
    fn the_parts_of_an_entry_are_those_its_value_splits_into(event in event()) {
        let retention = stated(Some(jiff::civil::date(2031, 11, 8)), &LogRetention::default());
        if let Some(entry) = entry_of(&event, None, "cdr.example.org", retention) {
            let value = serde_json::to_value(&entry).expect("the entry serialises");
            let written = parts(&entry, &[LINK]).expect("the parts write");
            prop_assert_eq!(written, through_value(&value));
            let Value::Object(members) = &value else {
                panic!("an entry is an object");
            };
            let whole = canonical_object(
                members
                    .iter()
                    .filter(|(name, _)| name.as_str() != LINK && name.as_str() != "proof")
                    .map(|(name, member)| (name.as_str(), member)),
            )
            .expect("the whole form canonicalises");
            prop_assert_eq!(
                crate::access_log::store::canonical_form(&entry).expect("the canonical form"),
                whole
            );
        }
    }
}

/// The columns an intake row keeps for the record whose canonical form is
/// `whole`, as the writer derives them.
fn columns_of(whole: &str) -> (String, uuid::Uuid, jiff::Timestamp, Option<uuid::Uuid>) {
    let cut = super::Cut::of(whole).expect("the whole reads");
    let text = |path: &[&str]| cut.text_at(path).expect("a text reads");
    let record_type = text(&["_type"]).expect("a record names its type");
    let uid = text(&["uid", "value"])
        .and_then(|uid| uuid::Uuid::parse_str(&uid).ok())
        .unwrap_or_else(uuid::Uuid::nil);
    let time = text(&["time_recorded", "value"])
        .and_then(|at| at.parse::<jiff::Timestamp>().ok())
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    let ehr_id =
        text(&["ehr_id", "value"]).and_then(|id| crate::access_log::stored_form::column_uuid(&id));
    (record_type, uid, time, ehr_id)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// An entry's stored parts and columns assemble back into the exact
    /// canonical form its content digest was computed over, whatever its
    /// index columns hold: the request and object of its event, as the
    /// writer indexes them, or any other text.
    #[test]
    fn an_entrys_parts_assemble_into_its_canonical_form(
        event in event(),
        indexed in any::<bool>(),
        request_id in proptest::option::of("[ -~é]{0,8}"),
        resource_id in proptest::option::of(prop_oneof![
            Just("8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1".to_owned()),
            "[ -~é]{0,8}",
        ]),
    ) {
        let retention = stated(Some(jiff::civil::date(2031, 11, 8)), &LogRetention::default());
        if let Some(entry) = entry_of(&event, None, "cdr.example.org", retention) {
            let index = if indexed {
                super::IndexColumns {
                    request_id: event.request_id.as_deref(),
                    resource_id: event.object_id.as_deref(),
                }
            } else {
                super::IndexColumns {
                    request_id: request_id.as_deref(),
                    resource_id: resource_id.as_deref(),
                }
            };
            let (joined, whole) = assembled(&entry, index);
            prop_assert_eq!(joined, whole);
        }
    }
}

/// A composition read whose entry has access details on both parts: the
/// caller's (principal, roles) shared, the access's own (origins, count) on
/// the row, with a delivered count and a query.
fn read_entry() -> openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::rest("composition_get"));
    "nurse".clone_into(&mut event.user_id);
    event.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
    event.object_id = Some("8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1".to_owned());
    event.request_id = Some("d44e8723-cf7e-4b37-8f52-60e1b3cd5f88".to_owned());
    event.roles = vec!["USER".to_owned(), "NURSE".to_owned()];
    event.origins = vec!["cdr.example.org".to_owned(), "other.example.org".to_owned()];
    event.result_count = Some(3);
    event.query = Some(QueryNaming::of("AQL", "SELECT c FROM COMPOSITION c", None));
    event.timestamp = "2026-11-18T10:11:12.3456Z".parse().expect("an instant");
    let retention = stated(
        Some(jiff::civil::date(2031, 11, 8)),
        &LogRetention::default(),
    );
    entry_of(&event, Some("patient-1"), "cdr.example.org", retention)
        .expect("a read writes an entry")
}

/// The form `record`'s stored parts, the columns of its row and the index
/// columns `index` assemble into, and its canonical form.
fn assembled<T: serde::Serialize>(record: &T, index: super::IndexColumns<'_>) -> (String, String) {
    let written = parts(record, &[LINK]).expect("the parts write");
    let row = super::without_columns(&written.row, &index).expect("the row part reads");
    let (record_type, uid, time_recorded, ehr_id) = columns_of(&written.whole);
    let columns = super::RowColumns {
        record_type: &record_type,
        uid,
        time_recorded,
        ehr_id,
        index,
    };
    let whole =
        super::whole(&row, written.shared.as_deref(), &columns).expect("the parts assemble");
    (whole, written.whole)
}

/// The index columns the writer keeps for `read_entry`.
const READ_INDEX: super::IndexColumns<'static> = super::IndexColumns {
    request_id: Some("d44e8723-cf7e-4b37-8f52-60e1b3cd5f88"),
    resource_id: Some("8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1"),
};

/// The row part leaves the channel's request and the item's version to the
/// index columns that hold them, and names a member `null` a reader would
/// otherwise give back.
#[test]
fn the_row_part_leaves_the_request_and_version_to_their_columns() {
    let entry = read_entry();
    let written = parts(&entry, &[LINK]).expect("the parts write");
    let row = super::without_columns(&written.row, &READ_INDEX).expect("the row part reads");
    assert!(written.row.contains("d44e8723") && written.row.contains("48ead4180515"));
    assert!(
        !row.contains("d44e8723") && !row.contains("48ead4180515") && !row.contains("null"),
        "{row}"
    );
    assert_eq!(assembled(&entry, READ_INDEX).0, written.whole);
    let other = super::IndexColumns {
        request_id: Some("another-request"),
        resource_id: Some("another::system::1"),
    };
    let kept = super::without_columns(&written.row, &other).expect("the row part reads");
    assert!(
        kept.contains("d44e8723") && kept.contains("48ead4180515"),
        "{kept}"
    );
    let mut unversioned = read_entry();
    if let Some(item) = unversioned
        .items
        .as_mut()
        .and_then(|items| items.first_mut())
    {
        item.version_id = None;
    }
    let written = parts(&unversioned, &[LINK]).expect("the parts write");
    let row = super::without_columns(&written.row, &READ_INDEX).expect("the row part reads");
    assert!(row.contains("\"version_id\":null"), "{row}");
    assert_eq!(assembled(&unversioned, READ_INDEX).0, written.whole);
}

/// The access details of both parts merge by their at-codes, `outcome` and
/// `query` by their members, and the columns give back `_type`, `uid`,
/// `time_recorded` and `ehr_id`.
#[test]
fn the_columns_and_both_parts_assemble_back() {
    let entry = read_entry();
    let written = parts(&entry, &[LINK]).expect("the parts write");
    assert!(
        written.row.contains("at0018") && !written.row.contains("at0004"),
        "{}",
        written.row
    );
    let (whole, original) = assembled(&entry, READ_INDEX);
    assert_eq!(whole, original);
}

/// An id or instant the columns cannot give back stays in the row part,
/// which replaces the column's member.
#[test]
fn a_member_the_row_part_keeps_replaces_the_columns() {
    use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
    let mut entry = read_entry();
    entry.uid = HierObjectId::new("1.2.840.10008.3.1.2.3.4").expect("an OID uid");
    entry.ehr_id = Some(HierObjectId::new("2.16.840.1.113883.2.4.6.3").expect("an OID EHR id"));
    entry.time_recorded = crate::versioning::audit::dv_date_time(
        &"2026-11-18T09:11:12Z".parse().expect("an instant"),
    );
    "2026-11-18T10:11:12+01:00".clone_into(&mut entry.time_recorded.value);
    let (whole, original) = assembled(&entry, READ_INDEX);
    assert_eq!(whole, original);
}

/// A tombstone keeps no shared part: its row part and columns are the whole.
#[test]
fn a_tombstone_assembles_from_its_row() {
    let entry = read_entry();
    let grounds = entry.retention.grounds.clone();
    let tombstone = crate::access_log::store::tombstone_of(
        entry,
        crate::versioning::audit::dv_date_time(
            &"2031-11-09T00:00:00Z".parse().expect("an instant"),
        ),
        grounds,
    );
    let written = parts(&tombstone, &[LINK]).expect("the parts write");
    assert_eq!(written.shared, None);
    let (whole, original) = assembled(&tombstone, READ_INDEX);
    assert_eq!(whole, original);
}

/// A record that is no object has no parts.
#[test]
fn a_record_that_is_no_object_is_refused() {
    assert!(parts(&Value::from(1), &[LINK]).is_err());
    assert!(parts(&Value::Array(Vec::new()), &[LINK]).is_err());
}
