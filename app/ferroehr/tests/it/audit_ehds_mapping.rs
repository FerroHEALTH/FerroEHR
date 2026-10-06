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

use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::{AuditContext, AuditMessage};

fn ctx() -> AuditContext {
    AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
    }
}

/// One clinical read by a named person, with everything the model can carry.
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
    e
}

/// The event as its two wire renderings: DICOM PS3.15 XML and the FHIR R4
/// `AuditEvent` the BALP feed carries.
fn renderings(event: &AuditEvent) -> (String, String) {
    let xml = AuditMessage::build(event, &ctx(), Some("patient-42"))
        .to_xml()
        .expect("the DICOM rendering builds");
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
        toml::from_str("[templates]\n\"Laboratory Report\" = [\"test-results\"]\n")
            .expect("the map parses");
    let map = map.compile().expect("the map compiles");
    let mut event = access_event();
    event.category = Some(map.classify(
        &ferroehr::system_log::categories::AccessedContent::Objects(vec![
            ferroehr::system_log::categories::ContentIds {
                template_id: Some("Laboratory Report".to_owned()),
                archetype_id: None,
                resource_kind: None,
            },
        ]),
    ));
    event.category_map_digest = Some(map.digest());
    let (xml, json) = renderings(&event);
    assert!(
        json.contains("categories of the data accessed") && json.contains("test-results"),
        "the FHIR rendering carries the category: {json}"
    );
    assert!(
        json.contains(&map.digest().to_string()),
        "the FHIR rendering names the map that classified the record: {json}"
    );
    assert!(
        !xml.contains("test-results") && !xml.contains("Laboratory Report"),
        "PS3.15 §A.5 has no element for the category: {xml}"
    );
}

/// (a) "identification of the healthcare provider or other individuals having
/// accessed" — the organisation, recorded and rendered in FHIR only.
///
/// Read beside (b), this element asks for the ORGANISATION on whose behalf the
/// access happened, distinct from the natural person (b) names. FHIR R4 takes
/// it as a second `agent` whose `who` references an `Organization` and which
/// declares no `type` — `agent.type` is 0..1, so no participation code has to
/// be invented (<https://hl7.org/fhir/R4/auditevent.html>).
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
        .find(|agent| agent["who"]["reference"] == "Organization/zh-noordwest")
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
/// count beside the capped set. The FHIR rendering carries one named entity
/// per origin; the DICOM Audit Message of PS3.15 §A.5 defines no element for
/// it, and a value appearing there would be invented.
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
        .filter(|e| {
            e["name"]
                .as_str()
                .is_some_and(|n| n.contains("origin of the served data"))
        })
        .filter_map(|e| e["what"]["identifier"]["value"].as_str())
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
/// requestor ("the security role that the user was acting under") and a
/// `RoleIDCode` per role on the DICOM source participant (PS3.15 §A.5.1 makes
/// the element 0..*).
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
        .filter_map(|r| r["text"].as_str())
        .collect();
    assert_eq!(roles, ["USER", "clinician"], "{requestor}");

    let (xml, _json) = renderings(&event);
    assert!(
        xml.contains(r#"csd-code="clinician" codeSystemName="urn:ferroehr:role""#),
        "the role must reach the DICOM source participant as a RoleIDCode: {xml}"
    );

    // No principal, no roles: the requestor carries no role element rather
    // than an empty one.
    let mut anonymous = access_event();
    anonymous.roles.clear();
    let agents = fhir_agents(&anonymous);
    assert!(
        agents[0]["role"].as_array().is_none_or(Vec::is_empty),
        "an unknown role set renders as absent: {}",
        agents[0]
    );
}
