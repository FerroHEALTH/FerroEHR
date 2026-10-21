// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DICOM audit message (DICOM PS3.15 §A.5) of one audit record, built on
//! `ihe-atna`'s message model, which writes the XML (ITI TF-2 §3.20.4.1.2).
//!
//! FerroEHR decides what the message says: the event id per resource class,
//! the participants and their data-flow roles, the participant objects. The
//! shared crate holds the model and the writer.
//!
//! The record's EHDS categories, the accessing organisation, the purpose of
//! use, the authentication assurance level and the emergency mark have no
//! element in the PS3.15 §A.5 schema, so this rendering omits them; the local
//! store and the FHIR `AuditEvent` carry them. The natural person behind a
//! client application is a second `ActiveParticipant`.

use ihe_atna::atna::message::{
    AccessPoint, ActiveParticipant, AuditMessage, AuditSource, CodedValue, EventIdentification,
    ParticipantObject,
};
use secrecy::{SecretSlice, SecretString};

use crate::system_log::AuditError;
use crate::system_log::codes::{
    self, AtnaAction, AtnaEventType, AtnaObject, AtnaOutcome, Code, NETWORK_ACCESS_POINT_IP,
    OBJECT_ROLE_PATIENT, OBJECT_ROLE_QUERY, OBJECT_TYPE_PERSON, OBJECT_TYPE_SYSTEM,
};
use crate::system_log::event::{ActingMode, AuditEvent};

/// The server-side identity shared by every emitted record.
///
/// The destination node and audit source, and the policies an access record
/// is written under. Built once by the sender from
/// [`super::config::AuditConfig`].
#[derive(Debug, Clone, Default)]
pub struct AuditContext {
    /// `AuditSourceID` and the destination `ActiveParticipant.UserID`.
    pub source_id: String,
    /// `AuditEnterpriseSiteID`.
    pub enterprise_site_id: String,
    /// This node's `NetworkAccessPointID` (destination participant).
    pub server_ip: String,
    /// Fill for empty mandatory fields (default `UNKNOWN`).
    pub value_if_missing: String,
    /// The retention, label and emergency policies of the EHDS access record.
    #[cfg(feature = "fhir")]
    pub recording: crate::system_log::fhir::Recording,
}

/// The `codeSystemName` of the deployment's own role vocabulary on a
/// `RoleIDCode`: the RBAC roles a caller held, which no published code system
/// names.
const ROLE_VOCABULARY: &str = "urn:ferroehr:role";

/// Builds the DICOM audit message of a resolved [`AuditEvent`], with the server
/// [`AuditContext`] and the optionally-resolved patient subject id (`None` →
/// `ctx.value_if_missing`).
#[must_use]
pub fn build(event: &AuditEvent, ctx: &AuditContext, subject: Option<&str>) -> AuditMessage {
    let missing = ctx.value_if_missing.as_str();
    let (event_code, event_text) = event.object.event_id(event.action);
    let client_ip = event
        .client_ip
        .clone()
        .unwrap_or_else(|| missing.to_owned());
    let user_id = nonempty(&event.user_id, missing);
    let (client_role, server_role) = if event.server_is_data_source() {
        (codes::ROLE_DESTINATION, codes::ROLE_SOURCE)
    } else {
        (codes::ROLE_SOURCE, codes::ROLE_DESTINATION)
    };
    let mut participants = requesting_participants(event, user_id, client_ip, client_role);
    participants.push(ActiveParticipant {
        user_id: nonempty(&ctx.source_id, missing),
        alternative_user_id: None,
        user_name: None,
        user_is_requestor: false,
        role_id_codes: vec![coded(server_role)],
        access_point: Some(ip_point(nonempty(&ctx.server_ip, missing))),
    });
    AuditMessage {
        event: EventIdentification {
            event_id: CodedValue::new(event_code, codes::DCM, event_text),
            action: event.action.as_code(),
            date_time: event.timestamp,
            outcome: event.outcome.as_code(),
            type_codes: event
                .event_type
                .as_ref()
                .map(|kind| coded(kind.code()))
                .into_iter()
                .collect(),
        },
        participants,
        source: AuditSource {
            id: nonempty(&ctx.source_id, missing),
            enterprise_site: Some(nonempty(&ctx.enterprise_site_id, missing)),
        },
        objects: build_objects(event, subject, missing),
    }
}

/// The message of `event` as UTF-8 XML, ready for the syslog frame.
///
/// # Errors
/// [`AuditError::Xml`] when the writer fails, which writing to memory does not.
pub fn to_xml(
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
) -> Result<SecretSlice<u8>, AuditError> {
    build(event, ctx, subject)
        .to_xml()
        .map_err(|e| AuditError::Xml(Box::new(e)))
}

/// The requesting side's `ActiveParticipant`s (PS3.15 §A.5.1).
///
/// A client application acting for a named professional yields two: the
/// person as requestor with the security roles, and the application at the
/// network address with `client_role`, the data-flow role
/// [`AuditEvent::server_is_data_source`] selects. A professional identifier
/// other than the token subject becomes `UserID`, the subject
/// `AlternativeUserID`.
fn requesting_participants(
    event: &AuditEvent,
    user_id: String,
    client_ip: String,
    client_role: Code,
) -> Vec<ActiveParticipant> {
    let roles: Vec<CodedValue> = event
        .roles
        .iter()
        .map(|role| CodedValue::new(role, ROLE_VOCABULARY, role))
        .collect();
    let professional = event
        .actor
        .professional_id
        .as_deref()
        .filter(|id| !id.is_empty());
    if let (Some(ActingMode::Client), Some(person)) = (event.actor.mode, professional) {
        return vec![
            ActiveParticipant {
                user_id: person.to_owned(),
                alternative_user_id: None,
                user_name: None,
                user_is_requestor: event.user_is_requestor,
                role_id_codes: roles,
                access_point: None,
            },
            ActiveParticipant {
                user_id,
                alternative_user_id: None,
                user_name: None,
                user_is_requestor: false,
                role_id_codes: vec![coded(client_role)],
                access_point: Some(ip_point(client_ip)),
            },
        ];
    }
    let (user_id, alternative_user_id) = match professional {
        Some(person) if person != user_id => (person.to_owned(), Some(user_id)),
        _ => (user_id, None),
    };
    let mut role_id_codes = vec![coded(client_role)];
    role_id_codes.extend(roles);
    vec![ActiveParticipant {
        user_id,
        alternative_user_id,
        user_name: None,
        user_is_requestor: event.user_is_requestor,
        role_id_codes,
        access_point: Some(ip_point(client_ip)),
    }]
}

/// Assemble the `ParticipantObjectIdentification` list for an event
/// (DICOM PS3.15 §A.5 / RFC 3881 §5.5 participant-object mapping).
fn build_objects(
    event: &AuditEvent,
    subject: Option<&str>,
    missing: &str,
) -> Vec<ParticipantObject> {
    let mut objects = Vec::new();
    let object_class = event.object;

    if object_class.is_query() {
        // A single Search-Criteria object (ad-hoc → UNKNOWN; stored → query name).
        objects.push(object(
            event
                .object_id
                .clone()
                .unwrap_or_else(|| missing.to_owned()),
            OBJECT_TYPE_SYSTEM,
            Some(OBJECT_ROLE_QUERY),
            codes::OBJ_ID_SEARCH_CRITERIA,
        ));
        return objects;
    }

    if object_class.is_patient_centric() {
        objects.push(object(
            subject.map_or_else(|| missing.to_owned(), str::to_owned),
            OBJECT_TYPE_PERSON,
            Some(OBJECT_ROLE_PATIENT),
            codes::OBJ_ID_PATIENT_NUMBER,
        ));
    }

    if object_class.has_object_uri() {
        // Patient-centric classes already carry the Patient-Number object and
        // add the URI object when the id is known; non-patient classes always
        // carry it, filled with `value_if_missing` when unknown, so no required
        // element is absent (DICOM PS3.15 §A.5).
        if let Some(id) = event.object_id.clone() {
            objects.push(uri_object(id));
        } else if !object_class.is_patient_centric() {
            objects.push(uri_object(missing.to_owned()));
        }
    }

    objects
}

fn uri_object(id: String) -> ParticipantObject {
    object(id, OBJECT_TYPE_SYSTEM, None, codes::OBJ_ID_URI)
}

fn object(
    id: String,
    type_code: &'static str,
    type_code_role: Option<&'static str>,
    id_type_code: Code,
) -> ParticipantObject {
    ParticipantObject {
        id: SecretString::from(id),
        type_code: Some(type_code),
        type_code_role,
        id_type_code: coded(id_type_code),
        query: None,
        details: Vec::new(),
    }
}

fn coded(code: Code) -> CodedValue {
    CodedValue::new(code.csd_code, code.code_system, code.original_text)
}

fn ip_point(id: String) -> AccessPoint {
    AccessPoint {
        type_code: NETWORK_ACCESS_POINT_IP,
        id,
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
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::system_log::event::{EventActionCode, EventOutcome, EventType, ObjectClass};
    use jiff::Timestamp;

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

    fn xml(event: &AuditEvent, subject: Option<&str>) -> String {
        let bytes = to_xml(event, &ctx(), subject).expect("xml");
        String::from_utf8(bytes.expose_secret().to_vec()).expect("utf-8")
    }

    // Redact the volatile EventDateTime before snapshotting.
    macro_rules! assert_audit_snapshot {
        ($name:expr, $xml:expr) => {{
            insta::with_settings!({filters => vec![
                (r#"EventDateTime="[^"]*""#, r#"EventDateTime="[REDACTED]""#),
            ]}, {
                insta::assert_snapshot!($name, $xml);
            });
        }};
    }

    #[test]
    fn golden_ehr_create() {
        // The DICOM PS3.15 §A.5 reference EHR-create success record.
        let mut e = event(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        e.ehr_id = Some("7d44b88c-4199-4bad-9765-99c1b0a5f9d3".to_owned());
        let xml = xml(&e, Some("patient-42"));
        assert!(xml.contains(r#"EventActionCode="C""#));
        assert!(xml.contains(r#"EventOutcomeIndicator="0""#));
        assert!(xml.contains(r#"csd-code="110110""#));
        assert!(xml.contains(r#"originalText="Patient Record""#));
        assert!(xml.contains(r#"originalText="Patient Number""#));
        assert!(xml.contains(r#"ParticipantObjectID="patient-42""#));
        assert!(xml.contains(r#"UserID="john doe""#));
        assert!(xml.contains(r#"UserID="ferroehr""#));
        assert_audit_snapshot!("ehr_create_C", xml);
    }

    #[test]
    fn composition_read() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::1".to_owned());
        let xml = xml(&e, Some("patient-42"));
        assert!(xml.contains(r#"EventActionCode="R""#));
        assert!(xml.contains(r#"originalText="composition""#));
        // Both the patient participant and the object URI participant.
        assert!(xml.contains(r#"originalText="Patient Number""#));
        assert!(xml.contains(r#"originalText="URI""#));
        assert!(xml.contains(r#"ParticipantObjectID="8fa1::ferroehr::1""#));
        assert_audit_snapshot!("composition_read_R", xml);
    }

    #[test]
    fn composition_update() {
        let mut e = event(
            EventActionCode::Update,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::2".to_owned());
        let xml = xml(&e, Some("patient-42"));
        assert!(xml.contains(r#"EventActionCode="U""#));
        assert!(xml.contains(r#"ParticipantObjectID="8fa1::ferroehr::2""#));
        assert_audit_snapshot!("composition_update_U", xml);
    }

    #[test]
    fn composition_delete() {
        let mut e = event(
            EventActionCode::Delete,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::3".to_owned());
        let xml = xml(&e, Some("patient-42"));
        assert!(xml.contains(r#"EventActionCode="D""#));
        assert!(xml.contains(r#"ParticipantObjectID="8fa1::ferroehr::3""#));
        assert_audit_snapshot!("composition_delete_D", xml);
    }

    #[test]
    fn execute_ad_hoc_query() {
        // Ad-hoc query: search criteria = UNKNOWN.
        let e = event(
            EventActionCode::Execute,
            ObjectClass::Query,
            EventOutcome::Success,
        );
        let xml = xml(&e, None);
        assert!(xml.contains(r#"EventActionCode="E""#));
        // Query has the dedicated DICOM EventID 110112 "Query".
        assert!(xml.contains(r#"csd-code="110112""#));
        assert!(xml.contains(r#"originalText="Query""#));
        assert!(xml.contains(r#"originalText="Search Criteria""#));
        assert!(xml.contains(r#"ParticipantObjectID="UNKNOWN""#));
        assert_audit_snapshot!("query_execute_E", xml);
    }

    #[test]
    fn template_upload() {
        let mut e = event(
            EventActionCode::Create,
            ObjectClass::Template,
            EventOutcome::Success,
        );
        e.object_id = Some("vital_signs.v1".to_owned());
        let xml = xml(&e, None);
        assert!(xml.contains(r#"EventActionCode="C""#));
        assert!(xml.contains(r#"csd-code="110100""#));
        assert!(xml.contains(r#"originalText="template""#));
        assert!(xml.contains(r#"ParticipantObjectID="vital_signs.v1""#));
        assert!(xml.contains(r#"originalText="URI""#));
        assert_audit_snapshot!("template_upload_C", xml);
    }

    #[test]
    fn template_list_without_id_uses_fill() {
        // A template list has no single object id → URI object with the fill.
        let e = event(
            EventActionCode::Read,
            ObjectClass::Template,
            EventOutcome::Success,
        );
        assert!(xml(&e, None).contains(r#"ParticipantObjectID="UNKNOWN""#));
    }

    #[test]
    fn demographic_read() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        e.object_id = Some("party-77".to_owned());
        let xml = xml(&e, None);
        assert!(xml.contains(r#"csd-code="110110""#));
        assert!(xml.contains(r#"originalText="demographic""#));
        assert!(xml.contains(r#"ParticipantObjectID="party-77""#));
        assert_audit_snapshot!("demographic_read_R", xml);
    }

    #[test]
    fn ehr_extract_export_is_patient_and_uri_scoped() {
        // EHR-Extract communication is patient-identifiable clinical data
        // audited for non-repudiation — it carries both a Patient-Number and an
        // object-URI participant (DICOM PS3.15 §A.5 / RFC 3881 §5.5). An
        // outbound (Read) extract uses the dedicated DICOM Export EventID
        // (110106); an inbound (Create) one uses Import (110107).
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        e.object_id = Some("extract::ferroehr::1".to_owned());
        let out = xml(&e, Some("patient-42"));
        assert!(out.contains(r#"csd-code="110106""#));
        assert!(out.contains(r#"originalText="Export""#));
        assert!(out.contains(r#"originalText="Patient Number""#));
        assert!(out.contains(r#"originalText="URI""#));
        assert!(out.contains(r#"ParticipantObjectID="patient-42""#));
        assert!(out.contains(r#"ParticipantObjectID="extract::ferroehr::1""#));

        // The inbound direction resolves to Import (110107).
        let mut imp = event(
            EventActionCode::Create,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        imp.object_id = Some("extract::ferroehr::2".to_owned());
        let inbound = xml(&imp, Some("patient-42"));
        assert!(inbound.contains(r#"csd-code="110107""#));
        assert!(inbound.contains(r#"originalText="Import""#));
    }

    #[test]
    fn authentication_login_record() {
        // A genuine login: DICOM EventID 110114 "User Authentication" with
        // EventTypeCode 110122 "Login"; no clinical participant object.
        let mut e = event(
            EventActionCode::Execute,
            ObjectClass::Authentication,
            EventOutcome::Success,
        );
        e.event_type = Some(EventType::Login);
        let xml = xml(&e, None);
        assert!(xml.contains(r#"csd-code="110114""#));
        assert!(xml.contains(r#"originalText="User Authentication""#));
        assert!(xml.contains(r#"csd-code="110122""#));
        assert!(xml.contains("<EventTypeCode"));
        assert!(!xml.contains("ParticipantObjectIdentification"));
        assert_audit_snapshot!("authentication_login", xml);
    }

    #[test]
    fn rest_operation_event_type_uses_own_code_system() {
        // The concrete ITS-REST operation is the EventTypeCode under our own
        // `openEHR-ITS-REST` code system name (no external system governs it).
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.object_id = Some("8fa1::ferroehr::1".to_owned());
        e.event_type = Some(EventType::RestOperation("composition_get"));
        let xml = xml(&e, Some("patient-42"));
        assert!(xml.contains(r#"csd-code="composition_get""#));
        assert!(xml.contains(r#"codeSystemName="openEHR-ITS-REST""#));
    }

    #[test]
    fn missing_subject_uses_fill() {
        // No resolved subject and no client ip → value_if_missing everywhere.
        let mut e = AuditEvent::new(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        e.timestamp = "2026-07-06T12:00:00Z".parse::<Timestamp>().unwrap();
        let xml = xml(&e, None);
        assert!(xml.contains(r#"ParticipantObjectID="UNKNOWN""#));
        assert!(xml.contains(r#"UserID="UNKNOWN""#));
    }

    /// A failed access carries its outcome indicator; ITI TF-2 §3.20.7 and the
    /// `ihe-atna` writer leave the optional `EventOutcomeDescription` out.
    #[test]
    fn failed_outcome_indicator() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::MinorFailure,
        );
        assert!(xml(&e, None).contains(r#"EventOutcomeIndicator="4""#));
    }

    /// A client application acting for a named professional renders the
    /// person as the requesting participant and the application beside it.
    #[test]
    fn a_client_acting_for_a_professional_renders_two_source_participants() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.user_id = "scheduler-app".to_owned();
        e.actor = crate::system_log::event::ActorAuthentication {
            mode: Some(ActingMode::Client),
            assurance_level: Some(crate::config::auth::AssuranceLevel::High),
            assurance_value: Some("loa-high".to_owned()),
            professional_id: Some("prof-007".to_owned()),
        };
        let xml = xml(&e, None);
        assert!(
            xml.contains(r#"<ActiveParticipant UserID="prof-007" UserIsRequestor="true">"#),
            "{xml}"
        );
        assert!(
            xml.contains(
                r#"<ActiveParticipant UserID="scheduler-app" UserIsRequestor="false" NetworkAccessPointID="10.216.24.150""#
            ),
            "{xml}"
        );
        // PS3.15 §A.5.1 has no element for the assurance level.
        assert!(!xml.contains("loa-high"), "{xml}");
    }

    /// A professional identifier other than the subject is `UserID`, and the
    /// subject is `AlternativeUserID`.
    #[test]
    fn a_professional_identifier_renders_with_the_subject_as_alternative() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        e.actor.mode = Some(ActingMode::Person);
        e.actor.professional_id = Some("uzi-12345".to_owned());
        assert!(
            xml(&e, None).contains(r#"UserID="uzi-12345" AlternativeUserID="john doe""#),
            "{e:?}"
        );
    }
}
