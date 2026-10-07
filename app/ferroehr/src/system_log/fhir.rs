// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FHIR R4 `AuditEvent` rendering of the audit event model, following the
//! IHE **BALP** (Basic Audit Log Patterns) content profiles.
//!
//! This is the modern half of the dual ATNA rendering, the classic half being
//! the DICOM PS3.15 §A.5 XML in [`super::message`]: the same resolved
//! [`super::event::AuditEvent`] renders to one FHIR R4 `AuditEvent` JSON
//! document. This module decides the BALP content and
//! [`ferroehr_ext::fhir::audit`] builds and serializes the resource. The BALP
//! profiles pin the codings emitted here (IHE BALP v1.1.4, `IHE.BasicAudit.*`
//! StructureDefinitions):
//!
//! - `RESTful` operations: `type` = `rest`, `subtype` = the FHIR
//!   restful-interaction class + the concrete ITS-REST operation id.
//! - Role direction per BALP: on a **Read** the server is the *source* of
//!   the data (DCM 110153) and the client the *destination* (DCM 110152);
//!   on every other action the client initiates as source
//!   (`IHE.BasicAudit.PatientRead` vs `IHE.BasicAudit.PatientQuery`
//!   agent:client/agent:server fixed codings).
//! - Patient entity: `audit-entity-type` code `1` (Person) +
//!   `object-role` code `1` (Patient), the resolved EHR subject as an
//!   identifier-only reference.
//! - Query entity: `audit-entity-type` `2` + `object-role` `24`, the search
//!   expression base64-encoded in `entity.query`
//!   (`IHE.BasicAudit.Query`/`PatientQuery`).
//! - Bearer token identity: an `agent` typed
//!   `UserAgentTypes#UserOauthAgent` carrying the token `jti` in
//!   `agent.policy` (`IHE.BasicAudit.OAUTHaccessTokenUse.Minimal` — minimal
//!   by design: never the token itself).
//!
//! `meta.profile` claims the matching BALP profile only when the record
//! actually satisfies it (the BALP `RESTful` profiles fix `outcome` = `0`, so
//! failures carry no claim; the `Patient*` variants additionally require the
//! resolved patient entity).

use ferroehr_ext::fhir::audit::{
    AuditAction, AuditAgent, AuditCoding, AuditEntityRef, AuditOutcome, AuditRecord,
    AuditSourceRef, AuditWho,
};

use crate::system_log::AuditError;
use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass};
use crate::system_log::message::AuditContext;

// ── Code systems (FHIR R4 / IHE BALP fixed bindings) ──────────────────────────

/// FHIR `audit-event-type` code system (`rest`).
pub const SYS_AUDIT_EVENT_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-event-type";
/// FHIR restful-interaction code system (`create`/`read`/…).
pub const SYS_RESTFUL_INTERACTION: &str = "http://hl7.org/fhir/restful-interaction";
/// DICOM controlled terminology (DCM) system URI.
pub const SYS_DCM: &str = "http://dicom.nema.org/resources/ontology/DCM";
/// FHIR `audit-entity-type` code system (`1` person / `2` system object).
pub const SYS_AUDIT_ENTITY_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-entity-type";
/// FHIR `object-role` code system (`1` patient / `24` query).
pub const SYS_OBJECT_ROLE: &str = "http://terminology.hl7.org/CodeSystem/object-role";
/// FHIR `security-source-type` code system (`4` application server).
pub const SYS_SECURITY_SOURCE_TYPE: &str =
    "http://terminology.hl7.org/CodeSystem/security-source-type";
/// IHE BALP `UserAgentTypes` code system (`UserOauthAgent`).
pub const SYS_BALP_USER_AGENT_TYPES: &str =
    "https://profiles.ihe.net/ITI/BALP/CodeSystem/UserAgentTypes";
/// The system for ITS-REST operation-id subtype codings.
///
/// NOTE: no external code system governs openEHR REST operations — our own
/// design/extension (the same ids the DICOM rendering emits under the
/// `openEHR-ITS-REST` `codeSystemName`).
pub const SYS_OPENEHR_ITS_REST: &str = "urn:openehr:its-rest:operation";

/// The canonical-URL prefix of the IHE BALP profiles (v1.1.4).
pub const BALP_PROFILE_BASE: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition";

/// A BALP coding with display text.
fn coding(system: &str, code: &str, display: &str) -> AuditCoding {
    AuditCoding {
        system: Some(system.to_owned()),
        code: code.to_owned(),
        display: Some(display.to_owned()),
    }
}

/// Renders a resolved [`AuditEvent`] as a FHIR R4 `AuditEvent` document.
///
/// The content follows the IHE BALP patterns, with the server identity from
/// the [`AuditContext`] and the optionally-resolved patient subject id.
///
/// # Errors
///
/// [`AuditError::Render`] when the typed FHIR resource cannot be built or
/// serialized (an unrepresentable instant, or a serializer fault).
pub fn to_fhir(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Result<serde_json::Value, AuditError> {
    let missing = ctx.value_if_missing.as_str();

    let (event_type, interaction) = type_and_interaction(event);

    let mut subtypes = Vec::new();
    if let Some(i) = interaction {
        subtypes.push(i);
    }
    match event.event_type {
        Some(EventType::Login) => subtypes.push(coding(SYS_DCM, "110122", "Login")),
        Some(EventType::Logout) => subtypes.push(coding(SYS_DCM, "110123", "Logout")),
        Some(EventType::RestOperation(op)) => {
            subtypes.push(coding(SYS_OPENEHR_ITS_REST, op, op));
        }
        None => {}
    }

    let record = AuditRecord {
        profiles: claimed_profiles(event, subject),
        event_type,
        subtypes,
        action: action_code(event.action),
        recorded: event.timestamp,
        outcome: outcome_code(event.outcome),
        outcome_desc: match event.outcome {
            EventOutcome::Success => None,
            _ => Some("Operation failed".to_owned()),
        },
        agents: build_agents(event, ctx, missing),
        source: AuditSourceRef {
            site: nonempty_opt(&ctx.enterprise_site_id),
            observer: nonempty(&ctx.source_id, missing),
            types: vec![coding(SYS_SECURITY_SOURCE_TYPE, "4", "Application Server")],
        },
        entities: build_entities(event, subject, missing),
    };

    ferroehr_ext::fhir::audit::render(&record).map_err(|e| AuditError::Render(Box::new(e)))
}

/// The `type` coding + the restful-interaction `subtype` for the event.
/// REST-dispatched resource operations are `rest`; the service-level events
/// keep their DICOM event id as the `type` (extract export/import, user
/// authentication) — the BALP `RESTful` patterns do not cover them.
fn type_and_interaction(event: &AuditEvent) -> (AuditCoding, Option<AuditCoding>) {
    match event.object {
        ObjectClass::Authentication => (coding(SYS_DCM, "110114", "User Authentication"), None),
        ObjectClass::Extract => match event.action {
            EventActionCode::Create | EventActionCode::Update => {
                (coding(SYS_DCM, "110107", "Import"), None)
            }
            _ => (coding(SYS_DCM, "110106", "Export"), None),
        },
        ObjectClass::Query => (
            coding(SYS_AUDIT_EVENT_TYPE, "rest", "RESTful Operation"),
            Some(coding(
                SYS_RESTFUL_INTERACTION,
                "search-type",
                "search-type",
            )),
        ),
        _ => (
            coding(SYS_AUDIT_EVENT_TYPE, "rest", "RESTful Operation"),
            Some(match event.action {
                EventActionCode::Create => coding(SYS_RESTFUL_INTERACTION, "create", "create"),
                EventActionCode::Read => coding(SYS_RESTFUL_INTERACTION, "read", "read"),
                EventActionCode::Update => coding(SYS_RESTFUL_INTERACTION, "update", "update"),
                EventActionCode::Delete => coding(SYS_RESTFUL_INTERACTION, "delete", "delete"),
                EventActionCode::Execute => {
                    coding(SYS_RESTFUL_INTERACTION, "operation", "operation")
                }
            }),
        ),
    }
}

/// The client/server agents (+ the OAuth token agent when a `jti` is known).
///
/// Role direction per the BALP fixed codings: on a **Read** the server is the
/// source of the data (DCM 110153 "Source Role ID") and the client the
/// destination (110152); on every other action the initiating client is the
/// source (`IHE.BasicAudit.PatientRead` vs `PatientQuery` agent slices).
fn build_agents(event: &AuditEvent, ctx: &AuditContext, missing: &str) -> Vec<AuditAgent> {
    let read = matches!(event.action, EventActionCode::Read);
    let (client_role, server_role) = if read {
        (
            coding(SYS_DCM, "110152", "Destination Role ID"),
            coding(SYS_DCM, "110153", "Source Role ID"),
        )
    } else {
        (
            coding(SYS_DCM, "110153", "Source Role ID"),
            coding(SYS_DCM, "110152", "Destination Role ID"),
        )
    };

    let user = nonempty(&event.user_id, missing);
    let client_ip = event
        .client_ip
        .clone()
        .unwrap_or_else(|| missing.to_owned());

    let mut agents = vec![
        // The requesting client, carrying the authenticated user identity
        // (the FHIR twin of the DICOM source ActiveParticipant) and the
        // purpose it declared: FHIR R4 puts `purposeOfUse` on the agent
        // (<https://hl7.org/fhir/R4/auditevent.html>), and the requestor is
        // the agent whose purpose it is.
        AuditAgent {
            role: Some(client_role),
            // The roles the person held, as `agent.role` (0..*): "the
            // security role that the user was acting under"
            // (<https://hl7.org/fhir/R4/auditevent-definitions.html#AuditEvent.agent.role>),
            // the NEN 7513 role element. The deployment's own vocabulary, so
            // text without a code system.
            roles: event.roles.clone(),
            who: Some(AuditWho::Identifier(user)),
            requestor: true,
            policy: Vec::new(),
            network_address: Some(client_ip),
            purpose_of_use: purpose_codings(event),
        },
        // This server.
        AuditAgent {
            role: Some(server_role),
            roles: Vec::new(),
            who: Some(AuditWho::Identifier(nonempty(&ctx.source_id, missing))),
            requestor: false,
            policy: Vec::new(),
            network_address: Some(nonempty(&ctx.server_ip, missing)),
            purpose_of_use: Vec::new(),
        },
    ];

    if let Some(organisation) = event.organisation.as_deref().filter(|s| !s.is_empty()) {
        // The organisation the caller acted for, as a second participant:
        // FHIR R4 `agent.who` may reference an `Organization` and
        // `agent.type` is 0..1, so the organisation needs no participation
        // code and none is invented for it
        // (<https://hl7.org/fhir/R4/auditevent.html>). It did not initiate the
        // request — the natural person did — so it is not the requestor.
        agents.push(AuditAgent {
            role: None,
            roles: Vec::new(),
            who: Some(AuditWho::Reference(format!("Organization/{organisation}"))),
            requestor: false,
            policy: Vec::new(),
            network_address: None,
            purpose_of_use: Vec::new(),
        });
    }

    if let Some(jti) = event.token_id.as_deref().filter(|s| !s.is_empty()) {
        // IHE.BasicAudit.OAUTHaccessTokenUse.Minimal: the token identity is
        // ONLY the jti, carried in agent.policy — never the token contents.
        agents.push(AuditAgent {
            role: Some(coding(
                SYS_BALP_USER_AGENT_TYPES,
                "UserOauthAgent",
                "User OAuth Agent participant",
            )),
            roles: Vec::new(),
            who: None,
            requestor: true,
            policy: vec![jti.to_owned()],
            network_address: None,
            purpose_of_use: Vec::new(),
        });
    }

    agents
}

/// The declared purpose of use as `agent.purposeOfUse` codings.
///
/// The code comes from the vocabulary the deployment agrees with its callers
/// (`[audit] purpose_codes`), which has no published code system, so the
/// coding carries the code alone: FHIR R4 makes `Coding.system` optional
/// (<https://hl7.org/fhir/R4/datatypes.html>) and naming a system the code
/// does not come from would misdescribe it.
fn purpose_codings(event: &AuditEvent) -> Vec<AuditCoding> {
    event
        .purpose
        .iter()
        .filter(|code| !code.is_empty())
        .map(|code| AuditCoding {
            system: None,
            code: code.clone(),
            display: None,
        })
        .collect()
}

/// The entity list: the patient (when resolved), the touched data object
/// and/or the query expression, the origins of the served data, and the
/// categories of the data accessed.
fn build_entities(event: &AuditEvent, subject: Option<&str>, missing: &str) -> Vec<AuditEntityRef> {
    use crate::system_log::codes::AtnaObject;

    let mut entities = Vec::new();

    if event.object.is_query() {
        // NOTE: the audit layer carries the qualified stored-query name, not
        // ad-hoc AQL text; it is base64-encoded per the BALP query pattern
        // ("base64 encoding preserves exactly what was requested").
        let expression = event
            .object_id
            .clone()
            .unwrap_or_else(|| missing.to_owned());
        entities.push(AuditEntityRef {
            what: None,
            entity_type: Some(coding(SYS_AUDIT_ENTITY_TYPE, "2", "System Object")),
            role: Some(coding(SYS_OBJECT_ROLE, "24", "Query")),
            name: None,
            query: Some(expression),
            details: Vec::new(),
        });
        entities.extend(origin_entities(event));
        entities.extend(category_entity(event));
        return entities;
    }

    if event.object.is_patient_centric()
        && let Some(subject) = subject
    {
        entities.push(AuditEntityRef {
            what: Some(subject.to_owned()),
            entity_type: Some(coding(SYS_AUDIT_ENTITY_TYPE, "1", "Person")),
            role: Some(coding(SYS_OBJECT_ROLE, "1", "Patient")),
            name: None,
            query: None,
            details: Vec::new(),
        });
    }

    if let Some(id) = event.object_id.as_deref().filter(|s| !s.is_empty()) {
        entities.push(AuditEntityRef {
            what: Some(id.to_owned()),
            entity_type: Some(coding(SYS_AUDIT_ENTITY_TYPE, "2", "System Object")),
            role: None,
            name: None,
            query: None,
            details: Vec::new(),
        });
    }

    entities.extend(origin_entities(event));
    entities.extend(category_entity(event));
    entities
}

/// The origins of the served data (EHDS Annex II 3.2(e), #3212), one entity
/// per originating system, for a document read and a query alike.
///
/// Each entity is named so a reader can tell it from the object entity; the
/// first carries the true distinct count when the recorded set was capped.
fn origin_entities(event: &AuditEvent) -> impl Iterator<Item = AuditEntityRef> + '_ {
    let recorded = event.origins.len();
    let capped = event
        .origin_count
        .filter(|count| *count > u64::try_from(recorded).unwrap_or(u64::MAX));
    event
        .origins
        .iter()
        .enumerate()
        .map(move |(index, origin)| {
            let name = match (index, capped) {
                (0, Some(total)) => format!(
                    "origin of the served data ({total} distinct, first {recorded} recorded)"
                ),
                _ => "origin of the served data".to_owned(),
            };
            AuditEntityRef {
                what: Some(origin.clone()),
                entity_type: Some(coding(SYS_AUDIT_ENTITY_TYPE, "2", "System Object")),
                role: None,
                name: Some(name),
                query: None,
                details: Vec::new(),
            }
        })
}

/// The categories of the data accessed (EHDS Annex II 3.2(c), #3621), as one
/// named entity whose details carry each category, the basis, each piece of
/// evidence and the digest of the map that classified the record.
///
/// FHIR R4 `AuditEvent.entity.detail` is a `(type, value[x])` pair
/// (<https://hl7.org/fhir/R4/auditevent.html>); the detail type names are our
/// own. `None` when the record carries no classification.
fn category_entity(event: &AuditEvent) -> Option<AuditEntityRef> {
    let record = event.category.as_ref()?;
    let mut details: Vec<(String, String)> = record
        .categories
        .iter()
        .map(|category| ("category".to_owned(), category.clone()))
        .collect();
    details.push((
        "category-basis".to_owned(),
        record.basis.as_str().to_owned(),
    ));
    details.extend(
        record
            .evidence
            .iter()
            .map(|id| ("category-evidence".to_owned(), id.clone())),
    );
    if let Some(digest) = event.category_map_digest {
        details.push(("category-map".to_owned(), digest.to_string()));
    }
    Some(AuditEntityRef {
        what: None,
        entity_type: Some(coding(SYS_AUDIT_ENTITY_TYPE, "2", "System Object")),
        role: None,
        name: Some("categories of the data accessed".to_owned()),
        query: None,
        details,
    })
}

/// The BALP profile(s) this record actually satisfies. The `RESTful` profiles
/// fix `outcome = 0`, so failures claim nothing; the `Patient*` variants
/// require the resolved patient entity.
fn claimed_profiles(event: &AuditEvent, subject: Option<&str>) -> Vec<String> {
    use crate::system_log::codes::AtnaObject;

    let mut profiles = Vec::new();
    if event.outcome == EventOutcome::Success {
        let patient = event.object.is_patient_centric() && subject.is_some();
        if let Some(name) = balp_profile_name(event.object, event.action, patient) {
            profiles.push(format!("{BALP_PROFILE_BASE}/IHE.BasicAudit.{name}"));
        }
    }
    if event.token_id.as_deref().is_some_and(|s| !s.is_empty()) {
        profiles.push(format!(
            "{BALP_PROFILE_BASE}/IHE.BasicAudit.OAUTHaccessTokenUse.Minimal"
        ));
    }
    profiles
}

/// The BALP basic-audit profile name one successful record claims, or `None`
/// where the profile family defines none for that action.
///
/// `patient` selects the `Patient*` variant, which requires a resolved patient
/// entity.
const fn balp_profile_name(
    object: ObjectClass,
    action: EventActionCode,
    patient: bool,
) -> Option<&'static str> {
    let (plain, patient_variant) = match (object, action) {
        (ObjectClass::Authentication | ObjectClass::Extract, _) => return None,
        (ObjectClass::Query, _) => ("Query", "PatientQuery"),
        (_, EventActionCode::Create) => ("Create", "PatientCreate"),
        (_, EventActionCode::Read) => ("Read", "PatientRead"),
        (_, EventActionCode::Update) => ("Update", "PatientUpdate"),
        (_, EventActionCode::Delete) => ("Delete", "PatientDelete"),
        (_, EventActionCode::Execute) => return None,
    };
    Some(if patient { patient_variant } else { plain })
}

/// The FHIR action code for an audited action.
const fn action_code(action: EventActionCode) -> AuditAction {
    match action {
        EventActionCode::Create => AuditAction::Create,
        EventActionCode::Read => AuditAction::Read,
        EventActionCode::Update => AuditAction::Update,
        EventActionCode::Delete => AuditAction::Delete,
        EventActionCode::Execute => AuditAction::Execute,
    }
}

/// The FHIR outcome indicator for an audited outcome.
const fn outcome_code(outcome: EventOutcome) -> AuditOutcome {
    match outcome {
        EventOutcome::Success => AuditOutcome::Success,
        EventOutcome::MinorFailure => AuditOutcome::MinorFailure,
        EventOutcome::SeriousFailure => AuditOutcome::SeriousFailure,
        EventOutcome::MajorFailure => AuditOutcome::MajorFailure,
    }
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn nonempty_opt(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use jiff::Timestamp;

    fn ctx() -> AuditContext {
        AuditContext {
            source_id: "ferroehr".to_owned(),
            enterprise_site_id: "1f332a66-0000-0000-0000-000000000001".to_owned(),
            server_ip: "10.42.23.77".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
        }
    }

    fn event(action: EventActionCode, object: ObjectClass, outcome: EventOutcome) -> AuditEvent {
        let mut e = AuditEvent::new(action, object, outcome);
        e.user_id = "john doe".to_owned();
        e.client_ip = Some("10.216.24.150".to_owned());
        e.timestamp = "2026-07-06T12:00:00Z".parse::<Timestamp>().unwrap();
        e
    }

    fn json(event: &AuditEvent, subject: Option<&str>) -> serde_json::Value {
        to_fhir(event, &ctx(), subject).expect("render")
    }

    #[test]
    fn golden_patient_composition_read() {
        // A successful composition read with a resolved subject claims the
        // BALP PatientRead profile; on a Read the server is the data source
        // (DCM 110153) and the client the destination (110152).
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::1".to_owned());
        e.event_type = Some(EventType::RestOperation("composition_get"));
        let v = json(&e, Some("patient-42"));
        insta::assert_json_snapshot!("fhir_patient_composition_read", v);
    }

    #[test]
    fn patient_read_claims_patient_profile_and_roles() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::1".to_owned());
        let v = json(&e, Some("patient-42"));
        assert_eq!(
            v["meta"]["profile"][0],
            "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead"
        );
        // Read: client = destination (110152), server = source (110153).
        assert_eq!(v["agent"][0]["type"]["coding"][0]["code"], "110152");
        assert_eq!(v["agent"][1]["type"]["coding"][0]["code"], "110153");
        // The patient entity: person(1)/patient(1) with the subject id.
        assert_eq!(v["entity"][0]["type"]["code"], "1");
        assert_eq!(v["entity"][0]["role"]["code"], "1");
        assert_eq!(v["entity"][0]["what"]["identifier"]["value"], "patient-42");
        // The data entity: the composition uid.
        assert_eq!(
            v["entity"][1]["what"]["identifier"]["value"],
            "8fa1::ferroehr::1"
        );
    }

    #[test]
    fn create_uses_source_role_for_client() {
        let e = event(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        let v = json(&e, Some("patient-42"));
        // Non-read: client initiates as source (110153).
        assert_eq!(v["agent"][0]["type"]["coding"][0]["code"], "110153");
        assert_eq!(v["agent"][1]["type"]["coding"][0]["code"], "110152");
        assert_eq!(v["subtype"][0]["code"], "create");
        assert_eq!(
            v["meta"]["profile"][0],
            "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientCreate"
        );
    }

    #[test]
    fn unresolved_subject_falls_back_to_non_patient_profile() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        let v = json(&e, None);
        assert_eq!(
            v["meta"]["profile"][0],
            "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read"
        );
        // No patient entity without a resolved subject.
        assert!(v["entity"].as_array().is_none_or(Vec::is_empty));
    }

    #[test]
    fn failure_claims_no_profile() {
        // The BALP RESTful profiles fix outcome = 0; a failure record keeps
        // the shape but claims nothing.
        let e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::MinorFailure,
        );
        let v = json(&e, Some("patient-42"));
        assert!(v.get("meta").is_none());
        assert_eq!(v["outcome"], "4");
        assert_eq!(v["outcomeDesc"], "Operation failed");
    }

    #[test]
    fn query_entity_is_base64_search_criteria() {
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Query,
            EventOutcome::Success,
        );
        e.object_id = Some("eu.ferroehr::q1".to_owned());
        let v = json(&e, None);
        assert_eq!(v["type"]["code"], "rest");
        assert_eq!(v["subtype"][0]["code"], "search-type");
        assert_eq!(v["entity"][0]["role"]["code"], "24");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(v["entity"][0]["query"].as_str().expect("query"))
            .expect("base64");
        assert_eq!(decoded, b"eu.ferroehr::q1");
        assert_eq!(
            v["meta"]["profile"][0],
            "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Query"
        );
    }

    /// A query record renders one origin entity per recorded origin, in the
    /// shape a document read uses, with the capped-count marker (#3654).
    #[test]
    fn query_record_carries_the_origins_of_the_served_data() {
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Query,
            EventOutcome::Success,
        );
        e.object_id = Some("eu.ferroehr::q1".to_owned());
        e.record_origins(vec!["system-a".to_owned(), "system-b".to_owned()], Some(7));
        let v = json(&e, None);
        let entities = v["entity"].as_array().expect("entities");
        let origins: Vec<&serde_json::Value> = entities
            .iter()
            .filter(|entity| {
                entity["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("origin of the served data"))
            })
            .collect();
        assert_eq!(origins.len(), 2, "{v}");
        let mut served: Vec<&str> = origins
            .iter()
            .map(|entity| {
                entity["what"]["identifier"]["value"]
                    .as_str()
                    .expect("what")
            })
            .collect();
        served.sort_unstable();
        assert_eq!(served, ["system-a", "system-b"]);
        assert_eq!(
            origins[0]["name"],
            "origin of the served data (7 distinct, first 2 recorded)"
        );
        assert_eq!(origins[1]["name"], "origin of the served data");
        assert_eq!(origins[0]["type"]["code"], "2");
        // The query entity keeps the first position.
        assert_eq!(entities[0]["role"]["code"], "24");
    }

    #[test]
    fn bearer_token_adds_oauth_minimal_agent() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.token_id = Some("jti-abc-123".to_owned());
        let v = json(&e, Some("patient-42"));
        let agents = v["agent"].as_array().expect("agents");
        assert_eq!(agents.len(), 3);
        assert_eq!(agents[2]["type"]["coding"][0]["code"], "UserOauthAgent");
        assert_eq!(agents[2]["policy"][0], "jti-abc-123");
        assert_eq!(agents[2]["requestor"], true);
        // Both the activity profile and the token-use profile are claimed.
        let profiles = v["meta"]["profile"].as_array().expect("profiles");
        assert!(profiles.iter().any(|p| {
            p.as_str()
                .is_some_and(|s| s.ends_with("IHE.BasicAudit.OAUTHaccessTokenUse.Minimal"))
        }));
    }

    #[test]
    fn login_record_is_user_authentication() {
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Authentication,
            EventOutcome::Success,
        );
        e.event_type = Some(EventType::Login);
        let v = json(&e, None);
        assert_eq!(v["type"]["code"], "110114");
        assert_eq!(v["subtype"][0]["code"], "110122");
        assert!(v.get("meta").is_none());
        insta::assert_json_snapshot!("fhir_login_success", v);
    }

    #[test]
    fn extract_directions_use_export_import() {
        let out = event(
            EventActionCode::Read,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        assert_eq!(json(&out, None)["type"]["code"], "110106");
        let inbound = event(
            EventActionCode::Create,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        assert_eq!(json(&inbound, None)["type"]["code"], "110107");
    }

    #[test]
    fn subsecond_timestamps_keep_their_precision() {
        // The rendered `recorded` is the record's own instant, trailing
        // zeros trimmed — the FHIR `instant` form of the stored timestamp.
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.timestamp = "2026-07-06T12:00:00.123456789Z"
            .parse::<Timestamp>()
            .unwrap();
        assert_eq!(json(&e, None)["recorded"], "2026-07-06T12:00:00.123456789Z");
        e.timestamp = "2026-07-06T12:00:00.5Z".parse::<Timestamp>().unwrap();
        assert_eq!(json(&e, None)["recorded"], "2026-07-06T12:00:00.5Z");
    }
}
