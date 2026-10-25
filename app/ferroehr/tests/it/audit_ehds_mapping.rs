// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Annex II 3.2 logging elements, asserted against the access-event
//! model and against BOTH renderings the audit trail leaves the server in.
//!
//! Annex II 3.2 of Regulation (EU) 2025/327 lists five things the European
//! logging software component must record "on every access event or group of
//! events" (<https://eur-lex.europa.eu/eli/reg/2025/327/oj>). The published
//! mapping from each of them to a field lives on the audit page of the
//! documentation site; this module is what stops that page from becoming a
//! claim nobody checks.
//!
//! All five are recorded; the renderings differ. Where a rendering cannot
//! carry an element its FORMAT does not define, that absence is pinned as a
//! test which fails the day the format changes, so the page and the code move
//! together.
//!
//! No openEHR spec governs the read-side access log — our own
//! design/extension. openEHR specifies the write-side `AUDIT_DETAILS` only
//! (`docs/specs/openehr/RM/docs/common/` §`AUDIT_DETAILS` Class).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a panicking fixture is the intended shape (the \
              Rust Book ch11), as in `multimedia_s3`"
)]

use jiff::Timestamp;

use ferroehr::system_log::categories::{AccessedContent, Classifier, ContentIds};
use ferroehr::system_log::event::{
    AccessDomain, ActingMode, ActorAuthentication, AuditEvent, EventActionCode, EventOutcome,
    EventType, ObjectClass,
};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::{self, AuditContext};
use ferrohealth_smart::assurance::AssuranceLevel;
use secrecy::ExposeSecret as _;

fn ctx() -> AuditContext {
    AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
        ..AuditContext::default()
    }
}

/// One clinical read by a named person, with everything the model can carry,
/// classified (unclassified under the empty map) so it is an EHDS access
/// record.
fn access_event() -> AuditEvent {
    let mut e = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "dr.jansen".clone_into(&mut e.user_id);
    e.organisation = Some("zh-noordwest".to_owned());
    e.roles = vec!["USER".to_owned(), "clinician".to_owned()];
    e.origins = vec!["lab.example".to_owned(), "pacs.example".to_owned()];
    e.origin_count = Some(2);
    e.client_ip = Some("10.0.0.9".to_owned());
    e.object_id = Some("8fa1::ferroehr::1".to_owned());
    e.event_type = Some(EventType::RestOperation("composition_get"));
    e.token_id = Some("jti-1".to_owned());
    e.purpose = Some("TREAT".to_owned());
    e.legal_basis = Some("gdpr-9-2-h".to_owned());
    e.result_count = Some(1);
    e.timestamp = "2026-07-10T08:30:00Z".parse().expect("a fixed instant");
    e.issuer = Some("https://idp.example".to_owned());
    e.client_id = Some("ward-app".to_owned());
    e.category = Some(Classifier::default().classify(
        &AccessedContent::Objects(vec![ContentIds {
            template_id: Some("Vitals".to_owned()),
            archetype_id: None,
            resource_kind: None,
        }]),
        false,
    ));
    e
}

/// The `ihe-otherId` of `agent`: the professional identifier BALP 1.1.4
/// §3:5.7.5.4 maps the IUA `national_provider_identifier` to.
fn other_id(agent: &serde_json::Value) -> Option<&str> {
    agent["extension"]
        .as_array()?
        .iter()
        .find(|x| x["url"] == "https://profiles.ihe.net/ITI/BALP/StructureDefinition/ihe-otherId")?
        .get("valueIdentifier")?
        .get("value")?
        .as_str()
}

/// The event as its two wire renderings: DICOM PS3.15 XML and the FHIR R4
/// `AuditEvent` the BALP feed carries.
fn renderings(event: &AuditEvent) -> (String, String) {
    let xml = String::from_utf8(
        message::to_xml(event, &ctx(), Some("patient-42"))
            .expect("the DICOM rendering builds")
            .expose_secret()
            .to_vec(),
    )
    .expect("the DICOM rendering is UTF-8");
    let json = serde_json::to_string(
        &fhir::to_fhir(event, &ctx(), Some("patient-42")).expect("the FHIR rendering builds"),
    )
    .expect("the FHIR resource serializes");
    (xml, json)
}

/// The FHIR rendering's `agent` list, parsed.
fn fhir_agents(event: &AuditEvent) -> Vec<serde_json::Value> {
    let rendered =
        fhir::to_fhir(event, &ctx(), Some("patient-42")).expect("the FHIR rendering builds");
    rendered
        .get("agent")
        .and_then(serde_json::Value::as_array)
        .expect("the rendering carries agents")
        .clone()
}

/// (b) "identification of the specific natural person or persons having
/// accessed the personal electronic health data".
#[test]
fn element_b_the_person_who_accessed_is_recorded_and_rendered() {
    let event = access_event();
    assert_eq!(event.user_id, "dr.jansen");
    let (xml, json) = renderings(&event);
    assert!(
        xml.contains("dr.jansen"),
        "the accessing person must reach the DICOM ActiveParticipant: {xml}"
    );
    assert!(
        json.contains("dr.jansen"),
        "the accessing person must reach the FHIR agent: {json}"
    );
}

/// (b) for a client application acting for a named professional: the
/// professional is the requesting DICOM participant, and the application
/// stands beside it as a non-requesting one. In FHIR the token's subject is
/// the requesting `agent:user` with the professional's identifier as its
/// `ihe-otherId`, and the application its own agent (BALP 1.1.4 §3:5.7.5.4).
#[test]
fn element_b_the_professional_behind_a_client_is_recorded_and_rendered() {
    let mut event = access_event();
    "scheduler-app".clone_into(&mut event.user_id);
    event.client_id = Some("scheduler-app".to_owned());
    event.actor = ActorAuthentication {
        mode: Some(ActingMode::Client),
        assurance_level: Some(AssuranceLevel::Substantial),
        assurance_value: Some("loa-substantial".to_owned()),
        professional_id: Some("dr.jansen".to_owned()),
    };
    let (xml, _json) = renderings(&event);
    assert!(
        xml.contains(r#"UserID="dr.jansen" UserIsRequestor="true""#),
        "the professional is the requesting DICOM participant: {xml}"
    );
    assert!(
        xml.contains(r#"UserID="scheduler-app" UserIsRequestor="false""#),
        "the client application is a further DICOM participant: {xml}"
    );
    let agents = fhir_agents(&event);
    let who = |agent: &serde_json::Value| {
        agent
            .pointer("/who/identifier/value")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let person = agents
        .iter()
        .find(|a| other_id(a) == Some("dr.jansen"))
        .expect("the professional is named on a FHIR agent");
    assert_eq!(person.get("requestor"), Some(&serde_json::json!(true)));
    let client = agents
        .iter()
        .find(|a| {
            a.get("requestor") == Some(&serde_json::json!(false))
                && who(a).as_deref() == Some("scheduler-app")
        })
        .expect("the client application is a FHIR agent");
    assert_eq!(client.get("requestor"), Some(&serde_json::json!(false)));
}

/// Annex II 3.1: the authentication assurance level is recorded on the event.
/// The DICOM rendering carries none (PS3.15 §A.5.1 `ActiveParticipant` has no
/// element for it); the FHIR rendering carries the level as the BALP
/// `ihe-assuranceLevel` extension (BALP 1.1.4 §3:5.7.5.4, Regulation (EU) No
/// 910/2014 Art. 8(2)), never the raw claim value.
#[test]
fn the_assurance_level_is_recorded_and_reaches_the_fhir_rendering_as_a_level() {
    let mut event = access_event();
    event.actor = ActorAuthentication {
        mode: Some(ActingMode::Person),
        assurance_level: Some(AssuranceLevel::High),
        assurance_value: Some("urn:loa:assurance-high".to_owned()),
        professional_id: Some("dr.jansen".to_owned()),
    };
    assert_eq!(event.actor.assurance_level, Some(AssuranceLevel::High));
    let (xml, json) = renderings(&event);
    assert!(!xml.contains("assurance-high"), "{xml}");
    assert!(!json.contains("assurance-high"), "{json}");
    assert!(
        json.contains("ihe-assuranceLevel") && json.contains(r#""code":"high""#),
        "{json}"
    );
}

/// (d) "the time and date of access".
#[test]
fn element_d_the_time_of_access_is_recorded_and_rendered() {
    let event = access_event();
    let stamp: Timestamp = "2026-07-10T08:30:00Z".parse().expect("a fixed instant");
    assert_eq!(event.timestamp, stamp);
    let (xml, json) = renderings(&event);
    assert!(
        xml.contains("2026-07-10T08:30:00"),
        "the event time must reach the DICOM EventDateTime: {xml}"
    );
    assert!(
        json.contains("2026-07-10T08:30:00"),
        "the event time must reach the FHIR recorded element: {json}"
    );
}

/// (c) "the categories of data accessed" — the resource class and the
/// pseudonymisation domain stand beside the priority category.
///
/// The record carries the resource class and the pseudonymisation domain, so
/// "what kind of thing was read, and was it clinical or identifying" is
/// answerable per event; the EHDS priority category itself is asserted in
/// [`element_c_the_priority_category_is_recorded_and_rendered_in_fhir_only`].
#[test]
fn element_c_the_resource_class_and_domain_stand_in_for_the_category() {
    let event = access_event();
    assert_eq!(event.object, ObjectClass::Composition);
    assert_eq!(event.domain, AccessDomain::Ehr);
    assert_eq!(
        AccessDomain::of(ObjectClass::Demographic),
        AccessDomain::Demographic,
        "reading a party is recorded in the demographic domain, which is what \
         separates 'who saw the identity' from 'who read the record'"
    );
    let (xml, json) = renderings(&event);
    // The DICOM ParticipantObject type/role codes carry the class; the FHIR
    // entity does the same.
    assert!(
        xml.contains("ParticipantObjectIdentification"),
        "the object the event touched must be rendered: {xml}"
    );
    assert!(
        json.contains("entity"),
        "the FHIR rendering must carry the entity: {json}"
    );
}

/// (c) "the categories of data accessed" — the EHDS priority category the
/// `[audit.categories]` map classifies the access into (#3621), recorded on the
/// event and rendered as FHIR entity details; the DICOM rendering carries
/// nothing, and that half is pinned as the format limit it is (PS3.15 §A.5
/// defines no element for it,
/// <https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html>).
#[test]
fn element_c_the_priority_category_is_recorded_and_rendered_in_fhir_only() {
    let map: ferroehr::system_log::categories::CategoryMapConfig =
        toml::from_str("[templates]\n\"Laboratory Report\" = [\"Laboratory-Reports\"]\n")
            .expect("the map parses");
    let map = map.compile().expect("the map compiles");
    let mut event = access_event();
    event.category = Some(map.classify(
        &AccessedContent::Objects(vec![ContentIds {
            template_id: Some("Laboratory Report".to_owned()),
            archetype_id: None,
            resource_kind: None,
        }]),
        false,
    ));
    event.category_map_digest = Some(map.digest());
    let (xml, json) = renderings(&event);
    assert!(
        json.contains("ehds-categories") && json.contains("|Laboratory-Reports"),
        "the FHIR rendering carries the category: {json}"
    );
    assert!(
        json.contains(&map.digest().to_string()),
        "the FHIR rendering names the map that classified the record: {json}"
    );
    assert!(
        !xml.contains("Laboratory-Reports") && !xml.contains("Laboratory Report"),
        "PS3.15 §A.5 has no element for the category: {xml}"
    );
}

/// (a) "identification of the healthcare provider or other individuals having
/// accessed" — the organisation, recorded and rendered in FHIR only.
///
/// Read beside (b), this element asks for the ORGANISATION on whose behalf the
/// access happened, distinct from the natural person (b) names. FHIR R4 takes
/// it as a further `agent` whose `who` is a logical reference to an
/// `Organization` (`Reference.type` and `Reference.identifier`) and which
/// declares no `type` — `agent.type` is 0..1, so no participation code has to
/// be invented (<https://hl7.org/fhir/R4/auditevent.html>,
/// <https://hl7.org/fhir/R4/references.html>).
///
/// The DICOM rendering carries nothing, and that half is pinned as the format
/// limit it is: PS3.15 §A.5 gives `ActiveParticipant` no organisation
/// attribute, and `AuditEnterpriseSiteID` names the reporting source's site
/// rather than the caller's organisation
/// (<https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html>).
#[test]
fn element_a_the_accessing_organisation_is_recorded_and_rendered_in_fhir() {
    let event = access_event();
    assert_eq!(event.organisation.as_deref(), Some("zh-noordwest"));

    let agents = fhir_agents(&event);
    let organisation = agents
        .iter()
        .find(|agent| {
            agent["who"]["type"] == "Organization"
                && agent["who"]["identifier"]["value"] == "zh-noordwest"
        })
        .expect("the organisation reaches the FHIR rendering as an Organization agent");
    assert!(
        organisation.get("type").is_none(),
        "the organisation agent must claim no participation code: {organisation}"
    );
    assert_eq!(
        organisation["requestor"], false,
        "the natural person initiated the request, not the organisation"
    );

    let (xml, _) = renderings(&event);
    assert!(
        !xml.contains("zh-noordwest"),
        "the DICOM schema defines no organisation attribute — a value appearing \
         here means one was invented: {xml}"
    );
}

/// (e) "the origin or origins of data" — recorded (#3212).
///
/// The origin of the DATA is not the origin of the request: the record knows
/// the client address it was asked from, and it now also carries where the
/// content it served came from, read from the `FEEDER_AUDIT` provenance
/// openEHR stamps on content (RM common `master04` §Feeder Audit) as the
/// commit path derived it onto `version.origins`, with the true distinct
/// count beside the capped set. The FHIR rendering carries one `origin`
/// entity per origin, naming its system; the DICOM Audit Message of PS3.15
/// §A.5 defines no element for it, and a value appearing there would be
/// invented.
#[test]
fn element_e_the_origin_of_the_data_is_recorded_and_rendered_in_fhir() {
    let event = access_event();
    assert_eq!(event.origins, ["lab.example", "pacs.example"]);
    assert_eq!(event.origin_count, Some(2));
    let rendered =
        fhir::to_fhir(&event, &ctx(), Some("patient-42")).expect("the FHIR rendering builds");
    let entities = rendered["entity"].as_array().expect("entities");
    let origins: Vec<&str> = entities
        .iter()
        .filter(|e| e["name"] == "origin")
        .flat_map(|e| e["detail"].as_array().expect("origin details"))
        .filter(|detail| detail["type"] == "system-id")
        .filter_map(|detail| detail["valueString"].as_str())
        .collect();
    assert_eq!(origins, ["lab.example", "pacs.example"], "{rendered}");
    let (xml, _) = renderings(&event);
    assert!(
        !xml.contains("lab.example"),
        "the DICOM schema defines no origin element — a value appearing here means one was \
         invented: {xml}"
    );
    // The client address is present and is deliberately NOT the answer.
    assert_eq!(event.client_ip.as_deref(), Some("10.0.0.9"));
}

/// The declared purpose of use reaches the FHIR rendering, and only that one
/// could carry it.
///
/// FHIR R4 `AuditEvent` defines `agent.purposeOfUse` as a 0..* `CodeableConcept`
/// on the agent (<https://hl7.org/fhir/R4/auditevent.html>), so it lands on the
/// requesting person's agent. The code is from the vocabulary the deployment
/// agrees with its callers, which has no published code system, so the coding
/// carries the code alone — `Coding.system` is optional
/// (<https://hl7.org/fhir/R4/datatypes.html>).
///
/// The DICOM Audit Message schema of PS3.15 §A.5 defines no purpose element at
/// all: `EventIdentification`, `ActiveParticipant`, `AuditSourceIdentification`
/// and `ParticipantObjectIdentification`, and none of them has one
/// (<https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html>).
/// That half is a limit of the format rather than of this code, and stays
/// pinned as one.
#[test]
fn the_declared_purpose_reaches_the_fhir_rendering_only() {
    let event = access_event();
    assert_eq!(event.purpose.as_deref(), Some("TREAT"));

    let agents = fhir_agents(&event);
    let person = agents
        .iter()
        .find(|agent| agent["who"]["identifier"]["value"] == "dr.jansen")
        .expect("the accessing person's agent");
    let coding = &person["purposeOfUse"][0]["coding"][0];
    assert_eq!(
        coding["code"], "TREAT",
        "the declared purpose must reach agent.purposeOfUse: {person}"
    );
    assert!(
        coding.get("system").is_none(),
        "a deployment-agreed code must not be attributed to a code system it \
         does not come from: {coding}"
    );

    let (xml, _) = renderings(&event);
    assert!(
        !xml.contains("TREAT") && !xml.contains("PurposeOfUse"),
        "the DICOM schema defines no purpose element — a value appearing here \
         means one was invented: {xml}"
    );
}

/// The trail separates the two pseudonymisation domains, which is what makes
/// "who read this person's identifying data" answerable on its own.
#[test]
fn every_resource_class_answers_which_domain_it_touched() {
    for (class, domain) in [
        (ObjectClass::Ehr, AccessDomain::Ehr),
        (ObjectClass::Composition, AccessDomain::Ehr),
        (ObjectClass::Contribution, AccessDomain::Ehr),
        (ObjectClass::Directory, AccessDomain::Ehr),
        (ObjectClass::Query, AccessDomain::Ehr),
        (ObjectClass::Extract, AccessDomain::Ehr),
        (ObjectClass::Demographic, AccessDomain::Demographic),
        (ObjectClass::Template, AccessDomain::System),
        (ObjectClass::ApplicationActivity, AccessDomain::System),
        (ObjectClass::Authentication, AccessDomain::System),
    ] {
        assert_eq!(
            AccessDomain::of(class),
            domain,
            "{class:?} must answer which pseudonymisation domain it touched"
        );
    }
}

// ── NEN 7513: the role under which the person accessed ───────────────────────

/// NEN 7513 lists the role or authority under which the person accessed the
/// record among what a logged event has to contain. The record carries the
/// caller's roles and both renderings show them: FHIR `agent.role` on the
/// requestor ("the security role that the user was acting under"), and a
/// `RoleIDCode` per role on the DICOM source participant (PS3.15 §A.5.1 makes
/// the element 0..*). An acting mode the deployment did not establish adds no
/// role of its own.
#[test]
fn nen_7513_the_actor_roles_are_recorded_and_rendered_in_both_formats() {
    let event = access_event();
    assert_eq!(event.roles, ["USER", "clinician"]);
    let agents = fhir_agents(&event);
    let requestor = agents
        .iter()
        .find(|a| a["requestor"] == true && a["who"]["identifier"]["value"] == "dr.jansen")
        .expect("the requesting person is an agent");
    let roles: Vec<&str> = requestor["role"]
        .as_array()
        .expect("agent.role is a list")
        .iter()
        .filter_map(|r| r["coding"][0]["code"].as_str())
        .collect();
    assert_eq!(roles, ["USER", "clinician"], "{requestor}");

    let (xml, _json) = renderings(&event);
    assert!(
        xml.contains(r#"csd-code="clinician" codeSystemName="urn:ferroehr:role""#),
        "the role must reach the DICOM source participant as a RoleIDCode: {xml}"
    );

    // No principal, no roles: the requestor carries no role element rather
    // than an empty one, and no invented role.
    let mut anonymous = access_event();
    anonymous.roles.clear();
    let agents = fhir_agents(&anonymous);
    let requestor = agents
        .iter()
        .find(|a| a["requestor"] == true)
        .expect("the requesting person is an agent");
    assert!(
        requestor["role"].as_array().is_none_or(Vec::is_empty),
        "an unknown role set renders as absent: {requestor}"
    );
}

/// EHDS Art. 11(5): an emergency access "shall be logged in a clear and
/// understandable format and shall be easily accessible for the data subject"
/// (`docs/law/eu/ehds/text.html Art. 11(5)`). The FHIR rendering carries the
/// mark as the `ehds-logging` entity that states it in plain words, with the
/// purpose that marked it; the DICOM schema has no purpose element, so the
/// mark is absent there and that absence is pinned.
#[test]
fn art_11_5_the_emergency_mark_reaches_the_fhir_rendering_only() {
    let mut event = access_event();
    event.purpose = Some("ETREAT".to_owned());
    event.emergency_access = true;
    let etreat = ehds_logging::record::Purpose {
        system: None,
        code: "ETREAT".to_owned(),
    };
    let marking = AuditContext {
        recording: fhir::Recording {
            emergency: ehds_logging::emergency::EmergencyPurposes::declare(&[etreat])
                .expect("one emergency purpose"),
            ..fhir::Recording::default()
        },
        ..ctx()
    };

    let rendered =
        fhir::to_fhir(&event, &marking, Some("patient-42")).expect("the FHIR rendering builds");
    let mark = rendered["entity"]
        .as_array()
        .expect("entities")
        .iter()
        .find(|entity| entity["name"] == ehds_logging::balp::EMERGENCY_ENTITY)
        .expect("the emergency mark is an entity");
    assert_eq!(
        mark["description"],
        ehds_logging::balp::EMERGENCY_DESCRIPTION
    );
    assert!(
        mark["detail"]
            .as_array()
            .expect("details")
            .iter()
            .any(|d| d["type"] == "ehds-emergency-purpose" && d["valueString"] == "ETREAT"),
        "{mark}"
    );

    let (xml, _) = renderings(&event);
    assert!(
        !xml.contains("ETREAT") && !xml.contains("PurposeOfUse") && !xml.contains("Emergency"),
        "the DICOM schema defines no purpose element: {xml}"
    );
}
