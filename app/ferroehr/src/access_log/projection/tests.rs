// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projections of a stored entry against the renderings of the access it
//! records (#5127).

use secrecy::ExposeSecret as _;

mod mapping;

use super::Projection;
use crate::access_log::entry::{entry_of, horizon_retention};
use crate::access_log::store::canonical_form;
use crate::system_log::access_binding::tests::{organisation, representative, system};
use crate::system_log::binding::{BindingCode, BindingRecord, Identified, PartyTriple, Summary};
use crate::system_log::categories::{
    AccessedContent, CategoryMapConfig, ContentIds, QueryConstraints,
};
use crate::system_log::config::AuditConfig;
use crate::system_log::event::{
    AccessDecision, AccessDomain, ActingMode, AuditEvent, Coding, EntryScope, EventActionCode,
    EventOutcome, EventType, ObjectClass, QueryNaming, TokenAttributes,
};
use crate::system_log::render::{CompiledPolicy, RenderPolicy};
use ferrohealth_smart::assurance::AssuranceLevel;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

pub(super) const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";
pub(super) const VERSION: &str = "8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1";
pub(super) const TEMPLATE: &str = "Lab Report";

/// The policy the projections are written under: an audit source with its
/// site and host, a category map that maps one template, and one emergency
/// purpose.
pub(super) fn policy() -> CompiledPolicy {
    let config: AuditConfig = toml::from_str(
        r#"
        source_id = "cdr.example.org"
        enterprise_site_id = "site-1"
        server_host = "10.42.23.77"
        emergency_purpose_codes = ["ETREAT"]
        "#,
    )
    .expect("an audit configuration");
    let categories: CategoryMapConfig = toml::from_str(
        r#"
        [templates]
        "Lab Report" = ["Laboratory-Reports"]
        "#,
    )
    .expect("a category map");
    RenderPolicy::of(&config, &categories)
        .compile()
        .expect("a policy")
}

/// A delegated clinician's granted, classified read of one composition
/// version, with every fact the record carries.
pub(super) fn classified_read(policy: &CompiledPolicy) -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::rest("composition_get"));
    event.user_id = "alice".to_owned();
    event.issuer = Some("https://login.example.org".to_owned());
    event.client_id = Some("ward-app".to_owned());
    event.token_id = Some("jti-7f3e".to_owned());
    event.roles = vec!["clinician".to_owned()];
    event.client_ip = Some("10.216.24.150".to_owned());
    event.ehr_id = Some(EHR.to_owned());
    event.subject_namespace = Some("urn:oid:2.16.840.1.113883.2.4.6.3".to_owned());
    event.object_id = Some(VERSION.to_owned());
    event.purpose = Some("TREAT".to_owned());
    event.legal_basis = Some("GDPR-9-2-h".to_owned());
    event.result_count = Some(1);
    event.origins = vec!["cdr.example.org".to_owned(), "lab.example.org".to_owned()];
    event.origin_count = Some(3);
    event.category_map_digest = Some(policy.categories.digest());
    event.category = Some(policy.categories.classify(
        &AccessedContent::Objects(vec![ContentIds {
            template_id: Some(TEMPLATE.to_owned()),
            archetype_id: Some("openEHR-EHR-COMPOSITION.report.v1".to_owned()),
            resource_kind: None,
        }]),
        false,
    ));
    event.actor.mode = Some(ActingMode::Delegated);
    event.actor.assurance_level = Some(AssuranceLevel::High);
    event.actor.assurance_value = Some("http://eidas.europa.eu/LoA/high".to_owned());
    event.actor.professional_id = Some("NPI-1".to_owned());
    event.request_id = Some("01JC2Q7N3X".to_owned());
    event.token = Some(TokenAttributes {
        purposes: vec![Coding {
            system: Some("http://terminology.hl7.org/CodeSystem/v3-ActReason".to_owned()),
            code: "ETREAT".to_owned(),
        }],
        roles: vec![Coding {
            system: Some("urn:oid:2.16.840.1.113883.2.4.15.111".to_owned()),
            code: "01.015".to_owned(),
        }],
    });
    event.emergency_access = true;
    event.permit = Some("permit-42".to_owned());
    event.binding = Some(BindingRecord::bound(
        PartyTriple {
            namespace: "demographic".to_owned(),
            class: "PERSON".to_owned(),
            id: "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5".to_owned(),
        },
        false,
        vec!["c1a9e0e4-0d8f-4d9c-9b51-3f6a2f1b7c11".to_owned()],
        Some(Identified {
            issuer: "urn:oid:2.16.528.1.1007.3.3".to_owned(),
            id: "Example Hospital".to_owned(),
        }),
        true,
    ));
    event.timestamp = "2026-11-13T10:15:30.123456Z".parse().expect("an instant");
    event
}

/// The events the parity test projects: the classified read, and the
/// records of every other shape an entry is written for.
fn events(policy: &CompiledPolicy) -> Vec<(&'static str, AuditEvent, Option<&'static str>)> {
    let read = classified_read(policy);
    let mut refused = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::MinorFailure,
    );
    refused.decision = AccessDecision::Refused;
    refused.entry = EntryScope::Refusal;
    refused.ehr_id = Some(EHR.to_owned());
    refused.request_id = Some("01JC2Q7N3Y".to_owned());
    refused.binding = Some(BindingRecord::refused(
        BindingCode::RefusedUnbound,
        std::collections::BTreeMap::from([("reason".to_owned(), vec!["unbound".to_owned()])]),
    ));
    refused.summary = Some(Summary {
        count: 2,
        ehr_ids: vec![EHR.to_owned()],
    });
    let mut query = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Query,
        EventOutcome::Success,
    );
    query.event_type = Some(EventType::rest("query_execute_ad_hoc"));
    query.user_id = "bob".to_owned();
    query.issuer = Some("https://login.example.org".to_owned());
    query.ehr_id = Some(EHR.to_owned());
    query.object_id = Some("SELECT c FROM EHR e CONTAINS COMPOSITION c".to_owned());
    query.query = Some(QueryNaming::of(
        "AQL",
        "SELECT c FROM EHR e CONTAINS COMPOSITION c",
        None,
    ));
    query.result_count = Some(4);
    query.category_map_digest = Some(policy.categories.digest());
    query.category = Some(policy.categories.classify(
        &AccessedContent::Query(QueryConstraints {
            reaches_content: true,
            ..QueryConstraints::default()
        }),
        false,
    ));
    query.binding = Some(system());
    let mut failed = AuditEvent::new(
        EventActionCode::Update,
        ObjectClass::Composition,
        EventOutcome::SeriousFailure,
    );
    failed.event_type = Some(EventType::rest("composition_update"));
    failed.user_id = "carol".to_owned();
    failed.issuer = Some("https://federation.example.org".to_owned());
    failed.ehr_id = Some("not-an-ehr-id".to_owned());
    failed.object_id = Some(VERSION.to_owned());
    failed.domain = AccessDomain::Ehr;
    failed.subject_lookup_failed = true;
    failed.binding = Some(organisation());
    let mut represented = classified_read(policy);
    represented.binding = Some(representative());
    let mut relayed = classified_read(policy);
    relayed.binding = Some(organisation());
    relayed.actor.mode = Some(ActingMode::External);
    vec![
        ("classified read", read, Some("subject-1")),
        ("refusal", refused, None),
        ("query", query, Some("subject-1")),
        ("failed update", failed, None),
        ("representative", represented, Some("subject-9")),
        ("relayed organisation", relayed, Some("subject-1")),
    ]
}

/// The entry of `event`, written as the store writes it and read back from
/// its canonical form, as every projection reads it.
pub(super) fn stored_entry(event: &AuditEvent, subject: Option<&str>) -> AccessLogEntry {
    let entry = entry_of(
        event,
        subject,
        "cdr.example.org",
        horizon_retention(Some(jiff::civil::date(2031, 11, 13))),
    )
    .expect("an entry");
    let stored = canonical_form(&entry).expect("a canonical form");
    let mut stored: serde_json::Value = serde_json::from_str(&stored).expect("a JSON object");
    stored["link"] = serde_json::to_value(&entry.link).expect("a link");
    serde_json::from_value(stored).expect("the entry reads back")
}

/// Each projection of a stored entry is the document the access it records
/// renders today: the BALP `AuditEvent`, the DICOM message and the EHDS
/// access record lose nothing by being written from the log.
#[test]
fn every_projection_is_the_rendering_of_the_access() {
    let policy = policy();
    let projection = Projection {
        context: &policy.context,
        categories: &policy.categories,
    };
    for (name, event, subject) in events(&policy) {
        let entry = stored_entry(&event, subject);
        assert_eq!(
            projection.audit_event(&entry, 1).expect("a projection"),
            crate::system_log::fhir::to_fhir(&event, &policy.context, subject)
                .expect("a rendering"),
            "{name}: the BALP AuditEvent"
        );
        let projected = projection.audit_message(&entry, 1).expect("a message");
        let rendered = crate::system_log::message::to_xml(&event, &policy.context, subject)
            .expect("a message");
        assert_eq!(
            projected.expose_secret(),
            rendered.expose_secret(),
            "{name}: the DICOM message"
        );
        let projected = projection.access_record(&entry, 1).expect("a record");
        let rendered = crate::system_log::fhir::access_record(&event, &policy.context, subject)
            .expect("a record");
        assert_eq!(
            projected.map(|record| format!("{record:?}")),
            rendered.map(|record| format!("{record:?}")),
            "{name}: the EHDS access record"
        );
    }
}

/// An entry a later map would classify otherwise is refused rather than
/// projected with categories it never had.
#[test]
fn an_entry_of_another_map_is_not_projected() {
    let policy = policy();
    let entry = stored_entry(&classified_read(&policy), Some("subject-1"));
    let other = RenderPolicy::default().compile().expect("a policy");
    let projection = Projection {
        context: &other.context,
        categories: &other.categories,
    };
    assert!(projection.audit_event(&entry, 1).is_err());
}

/// An entry with no FerroEHR access details, such as one imported from
/// another system, has no audit record to project.
#[test]
fn an_entry_without_details_is_not_projected() {
    let policy = policy();
    let mut entry = stored_entry(&classified_read(&policy), Some("subject-1"));
    entry.other_details = None;
    let projection = Projection {
        context: &policy.context,
        categories: &policy.categories,
    };
    assert!(matches!(
        projection.audit_event(&entry, 1),
        Err(crate::system_log::AuditError::Recorded(
            crate::access_log::recorded::RecordedError::NoDetails
        ))
    ));
}
