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

use ehds_logging::balp::ObjectIds;
use ehds_logging::emergency::EmergencyPurposes;
use ehds_logging::label::LabelPolicy;
use ehds_logging::record::{
    AccessRecord, Action, DataSubject, EhrAt, Origin, Outcome, PatientIdentifier, PatientLookup,
    Purpose, Request,
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
use crate::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, OPERATION_SYSTEM, ObjectClass,
};
use crate::system_log::fhir_accessor::accessor;
use crate::system_log::message::AuditContext;

/// The canonical-URL prefix of the IHE BALP profiles (v1.1.4).
pub const BALP_PROFILE_BASE: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition";

/// The namespace of the openEHR object ids (BASE 1.3.0 `OBJECT_ID.value`) an
/// access record names its data entity by, written as
/// `entity[data].what.identifier.system`.
// NOTE: no specification governs this: our own design; an openEHR object id is no FHIR
// resource, so it is an identifier (FHIR R4 references.html §2.3.0.3) in a namespace we name.
pub const OBJECT_ID_SYSTEM: &str = "urn:openehr:object-id";

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
    let exchange = match access_record(event, ctx, subject)? {
        Some(record) => ehds_logging::balp::exchange_with(&record, &endpoint, &object_ids()?),
        None => exchange(event, ctx, subject, &endpoint),
    };
    exchange
        .audit_event_value(&observer)
        .map(ihe_atna::balp::AuditEventValue::into_value)
        .map_err(|e| AuditError::Render(Box::new(e)))
}

/// How an access record names the openEHR object ids FerroEHR passes as its
/// resource: identifiers in [`OBJECT_ID_SYSTEM`], never literal references.
///
/// # Errors
///
/// [`AuditError::Render`] when [`OBJECT_ID_SYSTEM`] is not a URI.
fn object_ids() -> Result<ObjectIds, AuditError> {
    let system = url::Url::parse(OBJECT_ID_SYSTEM).map_err(|e| AuditError::Render(Box::new(e)))?;
    Ok(ObjectIds::Identifiers {
        system: Some(system),
    })
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
pub(crate) fn access_record(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Result<Option<AccessRecord>, AuditError> {
    let Some(category) = event.category.as_ref() else {
        return Ok(None);
    };
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
        .purpose_codings()
        .into_iter()
        .map(|coding| Purpose {
            system: coding.system,
            code: coding.code,
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
    let (accessor, correlation) = accessor(event, missing, purposes.clone())?;
    Ok(Some(AccessRecord {
        action,
        recorded: event.timestamp,
        outcome: outcome(event.outcome),
        outcome_description: Some(outcome_description(event.outcome).to_owned()),
        accessor,
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
            correlation,
        },
        retention,
        labels,
        emergency: event
            .emergency_access
            .then(|| ctx.recording.emergency.mark(&purposes))
            .flatten(),
        inserted: None,
        transformations: Vec::new(),
    }))
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
            .with_policies(event.policies())
            .with_purposes(
                event
                    .purpose_codings()
                    .into_iter()
                    .map(|coding| PurposeOfUse {
                        system: coding.system,
                        code: coding.code,
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
                    .role_codings()
                    .into_iter()
                    .map(|role| Code {
                        system: role.system,
                        code: role.code,
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

pub(super) fn nonempty(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

/// The FHIR R4 `AuditEvent` document for one resolved record, or `None` when
/// the rendering failed (metered + logged, never silent).
pub(crate) fn render_audit_event(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Option<serde_json::Value> {
    match to_fhir(event, ctx, subject) {
        Ok(document) => Some(document),
        Err(e) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_serialize_failed
                .add(1, &[]);
            tracing::warn!("ATNA audit FHIR rendering failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
