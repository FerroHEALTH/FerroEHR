// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access records for what one response served beyond the object it
//! addressed: each EHR an AQL statement served, and each party a party read
//! disclosed through `reverse_relationships`.
//!
//! The handler that knows what it served sets a response extension, and the
//! audit middleware ([`crate::system_log::middleware`]) writes one record per
//! entry beside the operation's own, so the caller identity stays in the one
//! place that has it. No openEHR spec governs the audit trail: our own design,
//! after IHE ATNA.

use std::collections::BTreeMap;

use axum::response::Response;

use ferroehr::system_log::categories::{AccessedContent, ContentIds};
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass, QueryNaming,
};

use crate::state::AppState;
use crate::system_log::middleware::{AccessContext, Caller, fill_access, fill_common};

/// The access records for everything the response served beyond the
/// addressed object: [`AuditServedEhrs`] and [`AuditServedParties`].
pub(super) fn records(
    state: &AppState,
    resp: &Response,
    op: &'static str,
    caller: Caller<'_>,
    client_ip: Option<&str>,
    timestamp: jiff::Timestamp,
    access: &AccessContext,
) -> Vec<AuditEvent> {
    let mut records = served_ehr_records(state, resp, op, caller, client_ip, timestamp, access);
    records.extend(served_party_records(
        resp, op, caller, client_ip, timestamp, access, state,
    ));
    records
}

/// The EHRs a query served, each with its served-row count and the
/// identifiers of the versions it served from that EHR.
///
/// A response extension the query dispatch sets and the audit middleware
/// reads: an AQL statement is one operation but potentially many accesses, and
/// NEN 7513 and EHDS Art. 9 both ask which records were accessed. Carrying it
/// here rather than emitting from the service keeps the caller identity in the
/// one place that has it.
#[derive(Debug, Clone)]
pub struct AuditServedEhrs {
    /// The served EHRs with their served-row counts, in first-served order.
    pub served: Vec<(String, u64)>,
    /// The identifiers of the versions served from each EHR, keyed by EHR id.
    pub content: BTreeMap<String, Vec<ContentIds>>,
    /// The statement that served them, named and never quoted, which each
    /// EHR's access-log entry names.
    pub query: Option<QueryNaming>,
}

/// The parties a read disclosed beside the one it addressed: the source
/// parties named by a target party's `reverse_relationships`.
#[derive(Debug, Clone)]
pub struct AuditServedParties {
    /// The disclosed party container ids, in first-served order.
    pub parties: Vec<String>,
}

/// One access record per EHR a query served, beside the statement's own
/// operation record.
///
/// The statement record answers "who ran what"; these answer "whose record was
/// disclosed", which is the question NEN 7513 and EHDS Art. 9 put per record.
fn served_ehr_records(
    state: &AppState,
    resp: &Response,
    op: &'static str,
    caller: Caller<'_>,
    client_ip: Option<&str>,
    timestamp: jiff::Timestamp,
    access: &AccessContext,
) -> Vec<AuditEvent> {
    let Some(AuditServedEhrs {
        served,
        content,
        query,
    }) = resp.extensions().get::<AuditServedEhrs>()
    else {
        return Vec::new();
    };
    let mut records = Vec::with_capacity(served.len());
    for (ehr_id, rows) in served {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::rest(op));
        fill_common(&mut event, caller, client_ip.map(str::to_owned), timestamp);
        event.ehr_id = Some(ehr_id.clone());
        event.object_id = Some(ehr_id.clone());
        event.result_count = Some(*rows);
        event.query.clone_from(query);
        caller.stamp_decision(&mut event);
        event.category = content.get(ehr_id).and_then(|objects| {
            state
                .backend()
                .audit_classify(&AccessedContent::Objects(objects.clone()), event.action)
        });
        fill_access(&mut event, access, state);
        records.push(event);
    }
    records
}

/// One access record per party a read disclosed beside the one it
/// addressed ([`AuditServedParties`]), under the operation's own event type.
fn served_party_records(
    resp: &Response,
    op: &'static str,
    caller: Caller<'_>,
    client_ip: Option<&str>,
    timestamp: jiff::Timestamp,
    access: &AccessContext,
    state: &AppState,
) -> Vec<AuditEvent> {
    let Some(AuditServedParties { parties }) = resp.extensions().get::<AuditServedParties>() else {
        return Vec::new();
    };
    let mut records = Vec::with_capacity(parties.len());
    for party in parties {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::rest(op));
        fill_common(&mut event, caller, client_ip.map(str::to_owned), timestamp);
        event.object_id = Some(party.clone());
        fill_access(&mut event, access, state);
        records.push(event);
    }
    records
}
