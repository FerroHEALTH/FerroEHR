// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The mapping of a stored entry to each projection, row by row: where each
//! value of the entry is carried, and each value a projection loses
//! (`website/book/src/audit.md`, "The projections and their losses").
//!
//! Each table is one test of named rows, so a failure names the row.

#![expect(
    clippy::disallowed_types,
    reason = "the rows read the projected FHIR document as the JSON it is served as"
)]

use secrecy::ExposeSecret as _;
use serde_json::Value;

use super::super::Projection;
use super::{EHR, TEMPLATE, VERSION, classified_read, policy, stored_entry};
use crate::system_log::access_binding::tests::{organisation, representative};
use crate::system_log::event::{
    AccessDecision, AuditEvent, EntryScope, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use crate::system_log::render::CompiledPolicy;
use ehds_logging::record::{Acting, Action, Outcome, PatientLookup};

/// The projections of the entry `event` writes, under `policy`.
struct Projected {
    entry: Value,
    document: Value,
    message: String,
    record: Option<ehds_logging::record::AccessRecord>,
}

fn project(policy: &CompiledPolicy, event: &AuditEvent, subject: Option<&str>) -> Projected {
    let projection = Projection {
        context: &policy.context,
        categories: &policy.categories,
    };
    let entry = stored_entry(event, subject);
    let message = projection.audit_message(&entry, 1).expect("a message");
    Projected {
        document: projection.audit_event(&entry, 1).expect("a document"),
        message: String::from_utf8(message.expose_secret().to_vec()).expect("UTF-8"),
        record: projection.access_record(&entry, 1).expect("a record"),
        entry: serde_json::to_value(&entry).expect("the entry"),
    }
}

/// The entities of `document`.
fn entities(document: &Value) -> &[Value] {
    document["entity"].as_array().map_or(&[], Vec::as_slice)
}

/// The entity of `document` named `name`.
fn named<'a>(document: &'a Value, name: &str) -> Option<&'a Value> {
    entities(document)
        .iter()
        .find(|entity| entity["name"] == name)
}

/// Whether the entity named `name` carries the detail `kind` with `value`.
fn detail(document: &Value, name: &str, kind: &str, value: &str) -> bool {
    named(document, name).is_some_and(|entity| {
        entity["detail"].as_array().is_some_and(|details| {
            details
                .iter()
                .any(|d| d["type"] == kind && d["valueString"] == value)
        })
    })
}

/// The agent of `document` that is the requestor.
fn user(document: &Value) -> &Value {
    document["agent"]
        .as_array()
        .and_then(|agents| agents.iter().find(|agent| agent["requestor"] == true))
        .unwrap_or(&Value::Null)
}

/// Whether the agent `agent` carries an `ihe-otherId` of type `kind` with
/// `value`.
fn other_id(agent: &Value, kind: &str, value: &str) -> bool {
    agent["extension"].as_array().is_some_and(|extensions| {
        extensions.iter().any(|x| {
            x["valueIdentifier"]["type"]["coding"][0]["code"] == kind
                && x["valueIdentifier"]["value"] == value
        })
    })
}

/// A row of a mapping table: its name and whether it holds.
type Row = (&'static str, bool);

/// Every row that does not hold, by name.
fn failing(rows: &[Row]) -> Vec<&'static str> {
    rows.iter()
        .filter(|(_, holds)| !holds)
        .map(|(name, _)| *name)
        .collect()
}

/// A stored query's execution, which the ad-hoc text never reaches.
fn stored_query(policy: &CompiledPolicy) -> AuditEvent {
    let mut event = classified_read(policy);
    event.action = EventActionCode::Execute;
    event.object = ObjectClass::Query;
    event.event_type = Some(EventType::rest("query_execute_stored_query"));
    event.object_id = Some("org.example::vitals".to_owned());
    event.query = Some(crate::system_log::event::QueryNaming::of(
        "AQL",
        "SELECT c FROM EHR e CONTAINS COMPOSITION c",
        Some("org.example::vitals".to_owned()),
    ));
    event.result_count = Some(4);
    event
}

/// A refused read and a failed write, each outcome a row of its own.
fn outcomes(policy: &CompiledPolicy) -> (Projected, Projected) {
    let mut refused = classified_read(policy);
    refused.outcome = EventOutcome::MinorFailure;
    refused.decision = AccessDecision::Refused;
    refused.entry = EntryScope::Refusal;
    let mut failed = classified_read(policy);
    failed.action = EventActionCode::Update;
    failed.outcome = EventOutcome::SeriousFailure;
    (
        project(policy, &refused, Some("subject-1")),
        project(policy, &failed, Some("subject-1")),
    )
}

/// An emergency access: a purpose of use the deployment declares as one.
fn emergency(policy: &CompiledPolicy) -> Projected {
    let mut event = classified_read(policy);
    event.token = None;
    event.purpose = Some("ETREAT".to_owned());
    project(policy, &event, Some("subject-1"))
}

/// An export of the EHR as an extract.
fn export(policy: &CompiledPolicy) -> Projected {
    let mut event = classified_read(policy);
    event.object = ObjectClass::Extract;
    event.event_type = Some(EventType::rest("ehr_extract_export"));
    project(policy, &event, Some("subject-1"))
}

mod balp;
mod dicom;
mod ehds;
