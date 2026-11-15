// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-94 subscription the PMIR Patient Identity Consumer keeps at the
//! Registry (#5693, PMIR 1.6.0 §2:3.94): created once, its status read at
//! the stored `Location` on every later check (§2:3.94.4.3), and created
//! again when the Registry no longer holds it. When the subscriber checks is
//! our own design.

use ferroehr::service::demographic::identity_feed::config::PixConfig;
use ferroehr::service::demographic::identity_feed::subscribe::{SubscribeOutcome, Subscription};
use ferrohealth_smart::config::endpoint::EndpointSection;
use ihe_iti::pmir::subscription::SubscriptionStatus;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The consumer's feed route.
const FEED: &str = "https://cdr.example/pmir/feed";

fn subscription(registry: &MockServer) -> Subscription {
    let config = PixConfig {
        endpoint: EndpointSection {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!("{}/fhir", registry.uri())),
            ..EndpointSection::default()
        },
        feed_issuer: "https://as.example".to_owned(),
        feed_client_id: "ferropix".to_owned(),
    };
    Subscription::new(&config, FEED).expect("the subscriber builds")
}

/// The `Subscription` the Registry answers, of `status`.
fn held(status: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/fhir+json")
        .set_body_json(json!({
            "resourceType": "Subscription", "id": "sub-1", "status": status,
            "reason": "feed", "criteria": "Patient",
            "channel": { "type": "message", "endpoint": FEED, "payload": "application/fhir+json" }
        }))
}

fn created(registry: &MockServer) -> ResponseTemplate {
    ResponseTemplate::new(201)
        .insert_header(
            "location",
            format!("{}/fhir/Subscription/sub-1", registry.uri()),
        )
        .insert_header("content-type", "application/fhir+json")
        .set_body_json(json!({
            "resourceType": "Subscription", "id": "sub-1", "status": "active",
            "reason": "feed", "criteria": "Patient",
            "channel": { "type": "message", "endpoint": FEED, "payload": "application/fhir+json" }
        }))
}

#[tokio::test]
async fn a_subscription_is_created_once_and_read_at_its_location_after() {
    let registry = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/Subscription"))
        .respond_with(created(&registry))
        .expect(1)
        .mount(&registry)
        .await;
    Mock::given(method("GET"))
        .and(path("/fhir/Subscription/sub-1"))
        .respond_with(held("error"))
        .mount(&registry)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let one = crate::patient_index::replica(
        &db,
        ferroehr::service::ehr_index::provider::EhrIndexProvider::Local,
    )
    .await;
    let two = crate::patient_index::replica(
        &db,
        ferroehr::service::ehr_index::provider::EhrIndexProvider::Local,
    )
    .await;
    let subscription = subscription(&registry);
    assert_eq!(
        one.pmir_subscribe(&subscription)
            .await
            .expect("the check runs"),
        SubscribeOutcome::Created
    );
    assert_eq!(
        two.pmir_subscribe(&subscription)
            .await
            .expect("the check runs"),
        SubscribeOutcome::Held(SubscriptionStatus::Error),
        "another replica reads the stored subscription and creates none"
    );
}

#[tokio::test]
async fn a_subscription_the_registry_no_longer_holds_is_created_again() {
    let registry = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/Subscription"))
        .respond_with(created(&registry))
        .expect(2)
        .mount(&registry)
        .await;
    Mock::given(method("GET"))
        .and(path("/fhir/Subscription/sub-1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&registry)
        .await;
    let db = testkit::db().await.expect("testkit database");
    let svc = crate::patient_index::replica(
        &db,
        ferroehr::service::ehr_index::provider::EhrIndexProvider::Local,
    )
    .await;
    let subscription = subscription(&registry);
    for _ in 0..2 {
        assert_eq!(
            svc.pmir_subscribe(&subscription)
                .await
                .expect("the check runs"),
            SubscribeOutcome::Created
        );
    }
}
