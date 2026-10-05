// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR's **event-subscription** surface the viewer consumes.
//!
//! The availability probe the subscriptions screen and its nav entry are gated
//! on, and the four operations the screen drives over
//! `admin/event_subscription` (list / create / update / delete). The group's
//! read-by-id has no reader here on purpose: the listing already carries every
//! field the screen shows, and a second GET for the same claim is exactly what
//! the one-reader-per-claim rule forbids.
//!
//! NOTE: no openEHR spec governs eventing — our own enterprise extension; the
//! whole group (paths, payloads, status codes) is the CDR's own design.
//!
//! A subscription is a flat predicate record — `kind` / `change_type` /
//! `template_id`, each absent value meaning "any" — plus a name
//! and an `enabled` flag, so the screen edits it with ordinary form fields
//! rather than a document editor.
//!
//! **The viewer never renders a broker binding key.** Which queue an enabled
//! subscription binds, and how a wildcard predicate is spelled in a topic key,
//! is the CDR publisher's grammar; restating it here would be a second copy of
//! a grammar this crate cannot import. The screen states the predicates in
//! plain words.
//!
//! **Probe-and-hide.** The group is config-gated on the CDR (`[events]
//! admin_api`, off by default) and answers `404` for every route as if
//! unmounted while it is off, so the viewer discovers it
//! ([`probe_event_subscriptions`]) before offering any of it. Capability is not
//! authorization: a mounted-but-refused group (`401`/`403`) still counts as
//! present, and the refusal surfaces as copy on the screen that asked.
//!
//! Every `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first (a server fn is a
//! publicly reachable HTTP endpoint) and keeps the CDR credential server-side.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// One subscription as the CDR serves it
/// (`{id, name, kind, change_type, template_id, enabled,
/// created_at}`).
///
/// Every wire value is a string, including the id and the timestamp: the record
/// crosses the server-fn boundary and is rendered, never computed with (no
/// `usize`, and no parsing the viewer would have to keep in step with the
/// CDR's own formatting). A predicate the CDR serves as `null` — its wildcard —
/// arrives here as the empty string.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscriptionRow {
    /// The subscription's UUID, as the CDR spelled it.
    pub id: String,
    /// The unique subscription name — immutable once created.
    pub name: String,
    /// The versioned-object kind predicate (empty = any kind).
    pub kind: String,
    /// The audit change-type predicate (empty = any change type).
    pub change_type: String,
    /// The template-id predicate (empty = any template).
    pub template_id: String,
    /// Whether the CDR delivers this subscription's events.
    pub enabled: bool,
    /// The record's creation instant, verbatim from the wire.
    pub created_at: String,
}

/// The four predicates plus the enabled flag — everything a create or an update
/// sends besides the name.
///
/// One named struct rather than four adjacent `String` parameters at each
/// dispatch site: the predicates are all the same type, so a transposition
/// would otherwise be silent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscriptionPredicates {
    /// The versioned-object kind to match (empty = any).
    pub kind: String,
    /// The audit change-type group code to match (empty = any).
    pub change_type: String,
    /// The template id to match (empty = any).
    pub template_id: String,
    /// Whether the subscription is delivered.
    pub enabled: bool,
}

/// Whether the CDR serves its event-subscription admin API.
///
/// Carries only fixed-size, client-safe data — it crosses the server-fn
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionAvailability {
    /// The group answered: it is mounted, so the screen is offered. Whether
    /// THIS session may use it is a per-request answer.
    Available,
    /// The group is not mounted (`[events] admin_api = false`) — the CDR
    /// answered `404` as if the routes did not exist.
    Disabled,
}

impl SubscriptionAvailability {
    /// Whether the viewer may offer the subscriptions screen.
    #[must_use]
    pub fn usable(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Classify the probe's HTTP status: only a `404` means "not mounted".
///
/// A `401`/`403` is a mounted group refusing THIS session (the surface sits
/// under `/admin`, which the coarse RBAC gate classes as admin work) — the
/// surface exists, which is what the nav gate asks about.
#[must_use]
pub fn availability_of_status(status: http::StatusCode) -> SubscriptionAvailability {
    if status == http::StatusCode::NOT_FOUND {
        SubscriptionAvailability::Disabled
    } else {
        SubscriptionAvailability::Available
    }
}

/// The single predicate every subscription-gated affordance uses: render only
/// for a probe that succeeded and found the group mounted.
///
/// A failed probe (CDR unreachable, expired session) hides it — never a nav
/// link to a screen that cannot work.
#[must_use]
pub fn renders_event_subscriptions(probe: &Result<SubscriptionAvailability, ViewerError>) -> bool {
    probe
        .as_ref()
        .copied()
        .is_ok_and(SubscriptionAvailability::usable)
}

/// The eventing gate: one probe [`Resource`].
///
/// Created in component setup — never inside a `Suspend` closure, which re-runs
/// and would re-create the resource.
#[must_use]
pub fn event_subscription_gate() -> Resource<Result<SubscriptionAvailability, ViewerError>> {
    Resource::new(|| (), |()| async move { probe_event_subscriptions().await })
}

/// Render `affordance` only when the gate found the group mounted; otherwise
/// render nothing at all (probe-and-hide).
///
/// The probe is resolved INSIDE the `<Suspense>`: an SSR'd `ErrorBoundary`
/// fallback mismatches at hydration in leptos 0.8, and a render-time resource
/// read is itself a hydration mismatch. `affordance` creates no resources, so
/// re-runs are safe, and it is shared through an `Arc` because the `Suspend`
/// closure must not consume its environment.
#[must_use]
pub fn when_event_subscriptions_usable(
    gate: Resource<Result<SubscriptionAvailability, ViewerError>>,
    affordance: impl Fn() -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let affordance: std::sync::Arc<dyn Fn() -> AnyView + Send + Sync> =
        std::sync::Arc::new(affordance);
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let affordance = std::sync::Arc::clone(&affordance);
                Suspend::new(async move {
                    if renders_event_subscriptions(&gate.await) {
                        affordance()
                    } else {
                        ().into_any()
                    }
                })
            }}
        </Suspense>
    }
    .into_any()
}

/// Whether a submitted subscription name can succeed.
///
/// Mirrors the CDR's own rule — non-empty after trimming and restricted to
/// `[A-Za-z0-9_.-]`, because the name is also the suffix of the broker queue
/// the CDR declares — so the create button is inert until the form can
/// actually be accepted. The CDR remains the guard; this is the courtesy.
#[must_use]
pub fn name_is_valid(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// How one predicate reads on screen: its value, or `any` for the wildcard.
///
/// Pure and unit-tested, so the table cell and the summary sentence spell an
/// absent predicate identically on the server pass and at hydration.
#[must_use]
pub fn predicate_label(value: &str) -> String {
    if value.trim().is_empty() {
        "any".to_owned()
    } else {
        value.trim().to_owned()
    }
}

/// The one sentence a row renders under its name: which committed-version
/// events this subscription selects.
///
/// A subscription with no predicate at all selects everything, and saying so in
/// words is more honest than four cells reading "any".
#[must_use]
pub fn match_summary(row: &SubscriptionRow) -> String {
    let mut parts = Vec::new();
    for (label, value) in [
        ("kind", row.kind.as_str()),
        ("change type", row.change_type.as_str()),
        ("template", row.template_id.as_str()),
    ] {
        if !value.trim().is_empty() {
            parts.push(format!("{label} {}", value.trim()));
        }
    }
    if parts.is_empty() {
        return "Matches every committed version.".to_owned();
    }
    format!("Matches {}.", parts.join(", "))
}

/// Actionable copy for a refused subscription operation — a write, a delete, or
/// a read the RBAC gate turned down.
///
/// Names the object, carries the CDR's own diagnostic verbatim, and names the
/// next action. The `409` here is a duplicate NAME (the CDR's unique key), not
/// the admin deletes' "still referenced by committed data".
#[must_use]
pub fn subscription_failure_copy(object: &str, error: &ViewerError) -> String {
    match error {
        ViewerError::Cdr { message, .. }
            if error.status_code() == Some(http::StatusCode::CONFLICT) =>
        {
            format!(
                "The CDR already holds a subscription with that name, so {object} was not saved \
                 ({message}). Choose another name, or edit the existing subscription."
            )
        }
        ViewerError::Cdr { message, .. }
            if error.status_code() == Some(http::StatusCode::NOT_FOUND) =>
        {
            format!(
                "{object} is not on the CDR any more ({message}) — another operator may have \
                 deleted it. Reload this screen."
            )
        }
        ViewerError::Forbidden(message) => format!(
            "This session may not administer {object} ({message}). Sign in with an ADMIN-role \
             account and retry."
        ),
        other => crate::feedback::write_failure_copy(object, other),
    }
}

/// Probe the CDR for its event-subscription admin API
/// (`GET admin/event_subscription`).
///
/// The list operation is the availability signal: a `404` means the group is
/// not mounted for this deployment (`[events] admin_api = false`), any other
/// answer means it is.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnreachable`] on transport failure.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn probe_event_subscriptions() -> Result<SubscriptionAvailability, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/event_subscription");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    Ok(availability_of_status(response.status))
}

/// List the stored subscriptions, newest first
/// (`GET admin/event_subscription`).
///
/// `Ok(None)` is the CDR answering `404` — the event-subscription admin API is
/// disabled, which is an absent surface rather than an error, and the screen
/// renders it as its own first-class state.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_event_subscriptions() -> Result<Option<Vec<SubscriptionRow>>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/event_subscription");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("event subscription list JSON: {e}")))?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(subscription_row).collect())
        .unwrap_or_default();
    Ok(Some(rows))
}

/// Create a subscription (`POST admin/event_subscription`).
///
/// The CDR's `400` (empty name, or a name outside `[A-Za-z0-9_.-]`), `409`
/// (duplicate name) and `415` (wrong content type) diagnostics surface verbatim
/// through [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when the name cannot be accepted;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the created record is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_event_subscription(
    /// The subscription name (unique on the CDR, immutable afterwards).
    name: String,
    /// The predicates and the enabled flag to store.
    predicates: SubscriptionPredicates,
) -> Result<SubscriptionRow, ViewerError> {
    let session = crate::session::require_session().await?;
    if !name_is_valid(&name) {
        return Err(ViewerError::Invalid(
            "a subscription name is required and may hold only letters, digits, `_`, `.` and `-`"
                .to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/event_subscription");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            subscription_definition(&name, &predicates),
        )
        .await?;
    stored_record(&crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Replace one subscription's predicates and enabled flag
/// (`PUT admin/event_subscription/{subscription_id}`).
///
/// The name is immutable on the CDR — it is the queue key — so it is not sent.
/// The PUT REPLACES the whole predicate set: a predicate left empty becomes the
/// wildcard, which is why the edit form always submits all four.
///
/// The id is percent-encoded with the `urlencoding` crate — the CDR serves it
/// as text, and the viewer never hand-rolls a codec for a path segment.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the stored record is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_event_subscription(
    /// The subscription to update, by CDR id.
    subscription_id: String,
    /// The predicates and the enabled flag to store, replacing what is there.
    predicates: SubscriptionPredicates,
) -> Result<SubscriptionRow, ViewerError> {
    let session = crate::session::require_session().await?;
    if subscription_id.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "no subscription id to update".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/event_subscription/{}",
        urlencoding::encode(&subscription_id)
    ));
    let response = state
        .cdr
        .put(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            subscription_update(&predicates),
        )
        .await?;
    stored_record(&crate::cdr::CdrClient::expect_success(response)?.body)
}

/// Delete one subscription
/// (`DELETE admin/event_subscription/{subscription_id}`).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_event_subscription(
    /// The subscription to delete, by CDR id.
    subscription_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    if subscription_id.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "no subscription id to delete".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "admin/event_subscription/{}",
        urlencoding::encode(&subscription_id)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// One predicate as the request document spells it: the trimmed value, or JSON
/// `null` for the wildcard.
///
/// The CDR reads absent, `null` and empty alike as "any"; `null` is sent
/// because it says exactly that.
#[cfg(feature = "ssr")]
fn predicate_value(raw: &str) -> serde_json::Value {
    let value = raw.trim();
    if value.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(value.to_owned())
    }
}

/// The create request body: the name plus the four predicates and the enabled
/// flag, each value trimmed.
///
/// Built through `serde_json` rather than string formatting so a value carrying
/// a quote or a backslash is escaped by the encoder.
#[cfg(feature = "ssr")]
fn subscription_definition(name: &str, predicates: &SubscriptionPredicates) -> String {
    serde_json::json!({
        "name": name.trim(),
        "kind": predicate_value(&predicates.kind),
        "change_type": predicate_value(&predicates.change_type),
        "template_id": predicate_value(&predicates.template_id),
        "enabled": predicates.enabled,
    })
    .to_string()
}

/// The update request body: the four predicates and the enabled flag, with no
/// name (the CDR ignores an echoed one, and sending it would suggest it could
/// change).
#[cfg(feature = "ssr")]
fn subscription_update(predicates: &SubscriptionPredicates) -> String {
    serde_json::json!({
        "kind": predicate_value(&predicates.kind),
        "change_type": predicate_value(&predicates.change_type),
        "template_id": predicate_value(&predicates.template_id),
        "enabled": predicates.enabled,
    })
    .to_string()
}

/// Parse the stored record a create/update answered with.
#[cfg(feature = "ssr")]
fn stored_record(body: &str) -> Result<SubscriptionRow, ViewerError> {
    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| ViewerError::Internal(format!("event subscription record JSON: {e}")))?;
    Ok(subscription_row(&value))
}

/// Distil one served element into a [`SubscriptionRow`], reading each field
/// defensively so a missing or renamed field empties that cell rather than
/// dropping the whole row from the listing. A `null` predicate — the CDR's
/// wildcard — reads as the empty string.
#[cfg(feature = "ssr")]
fn subscription_row(item: &serde_json::Value) -> SubscriptionRow {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    SubscriptionRow {
        id: text("id"),
        name: text("name"),
        kind: text("kind"),
        change_type: text("change_type"),
        template_id: text("template_id"),
        enabled: item
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_default(),
        created_at: text("created_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SubscriptionAvailability, SubscriptionRow, availability_of_status, match_summary,
        name_is_valid, predicate_label, renders_event_subscriptions, subscription_failure_copy,
    };
    use crate::error::ViewerError;

    fn row() -> SubscriptionRow {
        SubscriptionRow {
            id: "01a02c7c-67b8-747b-918e-1c4bc976b3eb".to_owned(),
            name: "vitals".to_owned(),
            kind: "COMPOSITION".to_owned(),
            change_type: "249".to_owned(),
            template_id: "minimal_evaluation.en.v1".to_owned(),
            enabled: true,
            created_at: "2026-08-23T02:39:05.656092Z".to_owned(),
        }
    }

    #[test]
    fn only_a_404_means_the_group_is_not_mounted() {
        assert_eq!(
            availability_of_status(http::StatusCode::NOT_FOUND),
            SubscriptionAvailability::Disabled
        );
        // Mounted-but-refused, and mounted-and-served, are both "it exists".
        for status in [
            http::StatusCode::OK,
            http::StatusCode::UNAUTHORIZED,
            http::StatusCode::FORBIDDEN,
            http::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                availability_of_status(status),
                SubscriptionAvailability::Available,
                "{status}"
            );
        }
        assert!(SubscriptionAvailability::Available.usable());
        assert!(!SubscriptionAvailability::Disabled.usable());
    }

    #[test]
    fn only_an_available_group_renders_its_affordances() {
        assert!(renders_event_subscriptions(&Ok(
            SubscriptionAvailability::Available
        )));
        assert!(!renders_event_subscriptions(&Ok(
            SubscriptionAvailability::Disabled
        )));
        // A probe that never got an answer hides the entry too.
        assert!(!renders_event_subscriptions(&Err(
            ViewerError::Unauthenticated
        )));
        assert!(!renders_event_subscriptions(&Err(
            ViewerError::CdrUnreachable("connection refused".to_owned())
        )));
    }

    #[test]
    fn a_name_matches_the_cdrs_own_rule() {
        assert!(name_is_valid("vitals"));
        assert!(name_is_valid("org.example_feed-1"));
        // Trimmed before it is judged, and blank is not a name.
        assert!(name_is_valid("  vitals  "));
        assert!(!name_is_valid(""));
        assert!(!name_is_valid("   "));
        // The CDR refuses anything outside [A-Za-z0-9_.-] — it is a queue-name
        // suffix, not free text.
        assert!(!name_is_valid("has space"));
        assert!(!name_is_valid("slash/es"));
        assert!(!name_is_valid("ünicode"));
    }

    #[test]
    fn an_absent_predicate_reads_as_any() {
        assert_eq!(predicate_label("COMPOSITION"), "COMPOSITION");
        assert_eq!(predicate_label("  249  "), "249");
        assert_eq!(predicate_label(""), "any");
        assert_eq!(predicate_label("   "), "any");
    }

    #[test]
    fn the_summary_names_every_filled_predicate_and_says_so_when_none_is() {
        assert_eq!(
            match_summary(&row()),
            "Matches kind COMPOSITION, change type 249, template \
             minimal_evaluation.en.v1."
        );
        let wildcard = SubscriptionRow {
            kind: String::new(),
            change_type: String::new(),
            template_id: String::new(),
            ..row()
        };
        assert_eq!(match_summary(&wildcard), "Matches every committed version.");
    }

    #[test]
    fn a_refusal_names_the_subscription_the_diagnostic_and_the_next_action() {
        let conflict = subscription_failure_copy(
            "subscription `vitals`",
            &ViewerError::Cdr {
                status: 409,
                message: "an event subscription with that name exists".to_owned(),
            },
        );
        assert!(conflict.contains("subscription `vitals`"), "{conflict}");
        assert!(
            conflict.contains("an event subscription with that name exists"),
            "{conflict}"
        );
        assert!(conflict.contains("Choose another name"), "{conflict}");

        let gone = subscription_failure_copy(
            "subscription `vitals`",
            &ViewerError::Cdr {
                status: 404,
                message: "not found: event subscription 01a0".to_owned(),
            },
        );
        assert!(gone.contains("Reload this screen"), "{gone}");

        let refused = subscription_failure_copy(
            "subscription `vitals`",
            &ViewerError::Forbidden("operation requires the 'ADMIN' role".to_owned()),
        );
        assert!(
            refused.contains("ADMIN-role") && refused.contains("requires the 'ADMIN' role"),
            "{refused}"
        );

        // Everything else keeps the shared write copy rather than inventing a
        // second vocabulary.
        let rejected = subscription_failure_copy(
            "subscription `vitals`",
            &ViewerError::Cdr {
                status: 400,
                message: "event subscription 'name' must match [A-Za-z0-9_.-]".to_owned(),
            },
        );
        assert_eq!(
            rejected,
            crate::feedback::write_failure_copy(
                "subscription `vitals`",
                &ViewerError::Cdr {
                    status: 400,
                    message: "event subscription 'name' must match [A-Za-z0-9_.-]".to_owned(),
                }
            )
        );
    }

    /// The wire shapes this module parses, pinned against the answers the CDR
    /// actually serves.
    #[cfg(feature = "ssr")]
    #[test]
    fn a_served_element_distils_into_a_row_and_a_null_predicate_is_the_wildcard() {
        let item = serde_json::json!({
            "id": "01a02c7c-67b8-747b-918e-1c4bc976b3eb",
            "name": "vitals",
            "kind": "COMPOSITION",
            "change_type": "249",
            "template_id": "minimal_evaluation.en.v1",
            "enabled": true,
            "created_at": "2026-08-23T02:39:05.656092Z",
        });
        assert_eq!(super::subscription_row(&item), row());
        // The all-wildcard record the CDR answers a bare `{"name":…}` create
        // with: every predicate null, `enabled` defaulted true.
        let minimal = serde_json::json!({
            "id": "01a02c7c-67ce-7934-b608-e7f51a85b830",
            "name": "probe-min",
            "kind": serde_json::Value::Null,
            "change_type": serde_json::Value::Null,
            "template_id": serde_json::Value::Null,
            "enabled": true,
            "created_at": "2026-08-23T02:39:05.678466Z",
        });
        let parsed = super::subscription_row(&minimal);
        assert_eq!(parsed.name, "probe-min");
        assert!(parsed.kind.is_empty());
        assert!(parsed.enabled);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn the_request_bodies_carry_each_value_under_its_own_key() {
        let predicates = super::SubscriptionPredicates {
            kind: " COMPOSITION ".to_owned(),
            change_type: "249".to_owned(),
            template_id: "minimal_evaluation.en.v1".to_owned(),
            enabled: false,
        };
        let created: serde_json::Value =
            serde_json::from_str(&super::subscription_definition("  vitals  ", &predicates))
                .expect("the create body is JSON");
        assert_eq!(created["name"], "vitals");
        assert_eq!(created["kind"], "COMPOSITION");
        assert_eq!(created["change_type"], "249");
        assert_eq!(created["template_id"], "minimal_evaluation.en.v1");
        // A blank predicate is sent as the CDR's own wildcard, never as "".
        assert_eq!(created["enabled"], false);

        // The update carries the same predicates and NO name: the CDR treats
        // the name as immutable.
        let updated: serde_json::Value =
            serde_json::from_str(&super::subscription_update(&predicates))
                .expect("the update body is JSON");
        assert!(updated.get("name").is_none());
        assert_eq!(updated["kind"], "COMPOSITION");
        assert_eq!(updated["enabled"], false);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn a_value_carrying_a_quote_is_escaped_by_the_encoder() {
        let predicates = super::SubscriptionPredicates {
            kind: "a\"b".to_owned(),
            ..super::SubscriptionPredicates::default()
        };
        let body = super::subscription_definition("n", &predicates);
        assert!(body.contains(r#""kind":"a\"b""#), "{body}");
    }
}
