// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use super::*;

/// The tokens read from the typed resource are those its JSON document
/// holds at the nodes the profile's matching reads, for every kind of
/// record: unclassified exchanges and classified EHDS access records,
/// with and without a patient (#6005).
#[test]
fn the_typed_resource_holds_the_documents_tokens() {
    use crate::system_log::categories::{AccessedContent, Classifier, QueryConstraints};
    use crate::system_log::message::AuditContext;
    use proptest::prelude::*;
    let ctx = AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "1f332a66-0000-0000-0000-000000000001".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
        ..AuditContext::default()
    };
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(2048));
    let strategy = (
        crate::access_log::stored_form::parts::tests::event(),
        any::<bool>(),
        proptest::option::of("[ -~]{1,12}"),
        proptest::option::of("[0-9.]{7,15}"),
    );
    runner
        .run(&strategy, |(mut event, classified, subject, address)| {
            event.client_ip = address;
            if classified {
                event.domain = crate::system_log::event::AccessDomain::Ehr;
                event.category = Some(Classifier::default().classify(
                    &AccessedContent::Query(QueryConstraints {
                        reaches_content: true,
                        ..QueryConstraints::default()
                    }),
                    false,
                ));
            }
            let subject = subject.as_deref();
            let resource = crate::system_log::fhir::to_resource(&event, &ctx, subject);
            let document = crate::system_log::fhir::to_fhir(&event, &ctx, subject);
            match (resource, document) {
                (Ok(resource), Ok(document)) => prop_assert_eq!(
                    resource_tokens(resource.as_resource(), &event),
                    document_tokens(&document, &event)
                ),
                (Err(_), Err(_)) => {}
                _ => prop_assert!(false, "both render or neither does"),
            }
            Ok(())
        })
        .expect("every record's tokens agree");
}

/// Every element names a distinct parameter, and the shared ones are the
/// code-system parameters and the audit source.
#[test]
fn every_element_names_its_own_parameter() {
    let parameters: BTreeSet<&str> = ELEMENTS.iter().map(|e| e.parameter()).collect();
    assert_eq!(parameters.len(), ELEMENTS.len());
    assert!(!parameters.contains(ADDRESS));
    let shared: Vec<&str> = ELEMENTS
        .iter()
        .filter(|e| e.shared())
        .map(|e| e.parameter())
        .collect();
    assert_eq!(
        shared,
        [
            "source.identifier",
            "entity-type",
            "entity-role",
            "type",
            "subtype"
        ]
    );
}

/// The tokens of a document are its nodes under each parameter, with the
/// system kept apart from its absence.
#[test]
fn a_document_yields_a_token_per_node() {
    let document = serde_json::json!({
        "type": {"system": "http://terminology.hl7.org/CodeSystem/audit-event-type", "code": "rest"},
        "subtype": [{"system": "urn:openehr:its-rest:operation", "code": "composition_get"}],
        "agent": [
            {"who": {"identifier": {"value": "alice"}}, "network": {"address": "10.0.0.9"}}
        ],
        "entity": [{
            "role": {"system": "http://terminology.hl7.org/CodeSystem/object-role", "code": "1"},
            "what": {"identifier": {"value": "patient-42"}}
        }]
    });
    let event = AuditEvent::new(
        crate::system_log::event::EventActionCode::Read,
        crate::system_log::event::ObjectClass::Composition,
        crate::system_log::event::EventOutcome::Success,
    );
    let tokens = document_tokens(&document, &event);
    let token = |parameter, system: Option<&str>, code: &str| SearchToken {
        parameter,
        system: system.map(str::to_owned),
        code: Some(code.to_owned()),
    };
    for expected in [
        token(
            "type",
            Some("http://terminology.hl7.org/CodeSystem/audit-event-type"),
            "rest",
        ),
        token(
            "subtype",
            Some("urn:openehr:its-rest:operation"),
            "composition_get",
        ),
        token("agent.identifier", None, "alice"),
        token("address", None, "10.0.0.9"),
        token("patient.identifier", None, "patient-42"),
        token("entity.identifier", None, "patient-42"),
        token(
            "entity-role",
            Some("http://terminology.hl7.org/CodeSystem/object-role"),
            "1",
        ),
    ] {
        assert!(tokens.contains(&expected), "{expected:?} in {tokens:?}");
    }
    assert_eq!(tokens.len(), 7, "{tokens:?}");
}

/// An `entity.identifier` carrying the event's request id or resource id
/// becomes the column token of its system, with no code; any other stays.
#[test]
fn a_request_id_and_a_resource_id_become_column_tokens() {
    use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

    let document = serde_json::json!({"entity": [
        {"what": {"identifier": {"value": "req-7"}}},
        {"what": {"identifier": {"system": "urn:openehr:object-id", "value": "8fa1::ferroehr::2"}}},
        {"what": {"identifier": {"value": "patient-42"}}}
    ]});
    let mut event = AuditEvent::new(
        EventActionCode::Create,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    event.request_id = Some("req-7".to_owned());
    event.object_id = Some("8fa1::ferroehr::2".to_owned());
    let tokens: Vec<SearchToken> = document_tokens(&document, &event).into_iter().collect();
    let token = |parameter, system: Option<&str>, code: Option<&str>| SearchToken {
        parameter,
        system: system.map(str::to_owned),
        code: code.map(str::to_owned),
    };
    assert_eq!(
        tokens,
        [
            token("entity.identifier", None, Some("patient-42")),
            token("entity.identifier=request_id", None, None),
            token(
                "entity.identifier=resource_id",
                Some("urn:openehr:object-id"),
                None
            ),
        ]
    );
    assert!(AuditElement::shares("entity.identifier=request_id"));
    assert!(AuditElement::shares("entity.identifier=resource_id"));
    assert!(!AuditElement::shares("entity.identifier"));
}
