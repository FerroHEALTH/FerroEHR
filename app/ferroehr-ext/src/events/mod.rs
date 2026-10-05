// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Contribution-outbox eventing.
//!
//! No openEHR spec governs this — our own design/extension. Gated by
//! `events.enabled` (default off), with which the publisher is never spawned and
//! the commit path is byte-identical.
//!
//! Delivery is at-least-once and per-EHR ordered: a single background drainer
//! reads pending rows in sequence order, publishes with broker confirms, and
//! only then stamps `published_at`, so a crash may duplicate (consumers
//! deduplicate on `contribution_id`) but never lose. The outbox buffers a broker
//! outage, so commits never block on it, and published rows are pruned after a
//! configurable window.
//!
//! The payload is PHI-free: the stored envelope (contribution id, `ehr_id`,
//! `committed_at`, and per version `(vo_id, kind, sys_version, change_type,
//! template_id)`) plus the delivery `seq` — identity and provenance metadata
//! only, never clinical content. [`EventPublisher`] is the broker seam and
//! [`amqp::AmqpPublisher`] its `RabbitMQ` implementation, whose topology is
//! declared when the connection is established or the subscription set changes,
//! never per poll cycle. The drainer task, the retention pruner and the
//! subscription CRUD live in the consuming crate.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): external FHIR resources, tenancy/event CRUD rows, \
              multimedia offload over stored fragments (families 3/6/8)"
)]

pub mod amqp;

use async_trait::async_trait;

/// Placeholder routing-key segment for an absent value.
/// The routing-key segment for an absent value (`-`).
pub const ABSENT: &str = "-";

/// The `*` single-word topic wildcard used for a NULL subscription predicate
/// — distinct from [`ABSENT`] (`-`), which is a *routing* key's
/// empty-template rendering, not a wildcard.
const WILDCARD: &str = "*";

/// An eventing failure — either the broker transport or a negative confirm.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// The AMQP transport failed (connect, channel, publish).
    #[error("amqp transport: {0}")]
    Amqp(#[from] lapin::Error),
    /// The broker negatively acknowledged the publish (could not route/store);
    /// the row stays pending for retry (at-least-once).
    #[error("broker nacked publish (routing key {0})")]
    Nack(String),
}

/// The broker-publish seam. AMQP is the first implementation
/// ([`amqp::AmqpPublisher`]); Kafka would be another impl of this same trait.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish one event `payload` under `routing_key`, resolving only after the
    /// broker confirms. An `Err` leaves the outbox row pending for retry.
    async fn publish(&self, routing_key: &str, payload: &[u8]) -> Result<(), EventError>;

    /// Declare a durable subscription `queue` bound to the topic exchange with
    /// `binding_key`. The drainer calls this when broker topology must be
    /// (re)established — on a fresh connection or a subscription-set change;
    /// it is idempotent (safe to re-declare a queue/binding). The default is a
    /// no-op so non-AMQP `EventPublisher`s (test doubles, a future Kafka impl
    /// with its own topic model) need not implement it.
    async fn declare_subscription(&self, queue: &str, binding_key: &str) -> Result<(), EventError> {
        let _ = (queue, binding_key);
        Ok(())
    }

    /// The topology epoch: a counter that advances every time the publisher
    /// establishes a **fresh** broker connection. The drainer re-declares the
    /// subscription topology whenever this differs from the epoch it last
    /// declared under, so a broker replaced under us (fresh, without our
    /// durable queues) gets the topology back without re-declaring on every
    /// poll cycle. The default is a constant `0` — a test double or a broker
    /// with its own persistent topology model never forces a re-declare.
    fn topology_epoch(&self) -> u64 {
        0
    }
}

/// Build the topic routing key for one committed version:
/// `<kind>.<change_type>.<template_id|->` on the topic exchange.
///
/// NOTE: AMQP topic keys use `.` as the word separator, so a
/// `template_id` containing dots (e.g. `openEHR-EHR-COMPOSITION.encounter.v1`)
/// is sanitised — every non-`[A-Za-z0-9_-]` char collapses to `_` — to keep the
/// key exactly three fields. An absent/empty `template_id` renders as `-`.
///
/// NOTE: a CONTRIBUTION may carry several versions of
/// differing kinds (e.g. EHR creation commits `EHR_STATUS` + `EHR_ACCESS`).
/// Each version is published as **its own message** under its own routing key,
/// carrying the shared envelope plus a `version_index` naming which entry it is
/// — so a template-filtered subscription receives exactly the matching
/// versions. The routing key is thus per version, not per contribution.
#[must_use]
pub fn routing_key(kind: &str, change_type: &str, template_id: Option<&str>) -> String {
    let template = template_id
        .filter(|s| !s.is_empty())
        .map_or_else(|| ABSENT.to_owned(), sanitize_segment);
    format!("{kind}.{change_type}.{template}")
}

/// Build the topic **binding** key for a subscription's predicates, parallel
/// to [`routing_key`] but substituting the `*` single-word wildcard for any
/// NULL (absent) predicate.
///
/// `archetype` is intentionally absent: the routing key has no archetype
/// segment (see the `event_subscription.archetype` NOTE), so it cannot
/// participate in topic binding.
#[must_use]
pub fn subscription_binding_key(
    kind: Option<&str>,
    change_type: Option<&str>,
    template_id: Option<&str>,
) -> String {
    let seg = |v: Option<&str>| {
        v.filter(|s| !s.is_empty())
            .map_or_else(|| WILDCARD.to_owned(), sanitize_segment)
    };
    format!("{}.{}.{}", seg(kind), seg(change_type), seg(template_id))
}

/// Derive the routing key from one stored version entry.
#[must_use]
pub fn routing_key_of_version(version: &serde_json::Value) -> String {
    let kind = version
        .get("kind")
        .and_then(|k| k.as_str())
        .unwrap_or("UNKNOWN");
    let change = version
        .get("change_type")
        .and_then(|c| c.as_str())
        .unwrap_or(ABSENT);
    let template = version.get("template_id").and_then(|t| t.as_str());
    routing_key(kind, change, template)
}

/// Collapse any AMQP-topic-hostile character (dots, spaces, …) to `_` so a
/// value stays a single routing-key word.
fn sanitize_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routing_key_shape() {
        assert_eq!(
            routing_key("COMPOSITION", "249", Some("template_x")),
            "COMPOSITION.249.template_x"
        );
    }

    #[test]
    fn routing_key_absent_template_is_dash() {
        assert_eq!(routing_key("EHR_STATUS", "251", None), "EHR_STATUS.251.-");
        assert_eq!(routing_key("FOLDER", "523", Some("")), "FOLDER.523.-");
    }

    #[test]
    fn routing_key_sanitises_dotted_template() {
        // A dotted OPT id must not split the three-field key.
        assert_eq!(
            routing_key(
                "COMPOSITION",
                "249",
                Some("openEHR-EHR-COMPOSITION.encounter.v1")
            ),
            "COMPOSITION.249.openEHR-EHR-COMPOSITION_encounter_v1"
        );
    }

    #[test]
    fn routing_key_of_version_reads_the_entry() {
        let version =
            json!({ "kind": "COMPOSITION", "change_type": "251", "template_id": "vitals.v2" });
        assert_eq!(
            routing_key_of_version(&version),
            "COMPOSITION.251.vitals_v2"
        );
    }

    #[test]
    fn routing_key_of_version_handles_null_template() {
        let version = json!({ "kind": "EHR_STATUS", "change_type": "249", "template_id": null });
        assert_eq!(routing_key_of_version(&version), "EHR_STATUS.249.-");
    }

    #[test]
    fn binding_key_wildcards_null_predicates() {
        // All-wildcard subscription: matches every three-field routing key.
        assert_eq!(subscription_binding_key(None, None, None), "*.*.*");
        // A template-only filter: any kind + change, a specific (sanitised)
        // template — the routing key `COMPOSITION.249.vitals_v2` matches it.
        assert_eq!(
            subscription_binding_key(None, None, Some("vitals.v2")),
            "*.*.vitals_v2"
        );
        // A kind + change filter, any template.
        assert_eq!(
            subscription_binding_key(Some("COMPOSITION"), Some("249"), None),
            "COMPOSITION.249.*"
        );
        // Empty string is treated as absent (→ wildcard), never `-`.
        assert_eq!(
            subscription_binding_key(Some(""), Some(""), Some("")),
            "*.*.*"
        );
    }
}
