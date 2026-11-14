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

/// The projections the BALP rows read.
struct Balp {
    read: Projected,
    refused: Projected,
    failed: Projected,
    query: Projected,
    represented: Projected,
    relayed: Projected,
    urgent: Projected,
    exported: Projected,
}

/// The rows of the mapping to the FHIR R4 `AuditEvent` under IHE BALP 1.1.4.
#[test]
fn the_balp_audit_event_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let read = project(&policy, &classified_read(&policy), Some("subject-1"));
    let (refused, failed) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
    let represented = {
        let mut event = classified_read(&policy);
        event.binding = Some(representative());
        project(&policy, &event, Some("subject-9"))
    };
    let relayed = {
        let mut event = classified_read(&policy);
        event.binding = Some(organisation());
        project(&policy, &event, Some("subject-1"))
    };
    let urgent = emergency(&policy);
    let exported = export(&policy);
    let f = Balp {
        read,
        refused,
        failed,
        query,
        represented,
        relayed,
        urgent,
        exported,
    };
    let rows = [
        balp_event_rows(&f),
        balp_accessor_rows(&f),
        balp_object_rows(&f),
        balp_authorisation_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{}", f.read.document);
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_event_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let failed = &f.failed;
    let exported = &f.exported;
    let doc = &read.document;
    vec![
        (
            "action: read is action R, type rest, subtype read",
            read.entry["action"]["value"] == "read"
                && doc["action"] == "R"
                && doc["type"]["code"] == "rest"
                && doc["subtype"][0]["code"] == "read",
        ),
        (
            "action, loss: the openEHR action is written in no code system of its own",
            doc["subtype"]
                .as_array()
                .map(|subtypes| {
                    subtypes
                        .iter()
                        .filter_map(|s| s["system"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                == [
                    "http://hl7.org/fhir/restful-interaction",
                    "urn:openehr:its-rest:operation",
                ],
        ),
        (
            "action, loss: export folds into R",
            exported.entry["action"]["value"] == "export" && exported.document["action"] == "R",
        ),
        (
            "operation: a subtype in the ITS-REST operation system",
            doc["subtype"][1]["system"] == "urn:openehr:its-rest:operation"
                && doc["subtype"][1]["code"] == "composition_get",
        ),
        (
            "time_recorded: recorded",
            doc["recorded"] == read.entry["time_recorded"]["value"],
        ),
        ("occurred, loss: no period", doc.get("period").is_none()),
        (
            "outcome granted: 0",
            doc["outcome"] == "0" && read.entry["outcome"]["result"]["value"] == "granted",
        ),
        (
            "outcome refused: 4",
            refused.document["outcome"] == "4"
                && refused.entry["outcome"]["result"]["value"] == "refused",
        ),
        (
            "outcome failed: 8 for a serious failure",
            failed.document["outcome"] == "8"
                && failed.entry["outcome"]["result"]["value"] == "failed",
        ),
        (
            "outcome, loss: the description is the outcome's text, never a reason code",
            doc["outcomeDesc"] == "Operation performed successfully"
                && read.entry["outcome"].get("reason").is_none(),
        ),
        (
            "outcome.delivered: the delivered detail of ehds-categories",
            detail(doc, "ehds-categories", "delivered", "1"),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_accessor_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let doc = &read.document;
    vec![
        (
            "accessor.party: the PRN otherId of agent:user",
            other_id(user(doc), "PRN", "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"),
        ),
        (
            "accessor.credential: issuer and subject are agent:user who.identifier",
            user(doc)["who"]["identifier"]["system"] == "https://login.example.org"
                && user(doc)["who"]["identifier"]["value"] == "alice",
        ),
        (
            "accessor.credential: the token id is the first policy of agent:user",
            user(doc)["policy"][0] == "jti-7f3e",
        ),
        (
            "accessor.credential: the client is an agent of its own",
            doc["agent"].as_array().is_some_and(|agents| {
                agents
                    .iter()
                    .any(|a| a["who"]["identifier"]["value"] == "ward-app")
            }),
        ),
        (
            "accessor.credential: the assurance is the ihe-assuranceLevel of agent:user",
            user(doc)["extension"].as_array().is_some_and(|x| {
                x.iter()
                    .any(|e| e["valueCodeableConcept"]["coding"][0]["code"] == "high")
            }),
        ),
        (
            "accessor.provider: an agent naming the organisation",
            doc["agent"].as_array().is_some_and(|agents| {
                agents.iter().any(|a| {
                    a["who"]["type"] == "Organization"
                        && a["who"]["identifier"]["value"] == "Example Hospital"
                })
            }),
        ),
        (
            "accessor.roles and kind: agent:user role, the acting mode first",
            user(doc)["role"][0]["coding"][0]["code"] == "client"
                && user(doc)["role"][1]["coding"][0]["code"] == "clinician",
        ),
        (
            "accessor.groups and binding: details of ehds-accessor",
            detail(doc, "ehds-accessor", "binding", "bound")
                && detail(
                    doc,
                    "ehds-accessor",
                    "group",
                    "demographic|GROUP|c1a9e0e4-0d8f-4d9c-9b51-3f6a2f1b7c11",
                ),
        ),
        (
            "accessor.delegation: each actor a detail of ehds-accessor, in order",
            detail(
                &represented.document,
                "ehds-accessor",
                "actor",
                "https://idp.example.org|app-1",
            ),
        ),
        (
            "authorisations, representation: a detail of ehds-accessor",
            named(&represented.document, "ehds-accessor")
                .is_some_and(|entity| entity.to_string().contains("subject-9")),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_object_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let doc = &read.document;
    vec![
        (
            "ehr_id: the ehr entity, the subject of care the patient entity",
            named(doc, "ehr").is_some_and(|e| e["what"]["identifier"]["value"] == EHR)
                && entities(doc).iter().any(|e| {
                    e["role"]["code"] == "1" && e["what"]["identifier"]["value"] == "subject-1"
                }),
        ),
        (
            "items: entity:data names the version in urn:openehr:object-id, type 2, role 4",
            entities(doc).iter().any(|e| {
                e["what"]["identifier"]["system"] == "urn:openehr:object-id"
                    && e["what"]["identifier"]["value"] == VERSION
                    && e["type"]["code"] == "2"
                    && e["role"]["code"] == "4"
            }),
        ),
        (
            "items.categories and template: details of ehds-categories",
            detail(doc, "ehds-categories", "template-id", TEMPLATE)
                && named(doc, "ehds-categories")
                    .is_some_and(|e| e.to_string().contains("Laboratory-Reports")),
        ),
        (
            "items.origins: one origin entity each",
            entities(doc)
                .iter()
                .filter(|e| e["name"] == "origin")
                .count()
                == 2,
        ),
        (
            "query: entity:query, the stored query's name a detail",
            entities(&query.document)
                .iter()
                .any(|e| e["role"]["code"] == "24")
                && detail(&query.document, "ehds-categories", "delivered", "4"),
        ),
        (
            "query, loss: the text digest is not written",
            !query
                .document
                .to_string()
                .contains(query.entry["query"]["text_digest"].as_str().unwrap_or("?")),
        ),
    ]
}

/// Rows of the `the_balp_audit_event_carries_each_row_where_the_mapping_says` table.
fn balp_authorisation_rows(f: &Balp) -> Vec<Row> {
    let read = &f.read;
    let relayed = &f.relayed;
    let urgent = &f.urgent;
    let doc = &read.document;
    vec![
        (
            "purposes: purposeOfEvent and agent:user purposeOfUse",
            doc["purposeOfEvent"][0]["coding"][0]["code"] == "ETREAT"
                && user(doc)["purposeOfUse"][0]["coding"][0]["code"] == "ETREAT",
        ),
        (
            "authorisations, permit: a policy of agent:user after the token id",
            user(doc)["policy"][1] == "permit-42",
        ),
        (
            "authorisations, agreement: details of ehds-relayed",
            detail(
                &relayed.document,
                "ehds-relayed",
                "agreement-name",
                "Regional exchange",
            ),
        ),
        (
            "is_emergency: the ehds-emergency-access entity naming its purpose",
            named(&urgent.document, "ehds-emergency-access")
                .is_some_and(|e| e.to_string().contains("ETREAT")),
        ),
        (
            "channel.request_id: entity of type X-Request-Id",
            entities(doc).iter().any(|e| {
                e["type"]["code"] == "XrequestId"
                    && e["what"]["identifier"]["value"] == "01JC2Q7N3X"
            }),
        ),
        (
            "channel.client_address: the network address of the client's agent",
            doc["agent"][0]["network"]["address"] == "10.216.24.150",
        ),
        (
            "system_id: source.observer.identifier, the site from the deployment",
            doc["source"]["observer"]["identifier"]["value"] == "cdr.example.org"
                && doc["source"]["site"] == "site-1",
        ),
        (
            "security_labels: on the entity they label, never meta.security",
            doc["meta"].get("security").is_none(),
        ),
        (
            "retention: details of ehds-categories",
            named(doc, "ehds-categories").is_some_and(|e| e.to_string().contains("retention")),
        ),
        (
            "link, loss: the chain does not travel",
            !doc.to_string().contains("content_digest"),
        ),
        (
            "other_details, loss: FerroEHR's access details are written as their values only",
            !doc.to_string().contains("ferroehr_access_details"),
        ),
    ]
}

/// The projections the DICOM rows read.
struct Dicom {
    read: Projected,
    refused: Projected,
    query: Projected,
    represented: Projected,
    urgent: Projected,
    exported: Projected,
}

/// The rows of the mapping to the DICOM PS3.15 A.5 audit message.
#[test]
fn the_dicom_message_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let mut plain = classified_read(&policy);
    plain.actor.professional_id = None;
    plain.actor.mode = None;
    let read = project(&policy, &plain, Some("subject-1"));
    let (refused, _) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
    let represented = {
        let mut event = plain.clone();
        event.binding = Some(representative());
        project(&policy, &event, Some("subject-9"))
    };
    let urgent = emergency(&policy);
    let exported = export(&policy);
    let f = Dicom {
        read,
        refused,
        query,
        represented,
        urgent,
        exported,
    };
    let rows = [
        dicom_event_rows(&f),
        dicom_accessor_rows(&f),
        dicom_object_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{}", f.read.message);
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_event_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let exported = &f.exported;
    let xml = &read.message;
    vec![
        (
            "action: EventActionCode and the Patient Record EventID",
            xml.contains(r#"EventActionCode="R""#) && xml.contains(r#"csd-code="110110""#),
        ),
        (
            "action, loss: export folds into R, under the Export EventID",
            exported.message.contains(r#"EventActionCode="R""#)
                && exported.message.contains(r#"csd-code="110106""#),
        ),
        (
            "operation: EventTypeCode in openEHR-ITS-REST",
            xml.contains(r#"csd-code="composition_get" codeSystemName="openEHR-ITS-REST""#),
        ),
        (
            "time_recorded: EventDateTime",
            xml.contains(r#"EventDateTime="2026-11-13T10:15:30.123456Z""#),
        ),
        (
            "outcome: EventOutcomeIndicator and EventOutcomeDescription",
            xml.contains(r#"EventOutcomeIndicator="0""#)
                && refused.message.contains(r#"EventOutcomeIndicator="4""#)
                && xml.contains("<EventOutcomeDescription>"),
        ),
        (
            "accessor.credential: the requestor's UserID is the token subject",
            xml.contains(r#"UserID="alice" UserIsRequestor="true""#),
        ),
        (
            "accessor.credential, loss: the issuer and the token id are not written",
            !xml.contains("login.example.org") && !xml.contains("jti-7f3e"),
        ),
    ]
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_accessor_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let xml = &read.message;
    vec![
        (
            "accessor.roles: RoleIDCode in the deployment's role vocabulary",
            xml.contains(r#"csd-code="clinician" codeSystemName="urn:ferroehr:role""#),
        ),
        (
            "accessor.party and binding: details of the ehds-accessor object",
            xml.contains("<ParticipantObjectName>ehds-accessor</ParticipantObjectName>")
                && xml.contains(r#"ParticipantObjectDetail type="party""#),
        ),
        (
            "accessor.delegation: a detail of the ehds-accessor object",
            represented.message.contains("ehds-accessor")
                && represented.message.contains(r#"type="actor""#),
        ),
        (
            "system_id: AuditSourceID, the site from the deployment",
            xml.contains(r#"AuditEnterpriseSiteID="site-1" AuditSourceID="cdr.example.org""#),
        ),
        (
            "accessor.provider, loss: the organisation is not written",
            !xml.contains("Example Hospital"),
        ),
        (
            "channel.client_address: NetworkAccessPointID of type 2",
            xml.contains(r#"NetworkAccessPointID="10.216.24.150" NetworkAccessPointTypeCode="2""#),
        ),
    ]
}

/// Rows of the `the_dicom_message_carries_each_row_where_the_mapping_says` table.
fn dicom_object_rows(f: &Dicom) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let urgent = &f.urgent;
    let xml = &read.message;
    vec![
        (
            "ehr_id: the patient participant names the subject of care",
            xml.contains(
                r#"ParticipantObjectID="subject-1" ParticipantObjectTypeCode="1" ParticipantObjectTypeCodeRole="1""#,
            ),
        ),
        (
            "ehr_id, loss: the ehr_id itself is not written",
            !xml.contains(EHR),
        ),
        (
            "items: a System Object of ID type URI naming the version",
            xml.contains(&format!(
                r#"ParticipantObjectID="{VERSION}" ParticipantObjectTypeCode="2""#
            )),
        ),
        (
            "items.categories, loss: no category is written",
            !xml.contains("Laboratory-Reports"),
        ),
        (
            "query: the query object of the Query EventID",
            query.message.contains(r#"csd-code="110112""#)
                && query.message.contains("org.example::vitals"),
        ),
        (
            "purposes, loss: no purpose of use is written",
            !xml.contains("ETREAT") && !xml.contains("TREAT\""),
        ),
        (
            "authorisations, loss: no permit is written",
            !xml.contains("permit-42"),
        ),
        (
            "is_emergency, loss: the mark is not written",
            !urgent.message.contains("emergency"),
        ),
        (
            "retention, security_labels, link and other_details, loss: none is written",
            !xml.contains("retention")
                && !xml.contains("ParticipantObjectSensitivity")
                && !xml.contains("ferroehr_access_details"),
        ),
    ]
}

/// The projections the EHDS rows read.
struct Ehds {
    read: Projected,
    refused: Projected,
    failed: Projected,
    query: Projected,
    exported: Projected,
    urgent: Projected,
    represented: Projected,
    relayed: Projected,
}

/// The rows of the mapping to the EHDS access record (`ehds-logging`
/// `AccessRecord`).
#[test]
fn the_ehds_access_record_carries_each_row_where_the_mapping_says() {
    let policy = policy();
    let read = project(&policy, &classified_read(&policy), Some("subject-1"));
    let (refused, failed) = outcomes(&policy);
    let query = project(&policy, &stored_query(&policy), Some("subject-1"));
    let exported = export(&policy);
    let urgent = emergency(&policy);
    let represented = {
        let mut event = classified_read(&policy);
        event.binding = Some(representative());
        project(&policy, &event, Some("subject-9"))
    };
    let relayed = {
        let mut event = classified_read(&policy);
        event.binding = Some(organisation());
        project(&policy, &event, Some("subject-1"))
    };
    let f = Ehds {
        read,
        refused,
        failed,
        query,
        exported,
        urgent,
        represented,
        relayed,
    };
    let rows = [
        ehds_event_rows(&f),
        ehds_accessor_rows(&f),
        ehds_object_rows(&f),
    ]
    .concat();
    assert_eq!(failing(&rows), Vec::<&str>::new(), "{:?}", f.read.record);
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_event_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let refused = &f.refused;
    let failed = &f.failed;
    let query = &f.query;
    let exported = &f.exported;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        ("action: Read", record.action == Action::Read),
        (
            "action: Query",
            query.record.as_ref().map(|r| r.action) == Some(Action::Query),
        ),
        (
            "action, loss: export folds into Read",
            exported.record.as_ref().map(|r| r.action) == Some(Action::Read),
        ),
        (
            "time_recorded: recorded",
            record.recorded.to_string() == "2026-11-13T10:15:30.123456Z",
        ),
        (
            "outcome: granted is Success",
            record.outcome == Outcome::Success,
        ),
        (
            "outcome, loss: refused shares MinorFailure with an admitted failure",
            refused.record.as_ref().map(|r| r.outcome) == Some(Outcome::MinorFailure),
        ),
        (
            "outcome: failed is the failure's own grade",
            failed.record.as_ref().map(|r| r.outcome) == Some(Outcome::SeriousFailure),
        ),
    ]
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_accessor_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let represented = &f.represented;
    let relayed = &f.relayed;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        (
            "accessor.credential: issuer, subject, client and token id",
            record.accessor.issuer.as_deref() == Some("https://login.example.org")
                && record.accessor.subject == "alice"
                && record.accessor.client_id.as_deref() == Some("ward-app")
                && record.accessor.token_id.as_deref() == Some("jti-7f3e"),
        ),
        (
            "accessor.kind: acting",
            record.accessor.acting == Some(Acting::Client),
        ),
        (
            "accessor.roles: roles",
            record.accessor.roles.iter().any(|r| r.code == "clinician"),
        ),
        (
            "accessor.credential.assurance: assurance",
            record.accessor.assurance.is_some(),
        ),
        (
            "accessor.party, groups, binding: the binding",
            record
                .accessor
                .party()
                .is_some_and(|party| party.id == "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5"),
        ),
        (
            "accessor.provider: provider",
            record.accessor.provider.as_deref() == Some("Example Hospital"),
        ),
        (
            "accessor.delegation: actors",
            represented
                .record
                .as_ref()
                .is_some_and(|r| !r.accessor.actors.is_empty()),
        ),
        (
            "authorisations, representation: represents",
            represented
                .record
                .as_ref()
                .is_some_and(|r| r.accessor.represents.is_some()),
        ),
        (
            "authorisations, agreement: relayed",
            relayed
                .record
                .as_ref()
                .is_some_and(|r| r.accessor.relayed.is_some()),
        ),
        (
            "purposes and permit: purposes and policies",
            record.accessor.purposes.iter().any(|p| p.code == "ETREAT")
                && record.accessor.policies == ["permit-42"],
        ),
    ]
}

/// Rows of the `the_ehds_access_record_carries_each_row_where_the_mapping_says` table.
fn ehds_object_rows(f: &Ehds) -> Vec<Row> {
    let read = &f.read;
    let query = &f.query;
    let urgent = &f.urgent;
    let record = read
        .record
        .as_ref()
        .expect("a classified access is a record");
    vec![
        (
            "ehr_id: one EhrAt, the subject of care found",
            record.subject.ehrs.len() == 1
                && record.subject.ehrs[0].ehr_id == EHR
                && matches!(record.subject.ehrs[0].patient, PatientLookup::Found(_)),
        ),
        (
            "items.categories: categories",
            record
                .categories
                .categories()
                .keys()
                .any(|c| c.code() == "Laboratory-Reports"),
        ),
        (
            "outcome.delivered and items.origins: delivered, origins, origin_count",
            record.delivered == Some(1)
                && record.origins.len() == 2
                && record.origin_count == Some(3),
        ),
        (
            "channel and operation: request id, operation, resource, client address",
            record.request.id.as_deref() == Some("01JC2Q7N3X")
                && record.request.operation == "composition_get"
                && record.request.resource.as_deref() == Some(VERSION)
                && record
                    .request
                    .client_address
                    .map(|a| a.to_string())
                    .as_deref()
                    == Some("10.216.24.150"),
        ),
        (
            "query, loss: the text is not carried, the name stands in",
            query.record.as_ref().is_some_and(|r| {
                r.request
                    .query
                    .as_ref()
                    .is_none_or(|text| !text.expose_secret().contains("SELECT"))
            }),
        ),
        (
            "retention: the period the entry states",
            record.retention.period().code() == "indefinite",
        ),
        (
            "is_emergency: emergency",
            urgent
                .record
                .as_ref()
                .is_some_and(|r| r.emergency.is_some()),
        ),
        (
            "link, loss: the record carries no chain",
            !format!("{record:?}").contains("content_digest"),
        ),
    ]
}
