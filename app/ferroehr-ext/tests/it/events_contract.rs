// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EventPublisher` contract (`ferroehr_ext::events`): what a publisher
//! must provide, what the trait provides for it, and how the AMQP publisher
//! behaves BEFORE and WITHOUT a broker. The at-least-once delivery over a real
//! `RabbitMQ` is `app/ferroehr/tests/it/events_amqp.rs`; nothing here needs a
//! broker, which is what lets these run in every lane.
//!
//! No openEHR spec governs eventing — our own design/extension.

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use ferroehr_ext::events::amqp::AmqpPublisher;
use ferroehr_ext::events::{EventError, EventPublisher, routing_key, subscription_binding_key};

/// A publisher that records what it was asked to publish: the shape a
/// non-AMQP implementation (or a test double in the platform) takes.
#[derive(Default)]
struct Recording {
    published: Mutex<Vec<(String, Vec<u8>)>>,
}

#[async_trait]
impl EventPublisher for Recording {
    async fn publish(&self, routing_key: &str, payload: &[u8]) -> Result<(), EventError> {
        self.published
            .lock()
            .expect("recording lock")
            .push((routing_key.to_owned(), payload.to_vec()));
        Ok(())
    }
}

/// The trait's defaults let a publisher with its own topology model implement
/// `publish` alone: declaring a subscription is a no-op that succeeds, and the
/// topology epoch never moves, so the drainer never re-declares against it.
#[tokio::test]
async fn the_trait_defaults_make_publish_the_only_obligation() {
    let publisher = Recording::default();
    publisher
        .declare_subscription("q", "COMPOSITION.*.*")
        .await
        .expect("the default declaration is a successful no-op");
    assert_eq!(publisher.topology_epoch(), 0);
    publisher
        .publish(&routing_key("COMPOSITION", "249", Some("vitals.v2")), b"{}")
        .await
        .expect("publish");
    assert_eq!(
        publisher.topology_epoch(),
        0,
        "publishing never moves the default epoch"
    );
    let recorded = publisher.published.lock().expect("recording lock");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "COMPOSITION.249.vitals_v2");
}

/// A routing key and the binding key a subscription derives for the same
/// facts agree word for word, so a topic match is exact on every declared
/// predicate and the `*` wildcard stands only where the subscription left a
/// predicate NULL. The `-` an absent template renders in a routing key is a
/// literal word, never a wildcard: a subscription filtering on a template does
/// not receive template-less versions.
#[test]
fn routing_and_binding_keys_agree_on_their_three_words() {
    let key = routing_key(
        "COMPOSITION",
        "249",
        Some("openEHR-EHR-COMPOSITION.encounter.v1"),
    );
    let exact = subscription_binding_key(
        Some("COMPOSITION"),
        Some("249"),
        Some("openEHR-EHR-COMPOSITION.encounter.v1"),
    );
    assert_eq!(
        key, exact,
        "a fully specified subscription binds the exact routing key"
    );
    assert_eq!(
        key.split('.').count(),
        3,
        "a dotted template id stays one word"
    );

    let no_template = routing_key("EHR_STATUS", "251", None);
    assert_eq!(no_template, "EHR_STATUS.251.-");
    assert_eq!(subscription_binding_key(None, None, None), "*.*.*");
    assert_ne!(
        subscription_binding_key(None, None, Some("-")),
        "*.*.*",
        "a literal `-` predicate is a word of its own, not a wildcard"
    );
}

/// Construction performs no I/O: a publisher over an unreachable broker is
/// built, reports the initial epoch, and fails on FIRST use with the typed
/// transport error, leaving the caller's outbox row pending. Nothing panics,
/// and the failure comes back quickly enough to be retried on the next poll.
#[tokio::test]
async fn an_unreachable_broker_is_a_typed_transport_error_on_first_use() {
    // Port 1 is never a broker; the connection is refused at once.
    let publisher = AmqpPublisher::new("amqp://127.0.0.1:1/%2f", "ferroehr.events");
    assert_eq!(
        publisher.topology_epoch(),
        0,
        "no connection was attempted yet"
    );

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        publisher.publish(&routing_key("COMPOSITION", "249", None), b"{}"),
    )
    .await
    .expect("a refused connection fails within the retry budget");
    let err = outcome.expect_err("no broker answers on port 1");
    assert!(matches!(err, EventError::Amqp(_)), "got {err:?}");
    assert!(err.to_string().starts_with("amqp transport: "), "{err}");
    assert_eq!(
        publisher.topology_epoch(),
        0,
        "a connection that never came up is not a fresh topology"
    );

    // Declaring a subscription takes the same path and fails the same way.
    let err = tokio::time::timeout(
        Duration::from_secs(20),
        publisher.declare_subscription("q", "*.*.*"),
    )
    .await
    .expect("within budget")
    .expect_err("no broker");
    assert!(matches!(err, EventError::Amqp(_)), "got {err:?}");
}
