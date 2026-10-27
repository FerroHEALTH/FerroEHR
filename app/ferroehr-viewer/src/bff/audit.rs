// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/audit` screen — the ATNA audit-log browser.
//!
//! Browses the CDR's local Audit Record Repository through the RESTful-ATNA
//! **ITI-81** retrieval (`GET fhir/r4/AuditEvent`): a filter form (event-time
//! window, patient, principal, outcome, action), a paged table of the stored
//! FHIR R4 `AuditEvent` documents (IHE BALP shape), and a per-row raw-record
//! view. No openEHR spec governs the viewer — our own design / product
//! extension; the wire it reads is IHE's (the `RESTful` ATNA supplement's
//! ITI-81 FHIR search), served by the CDR.
//!
//! The CDR answering `404` (the local audit store disabled) is a first-class
//! rendered state, not an error.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

#[cfg(not(target_arch = "wasm32"))]
use leptos::prelude::*;
use leptos::server;

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::PAGE_SIZE;
use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::{tr, tr_with};

/// One distilled audit record: the promoted facts the table shows plus the
/// full stored `AuditEvent` JSON for the raw view. Client-safe fields only
/// (fixed-size ints, no `usize`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditRow {
    /// The event instant (`AuditEvent.recorded`), as stored.
    pub recorded: String,
    /// The event family, humanized (REST / Authentication / Export / Import).
    pub family: String,
    /// The concrete operation (the ITS-REST operation id, or the login kind).
    pub operation: String,
    /// The action code (`C`/`R`/`U`/`D`/`E`).
    pub action: String,
    /// The outcome, humanized (success / minor failure / …).
    pub outcome: String,
    /// Whether the outcome is a success (drives the outcome tint).
    pub success: bool,
    /// The requesting principal (the requestor agent's identifier).
    pub principal: String,
    /// The recorded patient (EHR subject) id, when patient-centric.
    pub patient: String,
    /// The touched resource id, when one was recorded.
    pub resource: String,
    /// The full stored `AuditEvent` document, pretty-printed.
    pub raw: String,
}

/// One retrieved page plus the availability verdict.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditPage {
    /// `false` when the CDR answered `404` — the local Audit Record
    /// Repository is disabled (`[audit.store]`); a first-class state.
    pub available: bool,
    /// Total records matching the filter (the Bundle `total`).
    pub total: u32,
    /// The current page offset.
    pub offset: u32,
    /// The page rows, newest first.
    pub rows: Vec<AuditRow>,
}

/// Retrieve one page of audit records via the ITI-81 FHIR search
/// (`GET fhir/r4/AuditEvent`), with the supported filter subset. Empty
/// filter strings are omitted from the query; instants are sent as
/// `date=ge…`/`date=le…` bounds; values are percent-encoded.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] when the CDR no longer accepts this
/// session, [`ViewerError::Forbidden`] when the caller lacks the CDR's admin
/// role (the audit trail is an operator surface); [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// on an unparseable Bundle.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn search_audit(
    /// Inclusive lower bound on the record instant; empty means unbounded.
    from: String,
    /// Inclusive upper bound on the record instant; empty means unbounded.
    to: String,
    /// Patient reference to match; empty means any patient.
    patient: String,
    /// Agent (acting user) to match; empty means any agent.
    agent: String,
    /// Outcome code to match; empty means any outcome.
    outcome: String,
    /// Action code to match; empty means any action.
    action: String,
    /// First record of the page to return.
    offset: u32,
) -> Result<AuditPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();

    let mut params: Vec<String> = Vec::new();
    let mut push = |key: &str, value: &str| {
        if !value.trim().is_empty() {
            params.push(format!("{key}={}", urlencoding::encode(value.trim())));
        }
    };
    // NOTE: RESTful ATNA §3.81.4.1.2.1, a retrieval names at least one date, so an unset
    // window is the last 30 days (no specification governs the length: our own design).
    let from = if from.trim().is_empty() && to.trim().is_empty() {
        window_start()
    } else {
        from
    };
    push("date", &prefixed("ge", &from));
    push("date", &prefixed("le", &to));
    push("patient", &patient);
    push("agent", &agent);
    push("outcome", &outcome);
    push("action", &action);
    params.push(format!("_count={PAGE_SIZE}"));
    params.push(format!("_offset={offset}"));

    let url = format!(
        "{}?{}",
        state.cdr.rest_v1("fhir/r4/AuditEvent"),
        params.join("&")
    );
    let response = state
        .cdr
        .get(&session.credential, &url, "application/fhir+json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        // The local Audit Record Repository is disabled ([audit.store]) —
        // a rendered state, never an error.
        return Ok(AuditPage {
            available: false,
            total: 0,
            offset,
            rows: Vec::new(),
        });
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    parse_bundle(&body, offset)
}

/// `ge`/`le`-prefix a non-empty instant for the ITI-81 `date` parameter.
#[cfg(not(target_arch = "wasm32"))]
fn prefixed(prefix: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        String::new()
    } else {
        format!("{prefix}{value}")
    }
}

/// The first day of the window a search with no date asks for: 30 days
/// back, as a FHIR date.
#[cfg(not(target_arch = "wasm32"))]
fn window_start() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .checked_sub(jiff::Span::new().days(30))
        .map(|day| day.to_string())
        .unwrap_or_default()
}

/// Distil the ITI-81 `searchset` Bundle into an [`AuditPage`].
#[cfg(not(target_arch = "wasm32"))]
fn parse_bundle(body: &str, offset: u32) -> Result<AuditPage, ViewerError> {
    let bundle = serde_json::from_str::<serde_json::Value>(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            ConsoleMsg::AuditeventBundleJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let total = bundle
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .and_then(|t| u32::try_from(t).ok())
        .unwrap_or(0);
    let rows = bundle
        .get("entry")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("resource"))
                .map(audit_row)
                .collect()
        })
        .unwrap_or_default();
    Ok(AuditPage {
        available: true,
        total,
        offset,
        rows,
    })
}

/// Distil one stored `AuditEvent` (IHE BALP shape) into an [`AuditRow`],
/// reading every field defensively (a missing field renders empty, never
/// drops the record from the security log).
#[cfg(not(target_arch = "wasm32"))]
fn audit_row(resource: &serde_json::Value) -> AuditRow {
    let text = |v: Option<&serde_json::Value>| {
        v.and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let type_code = text(resource.pointer("/type/code"));
    let family = match type_code.as_str() {
        "rest" => "REST".to_owned(),
        "110114" => tr(ConsoleMsg::Authentication),
        "110106" => tr(ConsoleMsg::Export),
        "110107" => tr(ConsoleMsg::Import),
        other => other.to_owned(),
    };
    // The concrete operation: the last subtype coding (the ITS-REST
    // operation id, or the DCM login kind on authentication records).
    let operation = resource
        .pointer("/subtype")
        .and_then(serde_json::Value::as_array)
        .and_then(|codings| codings.last())
        .map(|coding| {
            let code = text(coding.get("code"));
            match code.as_str() {
                "110122" => "login".to_owned(),
                "110123" => "logout".to_owned(),
                _ => code,
            }
        })
        .unwrap_or_default();
    let outcome_code = text(resource.get("outcome"));
    let (outcome, success) = match outcome_code.as_str() {
        "0" => ("success".to_owned(), true),
        "4" => (tr(ConsoleMsg::MinorFailure), false),
        "8" => (tr(ConsoleMsg::SeriousFailure), false),
        "12" => (tr(ConsoleMsg::MajorFailure), false),
        other => (other.to_owned(), false),
    };
    // The requesting principal: the requestor agent's who-identifier
    // (skipping the token agent, which carries no who).
    let principal = resource
        .pointer("/agent")
        .and_then(serde_json::Value::as_array)
        .and_then(|agents| {
            agents.iter().find(|a| {
                a.get("requestor") == Some(&serde_json::Value::Bool(true))
                    && a.pointer("/who/identifier/value").is_some()
            })
        })
        .map(|a| text(a.pointer("/who/identifier/value")))
        .unwrap_or_default();
    // Entities: the patient (audit-entity-type 1) and the data object (2).
    let entity_value = |type_code: &str| {
        resource
            .pointer("/entity")
            .and_then(serde_json::Value::as_array)
            .and_then(|entities| {
                entities.iter().find(|e| {
                    e.pointer("/type/code").and_then(serde_json::Value::as_str) == Some(type_code)
                })
            })
            .map(|e| text(e.pointer("/what/identifier/value")))
            .unwrap_or_default()
    };
    AuditRow {
        recorded: text(resource.get("recorded")),
        family,
        operation,
        action: text(resource.get("action")),
        outcome,
        success,
        principal,
        patient: entity_value("1"),
        resource: entity_value("2"),
        raw: serde_json::to_string_pretty(resource).unwrap_or_default(),
    }
}

/// The outcome choices of the filter, as the value the query carries and
/// the words shown.
pub(crate) const OUTCOMES: [(&str, ConsoleMsg); 5] = [
    ("", ConsoleMsg::ChoiceAny),
    ("0", ConsoleMsg::ChoiceSuccess),
    ("4", ConsoleMsg::MinorFailure),
    ("8", ConsoleMsg::SeriousFailure),
    ("12", ConsoleMsg::MajorFailure),
];

/// The action choices of the filter, as the value the query carries and the
/// words shown.
pub(crate) const ACTIONS: [(&str, ConsoleMsg); 6] = [
    ("", ConsoleMsg::ChoiceAny),
    ("C", ConsoleMsg::ActionCreate),
    ("R", ConsoleMsg::ActionRead),
    ("U", ConsoleMsg::ActionUpdate),
    ("D", ConsoleMsg::ActionDelete),
    ("E", ConsoleMsg::ActionExecute),
];

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{audit_row, parse_bundle};

    fn bundle(entries: &serde_json::Value) -> String {
        serde_json::json!({
            "resourceType": "Bundle",
            "type": "searchset",
            "total": 2,
            "entry": entries,
        })
        .to_string()
    }

    #[test]
    fn parses_a_searchset_bundle() {
        let body = bundle(&serde_json::json!([
            { "resource": { "resourceType": "AuditEvent", "recorded": "2026-07-10T08:00:00Z",
                "type": { "code": "rest" }, "action": "R", "outcome": "0" } },
            { "resource": { "resourceType": "AuditEvent", "recorded": "2026-07-09T08:00:00Z",
                "type": { "code": "110114" }, "action": "E", "outcome": "4" } },
        ]));
        let page = parse_bundle(&body, 0).expect("page");
        assert!(page.available);
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[0].family, "REST");
        assert!(page.rows[0].success);
        assert_eq!(page.rows[1].family, "Authentication");
        assert_eq!(page.rows[1].outcome, "minor failure");
    }

    #[test]
    fn distils_the_balp_shape() {
        let resource = serde_json::json!({
            "resourceType": "AuditEvent",
            "recorded": "2026-07-10T08:00:00Z",
            "type": { "code": "rest" },
            "subtype": [
                { "code": "read" },
                { "code": "composition_get" }
            ],
            "action": "R",
            "outcome": "0",
            "agent": [
                { "requestor": true, "who": { "identifier": { "value": "alice" } } },
                { "requestor": false, "who": { "identifier": { "value": "ferroehr" } } },
                { "requestor": true, "policy": ["jti-1"] }
            ],
            "entity": [
                { "type": { "code": "1" }, "what": { "identifier": { "value": "patient-42" } } },
                { "type": { "code": "2" }, "what": { "identifier": { "value": "8fa1::ferroehr::1" } } }
            ]
        });
        let row = audit_row(&resource);
        assert_eq!(row.operation, "composition_get");
        assert_eq!(row.principal, "alice");
        assert_eq!(row.patient, "patient-42");
        assert_eq!(row.resource, "8fa1::ferroehr::1");
        assert!(row.raw.contains("composition_get"));
    }

    #[test]
    fn login_records_humanize() {
        let resource = serde_json::json!({
            "resourceType": "AuditEvent",
            "recorded": "2026-07-10T08:00:00Z",
            "type": { "code": "110114" },
            "subtype": [{ "code": "110122" }],
            "action": "E",
            "outcome": "4",
        });
        let row = audit_row(&resource);
        assert_eq!(row.family, "Authentication");
        assert_eq!(row.operation, "login");
        assert!(!row.success);
    }
}
