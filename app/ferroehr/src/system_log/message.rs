// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DICOM Audit Message model (DICOM PS3.15 §A.5) + a `quick-xml` serializer.
//!
//! This is the **DICOM audit schema**, not openEHR ITS-XML — it lives in its own
//! module and shares nothing with `openehr-its`. Plain structs mirror the schema
//! elements (`EventIdentification`, `ActiveParticipant`,
//! `AuditSourceIdentification`, `ParticipantObjectIdentification`; DICOM PS3.15
//! §A.5); [`AuditMessage::to_xml`] renders canonical (indented) XML with
//! `quick-xml`, which escapes all attribute/text values. The golden vector
//! snapshotted in the tests is a PS3.15 §A.5 EHR-create success record.
//!
//! The record's EHDS priority categories ([`AuditEvent::category`]) have no
//! element in the PS3.15 §A.5 schema, so this rendering omits them; the local
//! store and the FHIR `AuditEvent` carry them.
//!
//! The authentication assurance level has no element either: PS3.15 §A.5.1
//! gives `ActiveParticipant` no assurance attribute, so only the local store
//! carries it. The natural person behind a client application is a second
//! `ActiveParticipant`.
//!
//! The emergency-access mark ([`AuditEvent::emergency_access`], EHDS
//! Art. 11(5)) has no element either: the schema defines no purpose of use in
//! any of its four element groups, so the local store and the FHIR
//! `AuditEvent` carry the mark and this rendering does not.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::writer::Writer;

use crate::system_log::event::{ActingMode, AuditEvent};

use crate::system_log::AuditError;
use crate::system_log::codes::{
    self, AtnaAction, AtnaEventType, AtnaObject, AtnaOutcome, Code, NETWORK_ACCESS_POINT_IP,
    OBJECT_ROLE_PATIENT, OBJECT_ROLE_QUERY, OBJECT_TYPE_PERSON, OBJECT_TYPE_SYSTEM,
};

/// The server-side identity shared by every emitted record (the destination
/// node + audit source). Built once by the sender from
/// [`super::config::AuditConfig`].
#[derive(Debug, Clone)]
pub struct AuditContext {
    /// `AuditSourceID` and the destination `ActiveParticipant.UserID`.
    pub source_id: String,
    /// `AuditEnterpriseSiteID`.
    pub enterprise_site_id: String,
    /// This node's `NetworkAccessPointID` (destination participant).
    pub server_ip: String,
    /// Fill for empty mandatory fields (default `UNKNOWN`).
    pub value_if_missing: String,
}

/// A DICOM `ActiveParticipant` (a human/system actor in the event).
#[derive(Debug, Clone)]
struct ActiveParticipant {
    user_id: String,
    /// `AlternativeUserID` (PS3.15 §A.5.1): the authentication system's
    /// identifier when `UserID` names the person by another one.
    alternative_user_id: Option<String>,
    user_is_requestor: bool,
    network_access_point_id: Option<String>,
    /// The participation `RoleIDCode`; `None` for a person who reached the
    /// server through a client application, which carries the code itself.
    role: Option<Code>,
    /// The security roles the person acted under, each a further `RoleIDCode`
    /// (PS3.15 §A.5.1 makes the element 0..*), coded in the deployment's own
    /// role vocabulary (#3239).
    roles: Vec<String>,
}

/// A DICOM `ParticipantObjectIdentification` (a data object the event touched).
#[derive(Debug, Clone)]
struct ParticipantObject {
    id: String,
    type_code: &'static str,
    type_code_role: Option<&'static str>,
    data_life_cycle: Option<&'static str>,
    id_type_code: Code,
}

/// A DICOM Audit Message (PS3.15 §A.5) ready to serialize.
#[derive(Debug, Clone)]
pub struct AuditMessage {
    action: char,
    event_datetime: String,
    outcome: i32,
    event_id: Code,
    event_type: Option<Code>,
    outcome_description: &'static str,
    participants: Vec<ActiveParticipant>,
    enterprise_site_id: String,
    source_id: String,
    source_type: Code,
    objects: Vec<ParticipantObject>,
}

impl AuditMessage {
    /// Build a DICOM Audit Message from a resolved [`AuditEvent`], the server
    /// [`AuditContext`], and the optionally-resolved patient subject id
    /// (`None` → `ctx.value_if_missing`).
    #[must_use]
    pub fn build(event: &AuditEvent, ctx: &AuditContext, subject: Option<&str>) -> Self {
        let missing = ctx.value_if_missing.as_str();
        let (event_code, event_text) = event.object.event_id(event.action);
        let event_id = Code {
            csd_code: event_code,
            code_system: codes::DCM,
            original_text: event_text,
        };
        let event_type = event.event_type.as_ref().map(AtnaEventType::code);

        let client_ip = event
            .client_ip
            .clone()
            .unwrap_or_else(|| missing.to_owned());
        let user_id = if event.user_id.is_empty() {
            missing.to_owned()
        } else {
            event.user_id.clone()
        };

        // The accessing organisation, the declared purpose of use and the
        // authentication assurance level stay out of this rendering: PS3.15
        // §A.5 gives `ActiveParticipant` no organisation or assurance
        // attribute and defines no purpose element anywhere, and
        // `AuditEnterpriseSiteID` names the REPORTING source's site rather
        // than the caller's organisation
        // (https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html).
        let mut participants = source_participants(event, user_id, client_ip);
        // Destination (this server).
        participants.push(ActiveParticipant {
            user_id: nonempty(&ctx.source_id, missing),
            alternative_user_id: None,
            user_is_requestor: false,
            network_access_point_id: Some(nonempty(&ctx.server_ip, missing)),
            role: Some(codes::ROLE_DESTINATION),
            roles: Vec::new(),
        });

        let objects = build_objects(event, subject, missing);

        Self {
            action: event.action.as_char(),
            event_datetime: event.timestamp.to_string(),
            outcome: event.outcome.as_i32(),
            event_id,
            event_type,
            outcome_description: event.outcome.description(),
            participants,
            enterprise_site_id: nonempty(&ctx.enterprise_site_id, missing),
            source_id: nonempty(&ctx.source_id, missing),
            source_type: codes::SOURCE_TYPE_APPLICATION_SERVER,
            objects,
        }
    }

    /// Serialize to indented canonical XML.
    ///
    /// # Errors
    /// [`AuditError::Xml`] if the writer fails (only on an I/O fault of the
    /// in-memory buffer, which does not occur in practice).
    pub fn to_xml(&self) -> Result<String, AuditError> {
        let mut w = Writer::new_with_indent(Vec::new(), b' ', 2);
        w.write_event(Event::Start(BytesStart::new("AuditMessage")))?;

        let mut ev = BytesStart::new("EventIdentification");
        ev.push_attribute(("EventActionCode", self.action.to_string().as_str()));
        ev.push_attribute(("EventDateTime", self.event_datetime.as_str()));
        ev.push_attribute(("EventOutcomeIndicator", self.outcome.to_string().as_str()));
        w.write_event(Event::Start(ev))?;
        write_code(&mut w, "EventID", &self.event_id)?;
        // EventTypeCode follows EventID inside EventIdentification
        // (DICOM PS3.15 §A.5 message schema element order).
        if let Some(event_type) = &self.event_type {
            write_code(&mut w, "EventTypeCode", event_type)?;
        }
        w.write_event(Event::Start(BytesStart::new("EventOutcomeDescription")))?;
        w.write_event(Event::Text(BytesText::new(self.outcome_description)))?;
        w.write_event(Event::End(BytesEnd::new("EventOutcomeDescription")))?;
        w.write_event(Event::End(BytesEnd::new("EventIdentification")))?;

        for p in &self.participants {
            let mut ap = BytesStart::new("ActiveParticipant");
            ap.push_attribute(("UserID", p.user_id.as_str()));
            if let Some(alternative) = &p.alternative_user_id {
                ap.push_attribute(("AlternativeUserID", alternative.as_str()));
            }
            ap.push_attribute(("UserIsRequestor", bool_str(p.user_is_requestor)));
            if let Some(address) = &p.network_access_point_id {
                ap.push_attribute(("NetworkAccessPointID", address.as_str()));
                ap.push_attribute(("NetworkAccessPointTypeCode", NETWORK_ACCESS_POINT_IP));
            }
            w.write_event(Event::Start(ap))?;
            if let Some(role) = &p.role {
                write_code(&mut w, "RoleIDCode", role)?;
            }
            for role in &p.roles {
                let mut c = BytesStart::new("RoleIDCode");
                c.push_attribute(("csd-code", role.as_str()));
                c.push_attribute(("codeSystemName", ROLE_VOCABULARY));
                c.push_attribute(("originalText", role.as_str()));
                w.write_event(Event::Empty(c))?;
            }
            w.write_event(Event::End(BytesEnd::new("ActiveParticipant")))?;
        }

        let mut src = BytesStart::new("AuditSourceIdentification");
        src.push_attribute(("AuditEnterpriseSiteID", self.enterprise_site_id.as_str()));
        src.push_attribute(("AuditSourceID", self.source_id.as_str()));
        w.write_event(Event::Start(src))?;
        write_code(&mut w, "AuditSourceTypeCode", &self.source_type)?;
        w.write_event(Event::End(BytesEnd::new("AuditSourceIdentification")))?;

        for obj in &self.objects {
            let mut po = BytesStart::new("ParticipantObjectIdentification");
            po.push_attribute(("ParticipantObjectID", obj.id.as_str()));
            po.push_attribute(("ParticipantObjectTypeCode", obj.type_code));
            if let Some(role) = obj.type_code_role {
                po.push_attribute(("ParticipantObjectTypeCodeRole", role));
            }
            if let Some(dlc) = obj.data_life_cycle {
                po.push_attribute(("ParticipantObjectDataLifeCycle", dlc));
            }
            w.write_event(Event::Start(po))?;
            write_code(&mut w, "ParticipantObjectIDTypeCode", &obj.id_type_code)?;
            w.write_event(Event::End(BytesEnd::new("ParticipantObjectIdentification")))?;
        }

        w.write_event(Event::End(BytesEnd::new("AuditMessage")))?;
        String::from_utf8(w.into_inner()).map_err(|e| AuditError::Xml(Box::new(e)))
    }
}

/// The requesting side's `ActiveParticipant`s (PS3.15 §A.5.1).
///
/// A client application acting for a named professional yields two: the
/// person as requestor with the security roles, and the application at the
/// network address with the source role. A professional identifier other than
/// the token subject becomes `UserID`, the subject `AlternativeUserID`.
fn source_participants(
    event: &AuditEvent,
    user_id: String,
    client_ip: String,
) -> Vec<ActiveParticipant> {
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
                user_is_requestor: event.user_is_requestor,
                network_access_point_id: None,
                role: None,
                roles: event.roles.clone(),
            },
            ActiveParticipant {
                user_id,
                alternative_user_id: None,
                user_is_requestor: false,
                network_access_point_id: Some(client_ip),
                role: Some(codes::ROLE_SOURCE),
                roles: Vec::new(),
            },
        ];
    }
    let (user_id, alternative_user_id) = match professional {
        Some(person) if person != user_id => (person.to_owned(), Some(user_id)),
        _ => (user_id, None),
    };
    vec![ActiveParticipant {
        user_id,
        alternative_user_id,
        user_is_requestor: event.user_is_requestor,
        network_access_point_id: Some(client_ip),
        role: Some(codes::ROLE_SOURCE),
        roles: event.roles.clone(),
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
        objects.push(ParticipantObject {
            id: event
                .object_id
                .clone()
                .unwrap_or_else(|| missing.to_owned()),
            type_code: OBJECT_TYPE_SYSTEM,
            type_code_role: Some(OBJECT_ROLE_QUERY),
            data_life_cycle: None,
            id_type_code: codes::OBJ_ID_SEARCH_CRITERIA,
        });
        return objects;
    }

    if object_class.is_patient_centric() {
        // The patient (EHR subject) participant.
        objects.push(ParticipantObject {
            id: subject.map_or_else(|| missing.to_owned(), str::to_owned),
            type_code: OBJECT_TYPE_PERSON,
            type_code_role: Some(OBJECT_ROLE_PATIENT),
            data_life_cycle: Some(event.action.data_life_cycle()),
            id_type_code: codes::OBJ_ID_PATIENT_NUMBER,
        });
    }

    if object_class.has_object_uri() {
        // Patient-centric classes already carry the Patient-Number object and
        // add the URI object when the id is known; non-patient classes always
        // carry it, filled with `value_if_missing` when unknown, so no required
        // element is absent (DICOM PS3.15 §A.5).
        let id = event.object_id.clone();
        if let Some(id) = id {
            objects.push(uri_object(id, event));
        } else if !object_class.is_patient_centric() {
            objects.push(uri_object(missing.to_owned(), event));
        }
    }

    objects
}

fn uri_object(id: String, event: &AuditEvent) -> ParticipantObject {
    ParticipantObject {
        id,
        type_code: OBJECT_TYPE_SYSTEM,
        type_code_role: None,
        data_life_cycle: Some(event.action.data_life_cycle()),
        id_type_code: codes::OBJ_ID_URI,
    }
}

fn write_code<W: std::io::Write>(
    w: &mut Writer<W>,
    element: &str,
    code: &Code,
) -> Result<(), AuditError> {
    let mut c = BytesStart::new(element);
    c.push_attribute(("csd-code", code.csd_code));
    c.push_attribute(("codeSystemName", code.code_system));
    c.push_attribute(("originalText", code.original_text));
    w.write_event(Event::Empty(c))?;
    Ok(())
}

/// The `codeSystemName` of the deployment's own role vocabulary on a
/// `RoleIDCode`: the RBAC roles a caller held, which no published code system
/// names.
const ROLE_VOCABULARY: &str = "urn:ferroehr:role";

const fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
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
    use crate::system_log::event::{EventActionCode, EventOutcome, EventType, ObjectClass};
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
        let msg = AuditMessage::build(&e, &ctx(), Some("patient-42"));
        let xml = msg.to_xml().expect("xml");
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
        let msg = AuditMessage::build(&e, &ctx(), Some("patient-42"));
        let xml = msg.to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), Some("patient-42"))
            .to_xml()
            .expect("xml");
        assert!(xml.contains(r#"EventActionCode="U""#));
        assert!(xml.contains(r#"ParticipantObjectDataLifeCycle="3""#));
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
        let xml = AuditMessage::build(&e, &ctx(), Some("patient-42"))
            .to_xml()
            .expect("xml");
        assert!(xml.contains(r#"EventActionCode="D""#));
        assert!(xml.contains(r#"ParticipantObjectDataLifeCycle="14""#));
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
        assert!(xml.contains(r#"ParticipantObjectID="UNKNOWN""#));
    }

    #[test]
    fn demographic_read() {
        let mut e = event(
            EventActionCode::Read,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        e.object_id = Some("party-77".to_owned());
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), Some("patient-42"))
            .to_xml()
            .expect("xml");
        assert!(xml.contains(r#"csd-code="110106""#));
        assert!(xml.contains(r#"originalText="Export""#));
        assert!(xml.contains(r#"originalText="Patient Number""#));
        assert!(xml.contains(r#"originalText="URI""#));
        assert!(xml.contains(r#"ParticipantObjectID="patient-42""#));
        assert!(xml.contains(r#"ParticipantObjectID="extract::ferroehr::1""#));

        // The inbound direction resolves to Import (110107).
        let mut imp = event(
            EventActionCode::Create,
            ObjectClass::Extract,
            EventOutcome::Success,
        );
        imp.object_id = Some("extract::ferroehr::2".to_owned());
        let xml = AuditMessage::build(&imp, &ctx(), Some("patient-42"))
            .to_xml()
            .expect("xml");
        assert!(xml.contains(r#"csd-code="110107""#));
        assert!(xml.contains(r#"originalText="Import""#));
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), Some("patient-42"))
            .to_xml()
            .expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
        assert!(xml.contains(r#"ParticipantObjectID="UNKNOWN""#));
        assert!(xml.contains(r#"UserID="UNKNOWN""#));
    }

    #[test]
    fn failed_outcome_description() {
        let e = event(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::MinorFailure,
        );
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
        assert!(xml.contains(r#"EventOutcomeIndicator="4""#));
        assert!(xml.contains("Operation failed"));
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
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
        let xml = AuditMessage::build(&e, &ctx(), None).to_xml().expect("xml");
        assert!(
            xml.contains(r#"UserID="uzi-12345" AlternativeUserID="john doe""#),
            "{xml}"
        );
    }
}
