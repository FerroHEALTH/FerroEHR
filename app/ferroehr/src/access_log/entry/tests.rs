// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

use super::{entry_of, horizon_retention, writes_entry};
use crate::access_log::store::canonical_form;
use crate::system_log::binding::{BindingRecord, Identified, PartyTriple};
use crate::system_log::event::{
    AccessDecision, AccessDomain, AuditEvent, EntryScope, EventActionCode, EventOutcome, EventType,
    ObjectClass, QueryNaming,
};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;
use openehr_rm::draft::access_log::invariants::check_entry;

const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
const VERSION: &str = "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1";
const PARTY: &str = "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5";

fn retention() -> AccessRetention {
    horizon_retention(Some(jiff::civil::date(2031, 11, 8)))
}

/// `entry` with the link a chain would give it, checked against every
/// invariant of the draft.
fn linked(mut entry: AccessLogEntry) -> AccessLogEntry {
    entry.link = AccessLogLink {
        sequence: 1,
        algorithm: "SHA-256".to_owned(),
        previous_digest: "0".repeat(64),
        content_digest: "1".repeat(64),
        digest: "2".repeat(64),
        signature: None,
    };
    assert_eq!(check_entry(&entry), Ok(()), "{entry:?}");
    entry
}

/// A bound clinician's granted read of one composition version.
fn composition_read() -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::rest("composition_get"));
    event.user_id = "alice".to_owned();
    event.issuer = Some("https://login.example.org".to_owned());
    event.token_id = Some("jti-7f3e".to_owned());
    event.ehr_id = Some(EHR.to_owned());
    event.object_id = Some(VERSION.to_owned());
    event.request_id = Some("01JC2Q7N3X".to_owned());
    event.purpose = Some("TREAT".to_owned());
    event.result_count = Some(1);
    event.origins = vec!["cdr.example.org".to_owned()];
    event.binding = Some(BindingRecord::bound(
        PartyTriple {
            namespace: "demographic".to_owned(),
            class: "PERSON".to_owned(),
            id: PARTY.to_owned(),
        },
        false,
        vec!["c1a9e0e4-0d8f-4d9c-9b51-3f6a2f1b7c11".to_owned()],
        Some(Identified {
            issuer: "urn:oid:2.999.7.9".to_owned(),
            id: "hospital".to_owned(),
        }),
        false,
    ));
    event
}

#[test]
fn a_granted_read_names_the_version_the_party_and_the_request() {
    let entry = linked(
        entry_of(&composition_read(), None, "cdr.example.org", retention()).expect("an entry"),
    );
    assert_eq!(entry.action.value, "read");
    assert_eq!(entry.outcome.result.value, "granted");
    assert_eq!(entry.outcome.delivered, Some(1));
    assert_eq!(entry.ehr_id.as_ref().map(HierObjectId::value), Some(EHR));
    let items = entry.items.as_deref().expect("an item");
    assert_eq!(
        items[0].version_id.as_ref().map(ObjectVersionId::value),
        Some(VERSION)
    );
    assert!(items[0].was_delivered);
    assert_eq!(
        entry.channel.as_ref().and_then(|c| c.request_id.as_deref()),
        Some("01JC2Q7N3X")
    );
    assert_eq!(
        entry.accessor.binding.as_ref().map(|b| b.value.as_str()),
        Some("bound")
    );
    assert_eq!(
        entry
            .retention
            .retain_until
            .as_ref()
            .map(|d| d.value.as_str()),
        Some("2031-11-07"),
        "kept through the day before the first date it may be removed on"
    );
    let written = serde_json::to_value(&entry).expect("the entry writes");
    let read: AccessLogEntry = serde_json::from_value(written).expect("the entry reads back");
    assert_eq!(read, entry);
}

#[test]
fn a_refusal_with_no_operation_record_is_an_entry_without_an_ehr() {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::MinorFailure,
    );
    event.event_type = Some(EventType::Login);
    event.user_id = "UNKNOWN".to_owned();
    event.decision = AccessDecision::Refused;
    assert!(
        !writes_entry(&event),
        "a refusal an operation record covers"
    );
    event.entry = EntryScope::Refusal;
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert_eq!(entry.outcome.result.value, "refused");
    assert_eq!(entry.outcome.delivered, None);
    assert!(entry.ehr_id.is_none());
    assert!(entry.items.is_none());
}

#[test]
fn a_refused_read_is_refused_and_delivers_nothing() {
    let mut event = composition_read();
    event.outcome = EventOutcome::MinorFailure;
    event.decision = AccessDecision::Refused;
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert_eq!(entry.outcome.result.value, "refused");
    assert_eq!(entry.outcome.delivered, None);
    assert!(!entry.items.as_deref().expect("an item")[0].was_delivered);
}

#[test]
fn a_failed_access_is_failed() {
    let mut event = composition_read();
    event.outcome = EventOutcome::SeriousFailure;
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert_eq!(entry.outcome.result.value, "failed");
}

#[test]
fn a_query_is_named_by_its_digest_and_never_quoted() {
    let text = "SELECT c FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = 'secret-value'";
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::rest("query_execute_adhoc_query_body"));
    event.ehr_id = Some(EHR.to_owned());
    event.object_id = Some(EHR.to_owned());
    event.result_count = Some(3);
    event.query = Some(QueryNaming::of("AQL", text, None));
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert_eq!(entry.action.value, "query", "an EHR a query served");
    let query = entry.query.as_ref().expect("the query");
    assert_eq!(query.language.as_deref(), Some("AQL"));
    assert_eq!(query.rows, Some(3));
    assert_eq!(query.text, None);
    assert_eq!(
        query.text_digest.as_deref().map(str::len),
        Some(64),
        "a SHA-256 in hexadecimal"
    );
    let canonical = canonical_form(&entry).expect("the canonical form");
    assert!(!canonical.contains("secret-value"), "{canonical}");
    let item = &entry.items.as_deref().expect("the EHR item")[0];
    assert_eq!(item.rm_type.as_deref(), Some("EHR"));
}

#[test]
fn a_read_of_the_log_is_an_entry_naming_the_log_by_its_search() {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::ApplicationActivity,
        EventOutcome::Success,
    );
    event.domain = AccessDomain::System;
    event.event_type = Some(EventType::rest("audit_event_search"));
    event.user_id = "root".to_owned();
    assert!(!writes_entry(&event));
    event.entry = EntryScope::LogRead;
    event.query = Some(QueryNaming::of(
        super::LOG_SEARCH,
        "date=ge2026-01-01&patient=p1",
        None,
    ));
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert_eq!(entry.action.value, "read");
    assert_eq!(
        entry.operation.as_ref().map(|op| op.value.as_str()),
        Some("audit_event_search")
    );
    assert_eq!(
        entry.query.as_ref().and_then(|q| q.language.as_deref()),
        Some(super::LOG_SEARCH)
    );
}

#[test]
fn an_access_outside_the_ehr_writes_no_entry() {
    let event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Demographic,
        EventOutcome::Success,
    );
    assert!(entry_of(&event, None, "cdr.example.org", retention()).is_none());
}

#[test]
fn an_unparsable_ehr_segment_names_no_ehr() {
    let mut event = composition_read();
    event.ehr_id = Some("not an id".to_owned());
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    assert!(entry.ehr_id.is_none());
    assert!(entry.items.is_none(), "an item needs its EHR");
}

#[test]
fn the_decision_grounds_slot_is_carried_into_the_authorisations() {
    let mut event = composition_read();
    event.permit = Some("hdab-nl/2026/0042".to_owned());
    event.emergency_access = true;
    let entry = linked(entry_of(&event, None, "cdr.example.org", retention()).expect("an entry"));
    let kinds: Vec<&str> = entry
        .authorisations
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|a| a.kind.value.as_str())
        .collect();
    assert_eq!(kinds, ["permit", "emergency"]);
    assert!(entry.is_emergency);
}
