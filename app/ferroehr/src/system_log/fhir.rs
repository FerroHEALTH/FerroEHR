// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR R4 `AuditEvent` of one audit record, written by the shared
//! FerroAUDIT crates on the IHE BALP 1.1.4 patterns.
//!
//! An access to clinical content the middleware classified is the EHDS access
//! record of Regulation (EU) 2025/327 Annex II 3.2: FerroEHR builds the
//! `ehds-logging` [`AccessRecord`] from it and `ehds-logging` writes the BALP
//! `AuditEvent` (the `RESTful` pattern of its action, the categories, the
//! retention, the security labels, the origins and the emergency mark). Every
//! other record (an authentication, a template or party operation, a failed
//! access, an extract) is an `ihe-atna` BALP [`Exchange`] FerroEHR fills
//! directly. Both are written by `ihe-atna`.
//!
//! FerroEHR decides what the record says: the action, the participants as the
//! authentication layer judged them, the patient its subject resolver found,
//! the objects and the origins its storage read. The shared crates decide how
//! it is written.

use std::net::IpAddr;

use ehds_logging::emergency::EmergencyPurposes;
use ehds_logging::label::LabelPolicy;
use ehds_logging::record::{
    AccessRecord, Accessor, Acting, Action, AssuranceLevel, DataSubject, EhrAt, Origin, Outcome,
    PatientIdentifier, PatientLookup, Professional, Purpose, Request,
};
use ehds_logging::retention::RetentionPolicy;
use ihe_atna::balp::{
    APPLICATION, CREATE, CUSTODIAN, Coded, DELETE, DESTINATION_ROLE, DOMAIN_RESOURCE, Described,
    Direction, EXPORT, Entity, EventKind, Exchange, IMPORT, LOGIN, LOGOUT, NetworkAddress,
    OPERATION, Observer, Peer, READ, REST, SEARCH, SOURCE_APPLICATION_SERVER, SOURCE_ROLE,
    SYSTEM_OBJECT, UPDATE, USER_AUTHENTICATION, What,
};
use ihe_atna::user::{Code, OnBehalfOf, PurposeOfUse, User};
use secrecy::SecretString;

use crate::system_log::AuditError;
use crate::system_log::codes::AtnaObject;
use crate::system_log::event::{
    ActingMode, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use crate::system_log::message::AuditContext;

/// The canonical-URL prefix of the IHE BALP profiles (v1.1.4).
pub const BALP_PROFILE_BASE: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition";

/// The code system of the ITS-REST operation ids a record writes as its
/// operation subtype.
// NOTE: no specification governs this: our own design; no external code system names the
// openEHR REST operations, and the DICOM rendering names them `openEHR-ITS-REST`.
pub const OPERATION_SYSTEM: &str = "urn:openehr:its-rest:operation";

/// The policies an access record is written under: how long the store keeps
/// it, the security labels of its categories and origins, and the purposes of
/// use that mark an emergency access.
#[derive(Debug, Clone, Default)]
pub struct Recording {
    /// The retention every record states (Art. 9(2), Annex II 3.4).
    pub retention: RetentionPolicy,
    /// The labels of each category and origin (Annex II 3.4).
    pub labels: LabelPolicy,
    /// The purposes of use that mark an emergency access (Art. 11(5)).
    pub emergency: EmergencyPurposes,
    /// The store horizon, which a record outside the EHDS access record is
    /// kept for.
    pub horizon: crate::system_log::config::Retention,
}

/// The first date `event` may be deleted on once stored.
///
/// A classified access states its own end through `ehds-logging`'s retention
/// policy, by its categories and origins (Regulation (EU) 2025/327 Annex II
/// 3.4); every other record keeps the store horizon, ending the day after
/// the date of access plus the horizon, as `ehds-logging` counts an end.
/// A horizon whose end the calendar cannot hold keeps the record without end.
#[must_use]
pub fn record_end(event: &AuditEvent, ctx: &AuditContext) -> crate::system_log::store::RecordEnd {
    use crate::system_log::store::RecordEnd;
    let ends = match &event.category {
        Some(category) => {
            let origins = origins(event);
            ctx.recording
                .retention
                .retain(
                    event.timestamp,
                    &category.classification,
                    origins.iter().filter_map(Origin::key),
                )
                .ends()
        }
        None => horizon_end(event.timestamp, ctx.recording.horizon),
    };
    ends.map_or(RecordEnd::Never, RecordEnd::On)
}

/// The day after the date of `recorded` plus `horizon`, in UTC; `None` for a
/// record kept forever or an end past the calendar.
fn horizon_end(
    recorded: jiff::Timestamp,
    horizon: crate::system_log::config::Retention,
) -> Option<jiff::civil::Date> {
    use crate::system_log::config::Retention;
    let accessed = recorded.to_zoned(jiff::tz::TimeZone::UTC).date();
    let span = match horizon {
        Retention::Forever => return None,
        Retention::Days(days) => jiff::Span::new().days(i64::from(days)),
        Retention::Years(years) => jiff::Span::new().years(i64::from(years)),
    };
    // NOTE: an end past the calendar's last day is legitimately absent, so the record
    // is kept without end rather than refused.
    accessed.checked_add(span).ok()?.tomorrow().ok()
}

/// Renders a resolved [`AuditEvent`] as a FHIR R4 `AuditEvent` document.
///
/// A classified access is written as the EHDS access record through
/// `ehds-logging`; every other record as a BALP exchange through `ihe-atna`.
/// `subject` is the patient the subject resolver found behind the record's
/// EHR, `(id, namespace)`.
///
/// # Errors
///
/// [`AuditError::Render`] when the endpoint the record names is not a URL or
/// the document cannot be written as JSON.
pub fn to_fhir(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Result<serde_json::Value, AuditError> {
    let endpoint = endpoint(ctx);
    let observer = Observer {
        source_id: nonempty(&ctx.source_id, &ctx.value_if_missing),
        site: (!ctx.enterprise_site_id.is_empty()).then(|| ctx.enterprise_site_id.clone()),
        host: NetworkAddress::host(&nonempty(&ctx.server_ip, &ctx.value_if_missing)),
        source_types: vec![SOURCE_APPLICATION_SERVER],
    };
    let exchange = match access_record(event, ctx, subject) {
        Some(record) => ehds_logging::balp::exchange(&record, &endpoint),
        None => exchange(event, ctx, subject, &endpoint),
    };
    exchange
        .audit_event_value(&observer)
        .map(ihe_atna::balp::AuditEventValue::into_value)
        .map_err(|e| AuditError::Render(Box::new(e)))
}

/// The address the record names FerroEHR's own agent by: its
/// `[audit] server_host`, a machine name or an IP address, else its source id
/// as a machine name (FHIR R4 `network-type`).
fn endpoint(ctx: &AuditContext) -> NetworkAddress {
    NetworkAddress::host(&nonempty(
        &ctx.server_ip,
        &nonempty(&ctx.source_id, &ctx.value_if_missing),
    ))
}

/// The text the record states an outcome in.
const fn outcome_description(outcome: EventOutcome) -> &'static str {
    match outcome {
        EventOutcome::Success => "Operation performed successfully",
        EventOutcome::MinorFailure | EventOutcome::SeriousFailure => "Operation failed",
    }
}

/// The EHDS access record of `event`, when it is a classified access.
fn access_record(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Option<AccessRecord> {
    let category = event.category.as_ref()?;
    let missing = ctx.value_if_missing.as_str();
    let action = match (event.object, event.action) {
        (ObjectClass::Query, _) | (_, EventActionCode::Execute) => Action::Query,
        (_, EventActionCode::Read) => Action::Read,
        (_, EventActionCode::Create) => Action::Create,
        (_, EventActionCode::Update) => Action::Update,
        (_, EventActionCode::Delete) => Action::Delete,
    };
    let holder = nonempty(&ctx.source_id, missing);
    let purposes: Vec<Purpose> = event
        .purpose
        .iter()
        .map(|code| Purpose {
            system: None,
            code: code.clone(),
        })
        .collect();
    let origins = origins(event);
    let ehrs = event
        .ehr_id
        .iter()
        .map(|ehr_id| EhrAt {
            endpoint: holder.clone(),
            ehr_id: ehr_id.clone(),
            patient: patient_lookup(event, subject),
        })
        .collect();
    let classification = category.classification.clone();
    let retention = ctx.recording.retention.retain(
        event.timestamp,
        &classification,
        origins.iter().filter_map(Origin::key),
    );
    let labels = ctx
        .recording
        .labels
        .label(&classification, origins.iter().filter_map(Origin::key));
    Some(AccessRecord {
        action,
        recorded: event.timestamp,
        outcome: outcome(event.outcome),
        outcome_description: Some(outcome_description(event.outcome).to_owned()),
        accessor: accessor(event, missing, purposes.clone()),
        subject: DataSubject {
            patient: None,
            ehrs,
            population: None,
        },
        categories: classification,
        delivered: event
            .result_count
            .and_then(|count| usize::try_from(count).ok()),
        origin_count: event
            .origin_count
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count > origins.len()),
        origins,
        request: Request {
            id: event.request_id.clone(),
            operation: operation(event),
            operation_system: matches!(event.event_type, Some(EventType::RestOperation(_)))
                .then(|| OPERATION_SYSTEM.to_owned()),
            resource: event.object_id.clone(),
            query: (action == Action::Query)
                .then(|| event.object_id.clone().map(SecretString::from))
                .flatten(),
            stored_query: None,
            client_address: client_address(event),
            correlation: None,
        },
        retention,
        labels,
        emergency: event
            .emergency_access
            .then(|| ctx.recording.emergency.mark(&purposes))
            .flatten(),
        inserted: None,
        transformations: Vec::new(),
    })
}

/// The origins of the data an access read, from the systems its `FEEDER_AUDIT`s
/// name.
fn origins(event: &AuditEvent) -> Vec<Origin> {
    // NOTE: Regulation (EU) 2025/327 Annex II 3.2(e): the origin of stored data is the system
    // its FEEDER_AUDIT names, which FerroEHR did not ask, so an origin names no endpoint.
    event
        .origins
        .iter()
        .map(|system_id| Origin {
            endpoint: None,
            node: None,
            system_id: Some(system_id.clone()),
            status: None,
            rows: None,
            version: None,
            created_ehr: None,
            categories: None,
        })
        .collect()
}

/// Who accessed the data (Annex II 3.2(a), (b)), as the authentication layer
/// judged them: a Basic user has no issuer and no client, and an acting mode
/// the deployment did not establish is none.
fn accessor(event: &AuditEvent, missing: &str, purposes: Vec<Purpose>) -> Accessor {
    Accessor {
        issuer: event.issuer.clone(),
        subject: nonempty(&event.user_id, missing),
        client_id: event.client_id.clone(),
        audience: None,
        provider: event.organisation.clone(),
        acting: event.actor.mode.map(|mode| match mode {
            ActingMode::Client => Acting::Client,
            ActingMode::Person => Acting::Person,
        }),
        // NOTE: NEN 7513:2018 §5 (not vendored, NEN copyright) asks for the role or authority
        // under which the record was accessed, which BALP 1.1.4 §3:5.7.5.4 writes at role.
        roles: event
            .roles
            .iter()
            .map(|role| ehds_logging::record::Coded {
                system: None,
                code: role.clone(),
            })
            .collect(),
        token_id: event.token_id.clone(),
        assurance: event.actor.assurance_level.map(assurance),
        professional: Professional {
            name: None,
            identifier: event.actor.professional_id.clone(),
        },
        alt_id: None,
        purposes,
        // NOTE: Regulation (EU) 2025/327 Art 68 names the data permit; FHIR R4 AuditEvent.agent.policy carries it.
        policies: event.permit.iter().cloned().collect(),
        relayed: None,
    }
}

/// Whether the patient behind the record's EHR is named: the subject resolver
/// found `(id, namespace)`, found none, or could not ask.
fn patient_lookup(event: &AuditEvent, subject: Option<&str>) -> PatientLookup {
    if !event.object.is_patient_centric() && event.object != ObjectClass::Query {
        return PatientLookup::Unsupported;
    }
    match subject {
        Some(value) => PatientLookup::Found(vec![PatientIdentifier {
            namespace: event.subject_namespace.clone(),
            value: SecretString::from(value.to_owned()),
        }]),
        None if event.subject_lookup_failed => PatientLookup::Unavailable,
        None => PatientLookup::NotFound,
    }
}

/// The BALP exchange of a record that is no classified access.
fn exchange(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
    endpoint: &NetworkAddress,
) -> Exchange {
    let missing = ctx.value_if_missing.as_str();
    let user = (!event.user_id.is_empty()).then(|| {
        User::local(event.user_id.clone())
            .with_issuer(event.issuer.clone())
            .with_client_id(event.client_id.clone())
            .with_token_id(event.token_id.clone())
            .with_policies(event.permit.iter().cloned().collect())
            .with_purposes(
                event
                    .purpose
                    .iter()
                    .map(|code| PurposeOfUse {
                        system: None,
                        code: code.clone(),
                    })
                    .collect(),
            )
            .with_organisation(event.organisation.clone())
            .with_provider_identifier(event.actor.professional_id.clone())
            .with_assurance(event.actor.assurance_level.map(|level| Code {
                system: None,
                code: level.as_str().to_owned(),
            }))
            .with_roles(
                event
                    .roles
                    .iter()
                    .map(|role| Code {
                        system: None,
                        code: role.clone(),
                    })
                    .collect(),
            )
    });
    let client = Peer {
        who: event
            .client_id
            .clone()
            .unwrap_or_else(|| nonempty(&event.user_id, missing)),
        network: client_address(event).map_or_else(
            || NetworkAddress::MachineName(missing.to_owned()),
            NetworkAddress::Ip,
        ),
    };
    Exchange {
        kind: kind(event, subject.is_some()),
        recorded: event.timestamp,
        outcome: balp_outcome(event.outcome),
        outcome_description: Some(outcome_description(event.outcome).to_owned()),
        direction: Direction::Received {
            client,
            endpoint: endpoint.clone(),
        },
        on_behalf: user.map_or(OnBehalfOf::System, OnBehalfOf::User),
        entities: entities(event, subject, missing),
        security: Vec::new(),
    }
}

/// What the BALP pattern of `event` fixes; `patient` selects the `Patient*`
/// variant, which requires the patient entity.
fn kind(event: &AuditEvent, patient: bool) -> EventKind {
    let patient = patient && event.object.is_patient_centric();
    let success = event.outcome == EventOutcome::Success;
    let profile = |plain: &'static str, with_patient: &'static str| match (success, patient) {
        (false, _) => "",
        (true, true) => with_patient,
        (true, false) => plain,
    };
    let (client, server) = if event.server_is_data_source() {
        (DESTINATION_ROLE, SOURCE_ROLE)
    } else {
        (SOURCE_ROLE, DESTINATION_ROLE)
    };
    let operation = match &event.event_type {
        Some(EventType::RestOperation(op)) => Some(Coded {
            system: std::borrow::Cow::Borrowed(OPERATION_SYSTEM),
            code: op.clone(),
            display: op.clone(),
        }),
        _ => None,
    };
    let fixed = |event_type: Coded, subtypes: Vec<Coded>, action: &'static str| EventKind {
        profile: "",
        event_type,
        subtypes: subtypes.into_iter().chain(operation.clone()).collect(),
        action,
        client: client.clone(),
        server: server.clone(),
    };
    match (event.object, event.action) {
        (ObjectClass::Authentication, _) => fixed(
            USER_AUTHENTICATION,
            match event.event_type {
                Some(EventType::Login) => vec![LOGIN],
                Some(EventType::Logout) => vec![LOGOUT],
                Some(EventType::RestOperation(_)) | None => Vec::new(),
            },
            "E",
        ),
        (ObjectClass::Extract, EventActionCode::Create | EventActionCode::Update) => {
            fixed(IMPORT, Vec::new(), "C")
        }
        (ObjectClass::Extract, _) => fixed(EXPORT, Vec::new(), "R"),
        (ObjectClass::Query, _) => EventKind {
            profile: profile(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Query",
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientQuery",
            ),
            ..fixed(REST, vec![SEARCH], "E")
        },
        (_, EventActionCode::Read) => EventKind {
            profile: profile(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read",
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead",
            ),
            ..fixed(REST, vec![READ], "R")
        },
        (_, EventActionCode::Create) => EventKind {
            profile: profile(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Create",
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientCreate",
            ),
            ..fixed(REST, vec![CREATE], "C")
        },
        (_, EventActionCode::Update) => EventKind {
            profile: profile(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Update",
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientUpdate",
            ),
            ..fixed(REST, vec![UPDATE], "U")
        },
        (_, EventActionCode::Delete) => EventKind {
            profile: profile(
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Delete",
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientDelete",
            ),
            client: APPLICATION,
            server: CUSTODIAN,
            ..fixed(REST, vec![DELETE], "D")
        },
        (_, EventActionCode::Execute) => fixed(REST, vec![OPERATION], "E"),
    }
}

/// The entities of a record that is no classified access: the query, or the
/// patient and the object the record names.
fn entities(event: &AuditEvent, subject: Option<&str>, missing: &str) -> Vec<Entity> {
    if event.object.is_query() {
        return vec![Entity::Query(SecretString::from(
            event
                .object_id
                .clone()
                .unwrap_or_else(|| missing.to_owned()),
        ))];
    }
    let mut entities = Vec::new();
    if event.object.is_patient_centric()
        && let Some(value) = subject
    {
        let namespace = event.subject_namespace.clone();
        let uri = namespace
            .as_deref()
            .is_some_and(|namespace| url::Url::parse(namespace).is_ok());
        entities.push(Entity::Patient {
            system: namespace.clone().filter(|_| uri),
            value: SecretString::from(value.to_owned()),
            namespace: namespace.filter(|_| !uri),
        });
    }
    // NOTE: BALP 1.1.4 Create, Read, Update and Delete require entity:data (type 2) with what
    // and role, so those records name the operation when they name no object.
    let crud = !matches!(
        event.object,
        ObjectClass::Authentication | ObjectClass::Extract
    ) && matches!(
        event.action,
        EventActionCode::Create
            | EventActionCode::Read
            | EventActionCode::Update
            | EventActionCode::Delete
    );
    let id = event
        .object_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| crud.then(|| operation(event)));
    if let Some(id) = id {
        entities.push(Entity::Described(Described {
            what: Some(What::Identifier {
                system: None,
                value: id,
            }),
            kind: SYSTEM_OBJECT,
            role: crud.then_some(DOMAIN_RESOURCE),
            name: Some(operation(event)),
            description: None,
            details: Vec::new(),
            security_labels: Vec::new(),
        }));
    }
    entities
}

/// The operation a record names: the ITS-REST operation id, else the DICOM
/// event the resource class audits as.
fn operation(event: &AuditEvent) -> String {
    match &event.event_type {
        Some(EventType::RestOperation(op)) => op.to_string(),
        _ => event.object.event_id(event.action).1.to_owned(),
    }
}

fn client_address(event: &AuditEvent) -> Option<IpAddr> {
    event.client_ip.as_deref()?.parse().ok()
}

const fn outcome(outcome: EventOutcome) -> Outcome {
    match outcome {
        EventOutcome::Success => Outcome::Success,
        EventOutcome::MinorFailure => Outcome::MinorFailure,
        EventOutcome::SeriousFailure => Outcome::SeriousFailure,
    }
}

const fn balp_outcome(outcome: EventOutcome) -> ihe_atna::balp::Outcome {
    match outcome {
        EventOutcome::Success => ihe_atna::balp::Outcome::Success,
        EventOutcome::MinorFailure => ihe_atna::balp::Outcome::MinorFailure,
        EventOutcome::SeriousFailure => ihe_atna::balp::Outcome::SeriousFailure,
    }
}

const fn assurance(level: ferrohealth_smart::assurance::AssuranceLevel) -> AssuranceLevel {
    match level {
        ferrohealth_smart::assurance::AssuranceLevel::Low => AssuranceLevel::Low,
        ferrohealth_smart::assurance::AssuranceLevel::Substantial => AssuranceLevel::Substantial,
        ferrohealth_smart::assurance::AssuranceLevel::High => AssuranceLevel::High,
    }
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::categories::{AccessedContent, Classifier, QueryConstraints};
    use crate::system_log::event::ActorAuthentication;
    use base64::Engine;
    use ferrohealth_smart::assurance::AssuranceLevel as Level;
    use jiff::Timestamp;
    use secrecy::ExposeSecret as _;

    const OTHER_ID: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/ihe-otherId";

    fn ctx() -> AuditContext {
        AuditContext {
            source_id: "ferroehr".to_owned(),
            enterprise_site_id: "1f332a66-0000-0000-0000-000000000001".to_owned(),
            server_ip: "10.42.23.77".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
            ..AuditContext::default()
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

    /// The agent whose `who.identifier.value` is `value`.
    fn agent<'a>(v: &'a serde_json::Value, value: &str) -> &'a serde_json::Value {
        v["agent"]
            .as_array()
            .expect("agents")
            .iter()
            .find(|a| a["who"]["identifier"]["value"] == value)
            .unwrap_or_else(|| panic!("no agent {value}: {v}"))
    }

    /// The `ihe-otherId` value of `agent`, the professional identifier BALP
    /// 1.1.4 §3:5.7.5.4 maps the IUA `national_provider_identifier` to.
    fn other_id(agent: &serde_json::Value) -> Option<&str> {
        agent["extension"]
            .as_array()?
            .iter()
            .find(|x| x["url"] == OTHER_ID)?["valueIdentifier"]["value"]
            .as_str()
    }

    /// The `patient-lookup` detail of the access record `v` writes.
    fn patient_lookup_detail(v: &serde_json::Value) -> Vec<&str> {
        v["entity"]
            .as_array()
            .expect("entities")
            .iter()
            .filter_map(|entity| entity["detail"].as_array())
            .flatten()
            .filter(|detail| detail["type"] == "patient-lookup")
            .filter_map(|detail| detail["valueString"].as_str())
            .collect()
    }

    /// A subject lookup that could not ask writes `unavailable`, and one that
    /// found no subject `not-found` (Regulation (EU) 2025/327 Art 9(1)).
    #[test]
    fn a_failed_subject_lookup_is_recorded_as_unavailable() {
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Query,
            EventOutcome::Success,
        );
        e.object_id = Some("eu.ferroehr::q1".to_owned());
        e.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
        e.category = Some(Classifier::default().classify(
            &AccessedContent::Query(QueryConstraints {
                reaches_content: true,
                ..QueryConstraints::default()
            }),
            false,
        ));
        let none = json(&e, None);
        assert_eq!(patient_lookup_detail(&none), ["not-found"], "{none}");
        e.subject_lookup_failed = true;
        let failed = json(&e, None);
        assert_eq!(patient_lookup_detail(&failed), ["unavailable"], "{failed}");
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
        e.event_type = Some(EventType::rest("composition_get"));
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

    /// The DICOM message and the FHIR document give the client and the server
    /// the same data-flow role for one record: on a read (and an export) the
    /// server is Source (110153) and the client Destination (110152), per
    /// DICOM PS3.15 §A.5.2.7 and the BALP `Read` agent slices; on a write and
    /// on a query (§A.5.3.10, BALP `Query`) the client is Source.
    #[test]
    fn dicom_and_fhir_agree_on_the_data_flow_direction() {
        let cases = [
            (
                EventActionCode::Read,
                ObjectClass::Composition,
                "110152",
                "110153",
            ),
            (
                EventActionCode::Read,
                ObjectClass::Extract,
                "110152",
                "110153",
            ),
            (
                EventActionCode::Create,
                ObjectClass::Composition,
                "110153",
                "110152",
            ),
            (
                EventActionCode::Update,
                ObjectClass::Ehr,
                "110153",
                "110152",
            ),
            (
                EventActionCode::Execute,
                ObjectClass::Query,
                "110153",
                "110152",
            ),
            (
                EventActionCode::Read,
                ObjectClass::Query,
                "110153",
                "110152",
            ),
        ];
        for (action, object, client, server) in cases {
            let e = event(action, object, EventOutcome::Success);
            let v = json(&e, Some("patient-42"));
            let agents = v["agent"].as_array().expect("agents");
            assert_eq!(
                agents[0]["type"]["coding"][0]["code"], client,
                "FHIR client {action:?} {object:?}"
            );
            assert_eq!(
                agents[1]["type"]["coding"][0]["code"], server,
                "FHIR server {action:?} {object:?}"
            );

            let xml =
                crate::system_log::message::to_xml(&e, &ctx(), Some("patient-42")).expect("xml");
            let xml = String::from_utf8(xml.expose_secret().to_vec()).expect("utf-8");
            let roles: Vec<&str> = xml
                .split("<RoleIDCode csd-code=\"")
                .skip(1)
                .filter_map(|rest| rest.split('"').next())
                .collect();
            assert_eq!(
                roles,
                vec![client, server],
                "DICOM participants {action:?} {object:?}"
            );
        }
    }

    /// BALP 1.1.4 slices `AuditEvent.entity` by `type` on its Create, Read,
    /// Update and Delete profiles and requires `entity:data` (type `2`) with
    /// `what` and `role`
    /// (<https://profiles.ihe.net/ITI/BALP/1.1.4/StructureDefinition-IHE.BasicAudit.Create.html>),
    /// so an unclassified record carries it, naming the operation when it
    /// names no object.
    #[test]
    fn an_unclassified_crud_record_carries_the_data_entity() {
        for object_id in [Some("IDCR - Vital Signs.v1"), None] {
            let mut e = event(
                EventActionCode::Create,
                ObjectClass::Template,
                EventOutcome::Success,
            );
            e.object_id = object_id.map(str::to_owned);
            e.event_type = Some(EventType::rest("definition_template_adl1.4_upload"));
            let v = json(&e, None);
            assert_eq!(
                v["meta"]["profile"][0],
                "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Create"
            );
            let data: Vec<&serde_json::Value> = v["entity"]
                .as_array()
                .expect("entities")
                .iter()
                .filter(|entity| entity["type"]["code"] == "2")
                .collect();
            assert_eq!(data.len(), 1, "{v}");
            assert_eq!(data[0]["role"]["code"], "4", "{v}");
            assert_eq!(
                data[0]["what"]["identifier"]["value"],
                object_id.unwrap_or("definition_template_adl1.4_upload"),
                "{v}"
            );
        }
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

    /// `IHE.BasicAudit.Delete` (and `PatientDelete`, derived from it) fixes
    /// `agent:client.type` to DCM 110150 and `agent:server.type` to
    /// provenance-participant-type `custodian`
    /// (<https://profiles.ihe.net/ITI/BALP/1.1.4/StructureDefinition-IHE.BasicAudit.Delete.html>);
    /// the person is the `agent:user` slice, the requestor (BALP 1.1.4
    /// §3:5.7.5.4).
    #[test]
    fn delete_carries_the_agent_types_balp_delete_fixes() {
        for (subject, profile) in [
            (Some("patient-42"), "IHE.BasicAudit.PatientDelete"),
            (None, "IHE.BasicAudit.Delete"),
        ] {
            let mut e = event(
                EventActionCode::Delete,
                ObjectClass::Composition,
                EventOutcome::Success,
            );
            e.object_id = Some("8fa1::ferroehr::2".to_owned());
            let v = json(&e, subject);
            assert_eq!(
                v["meta"]["profile"][0],
                format!("https://profiles.ihe.net/ITI/BALP/StructureDefinition/{profile}")
            );
            assert_eq!(v["subtype"][0]["code"], "delete");
            let client = &v["agent"][0]["type"]["coding"][0];
            assert_eq!(
                client["system"],
                "http://dicom.nema.org/resources/ontology/DCM"
            );
            assert_eq!(client["code"], "110150");
            let server = &v["agent"][1]["type"]["coding"][0];
            assert_eq!(
                server["system"],
                "http://terminology.hl7.org/CodeSystem/provenance-participant-type"
            );
            assert_eq!(server["code"], "custodian");
            assert_eq!(v["agent"][1]["requestor"], false);
            assert_eq!(agent(&v, "john doe")["requestor"], true);
        }
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
        // No patient entity without a resolved subject; the BALP Read profile's
        // entity:data (type 2) is still there.
        let entities = v["entity"].as_array().expect("entities");
        assert!(
            entities.iter().all(|entity| entity["type"]["code"] != "1"),
            "{v}"
        );
        assert!(
            entities.iter().any(|entity| entity["type"]["code"] == "2"),
            "{v}"
        );
    }

    /// The BALP `RESTful` profiles fix outcome = 0; a failure record keeps the
    /// shape but claims nothing.
    #[test]
    fn failure_claims_no_profile() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::MinorFailure,
        );
        let v = json(&e, Some("patient-42"));
        assert!(v.get("meta").is_none());
        assert_eq!(v["outcome"], "4");
    }

    /// A query is `rest` with the `search` subtype `IHE.BasicAudit.Query`
    /// fixes (`subtype:anySearch`), and its entity the base64 request.
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
        assert_eq!(v["subtype"][0]["code"], "search");
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

    /// A classified query renders one origin entity per recorded origin, each
    /// naming the system the data came from (EHDS Annex II 3.2(e)).
    #[test]
    fn query_record_carries_the_origins_of_the_served_data() {
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Query,
            EventOutcome::Success,
        );
        e.object_id = Some("eu.ferroehr::q1".to_owned());
        e.record_origins(vec!["system-a".to_owned(), "system-b".to_owned()], Some(7));
        e.category = Some(Classifier::default().classify(
            &AccessedContent::Query(QueryConstraints {
                reaches_content: true,
                ..QueryConstraints::default()
            }),
            false,
        ));
        let v = json(&e, None);
        let entities = v["entity"].as_array().expect("entities");
        let mut served: Vec<&str> = entities
            .iter()
            .filter(|entity| entity["name"] == "origin")
            .flat_map(|entity| entity["detail"].as_array().expect("details"))
            .filter(|detail| detail["type"] == "system-id")
            .filter_map(|detail| detail["valueString"].as_str())
            .collect();
        served.sort_unstable();
        assert_eq!(served, ["system-a", "system-b"], "{v}");
        assert!(
            entities.iter().any(|entity| entity["role"]["code"] == "24"),
            "the query entity stays: {v}"
        );
    }

    /// The token identity is the issuer and the subject on the `agent:user`
    /// slice and the client on the Application agent (BALP 1.1.4 §3:5.7.5.4).
    #[test]
    fn the_token_identity_is_the_issuer_subject_and_client() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.issuer = Some("https://idp.example".to_owned());
        e.client_id = Some("ward-app".to_owned());
        let v = json(&e, Some("patient-42"));
        let user = agent(&v, "john doe");
        assert_eq!(user["who"]["identifier"]["system"], "https://idp.example");
        assert_eq!(user["requestor"], true);
        assert_eq!(agent(&v, "ward-app")["requestor"], false);
    }

    /// A client application acting for a named professional names the token's
    /// subject as the `agent:user`, with the professional's identifier as its
    /// `ihe-otherId` and the assurance level as `ihe-assuranceLevel` (BALP
    /// 1.1.4 §3:5.7.5.4), never the raw claim value.
    #[test]
    fn a_client_acting_for_a_professional_names_the_professional() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.user_id = "scheduler-app".to_owned();
        e.client_id = Some("scheduler-app".to_owned());
        e.roles = vec!["CLINICIAN".to_owned()];
        e.actor = ActorAuthentication {
            mode: Some(ActingMode::Client),
            assurance_level: Some(Level::Substantial),
            assurance_value: Some("loa-substantial".to_owned()),
            professional_id: Some("prof-007".to_owned()),
        };
        let v = json(&e, Some("patient-42"));
        let user = v["agent"]
            .as_array()
            .expect("agents")
            .iter()
            .find(|a| a["requestor"] == true)
            .expect("the agent:user");
        assert_eq!(other_id(user), Some("prof-007"), "{v}");
        assert_eq!(user["role"][0]["coding"][0]["code"], "CLINICIAN");
        assert_eq!(v["agent"][0]["network"]["address"], "10.216.24.150");
        assert_eq!(v["agent"][0]["type"]["coding"][0]["code"], "110152");
        assert_eq!(v["agent"][1]["who"]["display"], "ferroehr");
        let rendered = v.to_string();
        assert!(!rendered.contains("loa-substantial"), "{rendered}");
    }

    /// A professional identifier other than the token subject is the
    /// `ihe-otherId` of the `agent:user`, whose `who` stays the subject.
    #[test]
    fn a_person_with_a_professional_identifier_carries_it_as_other_id() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.actor = ActorAuthentication {
            mode: Some(ActingMode::Person),
            professional_id: Some("uzi-12345".to_owned()),
            ..ActorAuthentication::default()
        };
        let v = json(&e, Some("patient-42"));
        let user = agent(&v, "john doe");
        assert_eq!(other_id(user), Some("uzi-12345"));
        assert_eq!(user["requestor"], true);
    }

    /// A client token naming no professional renders one `agent:user`, never
    /// an invented person.
    #[test]
    fn a_client_without_a_professional_renders_the_client_alone() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Template,
            EventOutcome::Success,
        );
        e.actor.mode = Some(ActingMode::Client);
        let v = json(&e, None);
        let requestors: Vec<_> = v["agent"]
            .as_array()
            .expect("agents")
            .iter()
            .filter(|a| a["requestor"] == true)
            .collect();
        assert_eq!(requestors.len(), 1, "{v}");
        assert_eq!(requestors[0]["who"]["identifier"]["value"], "john doe");
        assert!(other_id(requestors[0]).is_none());
    }

    /// BALP 1.1.4 §3:5.7.5.4 writes the token's `jti` at `agent[user].policy`,
    /// on a classified access and on every other record.
    #[test]
    fn the_token_id_is_the_users_policy() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.token_id = Some("jti-1".to_owned());
        let v = json(&e, Some("patient-42"));
        assert_eq!(agent(&v, "john doe")["policy"][0], "jti-1", "{v}");
        e.category =
            Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
        let classified = json(&e, Some("patient-42"));
        assert_eq!(
            agent(&classified, "john doe")["policy"][0],
            "jti-1",
            "{classified}"
        );
    }

    /// The declared data permit (Regulation (EU) 2025/327 Art 68) follows the
    /// token's `jti` at `agent[user].policy` (FHIR R4 `AuditEvent.agent.policy`)
    /// on a classified access and on every other record, and is absent when
    /// the caller declared none.
    #[test]
    fn the_declared_permit_follows_the_token_id_as_a_policy() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.token_id = Some("jti-1".to_owned());
        let without = json(&e, Some("patient-42"));
        assert_eq!(
            agent(&without, "john doe")["policy"],
            serde_json::json!(["jti-1"]),
            "{without}"
        );
        e.permit = Some("hdab-nl/2026/0042".to_owned());
        let with = json(&e, Some("patient-42"));
        assert_eq!(
            agent(&with, "john doe")["policy"],
            serde_json::json!(["jti-1", "hdab-nl/2026/0042"]),
            "{with}"
        );

        e.category =
            Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
        let classified = json(&e, Some("patient-42"));
        assert_eq!(
            agent(&classified, "john doe")["policy"],
            serde_json::json!(["jti-1", "hdab-nl/2026/0042"]),
            "{classified}"
        );
        e.permit = None;
        let classified_without = json(&e, Some("patient-42"));
        assert_eq!(
            agent(&classified_without, "john doe")["policy"],
            serde_json::json!(["jti-1"]),
            "{classified_without}"
        );
    }

    /// The ITS-REST operation id is a subtype in its own code system, after
    /// the restful-interaction code (every BALP `RESTful` pattern slices
    /// `subtype` openly), on a classified access and on every other record.
    #[test]
    fn the_operation_id_is_a_subtype() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.event_type = Some(EventType::rest("composition_get"));
        for v in [json(&e, Some("patient-42")), {
            e.category =
                Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
            json(&e, Some("patient-42"))
        }] {
            assert_eq!(v["subtype"][0]["code"], "read", "{v}");
            assert_eq!(v["subtype"][1]["code"], "composition_get", "{v}");
            assert_eq!(v["subtype"][1]["system"], OPERATION_SYSTEM, "{v}");
        }
    }

    /// FHIR R4 `AuditEvent.outcomeDesc` and `AuditEvent.source.type`
    /// (`security-source-type` `4`, an application server).
    #[test]
    fn the_outcome_description_and_the_source_type_are_written() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::MinorFailure,
        );
        let v = json(&e, Some("patient-42"));
        assert_eq!(v["outcomeDesc"], "Operation failed", "{v}");
        assert_eq!(
            v["source"]["type"][0]["system"],
            "http://terminology.hl7.org/CodeSystem/security-source-type"
        );
        assert_eq!(v["source"]["type"][0]["code"], "4");
    }

    /// A Basic user names no issuer and no client: the record writes neither,
    /// never FerroEHR's own URN or `UNKNOWN` in their place.
    #[test]
    fn a_basic_user_writes_no_issuer_and_no_client() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        let v = json(&e, Some("patient-42"));
        let user = agent(&v, "john doe");
        assert!(user["who"]["identifier"].get("system").is_none(), "{v}");
        let text = v.to_string();
        assert!(!text.contains("urn:ferroehr"), "{text}");
        assert!(!text.contains("UNKNOWN"), "{text}");
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

    /// The emergency mark (EHDS Art. 11(5)) renders as the `ehds-logging`
    /// entity that states it in words, beside the declared purpose; an
    /// unmarked access renders no such entity, whatever purpose it declared.
    #[test]
    fn the_emergency_mark_renders_as_its_entity() {
        let etreat = Purpose {
            system: None,
            code: "ETREAT".to_owned(),
        };
        let marking = AuditContext {
            recording: Recording {
                emergency: EmergencyPurposes::declare(&[etreat]).expect("one purpose"),
                ..Recording::default()
            },
            ..ctx()
        };
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.purpose = Some("ETREAT".to_owned());
        e.category =
            Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
        let mark = |v: &serde_json::Value| {
            v["entity"]
                .as_array()
                .expect("entities")
                .iter()
                .find(|entity| entity["name"] == ehds_logging::balp::EMERGENCY_ENTITY)
                .cloned()
        };
        let unmarked = to_fhir(&e, &marking, None).expect("render");
        assert!(mark(&unmarked).is_none(), "{unmarked}");

        e.emergency_access = true;
        let rendered = to_fhir(&e, &marking, None).expect("render");
        let entity = mark(&rendered).expect("the mark");
        assert_eq!(
            entity["description"],
            ehds_logging::balp::EMERGENCY_DESCRIPTION
        );
        assert_eq!(
            agent(&rendered, "john doe")["purposeOfUse"][0]["coding"][0]["code"],
            "ETREAT",
            "the declared code stays on the requesting agent"
        );
    }

    /// A record outside the EHDS access record ends the day after its date of
    /// access plus the store horizon, in UTC, and never under a forever horizon.
    #[test]
    fn a_horizon_end_is_the_day_after_the_period() {
        use crate::system_log::config::Retention;
        let at: Timestamp = "2026-02-28T23:30:00Z".parse().expect("timestamp");
        let date = |s: &str| s.parse::<jiff::civil::Date>().expect("date");
        assert_eq!(
            horizon_end(at, Retention::Days(1)),
            Some(date("2026-03-02"))
        );
        assert_eq!(
            horizon_end(at, Retention::Years(3)),
            Some(date("2029-03-01"))
        );
        assert_eq!(horizon_end(at, Retention::Forever), None);
    }
}
