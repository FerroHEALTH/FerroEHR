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

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::hooks::use_query_map;

use crate::components::data_table::{CELL, CELL_MONO, PAGE_SIZE, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::INPUT;
use crate::components::page_header::PageHeader;
use crate::error::ViewerError;

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
#[cfg(feature = "ssr")]
fn prefixed(prefix: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        String::new()
    } else {
        format!("{prefix}{value}")
    }
}

/// Distil the ITI-81 `searchset` Bundle into an [`AuditPage`].
#[cfg(feature = "ssr")]
fn parse_bundle(body: &str, offset: u32) -> Result<AuditPage, ViewerError> {
    let bundle = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| ViewerError::Internal(format!("AuditEvent Bundle JSON: {e}")))?;
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
#[cfg(feature = "ssr")]
fn audit_row(resource: &serde_json::Value) -> AuditRow {
    let text = |v: Option<&serde_json::Value>| {
        v.and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let type_code = text(resource.pointer("/type/code"));
    let family = match type_code.as_str() {
        "rest" => "REST".to_owned(),
        "110114" => "Authentication".to_owned(),
        "110106" => "Export".to_owned(),
        "110107" => "Import".to_owned(),
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
        "4" => ("minor failure".to_owned(), false),
        "8" => ("serious failure".to_owned(), false),
        "12" => ("major failure".to_owned(), false),
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

/// The audit-log browser screen: the URL-driven filter form and the paged
/// record table over the ITI-81 retrieval.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn AuditPage() -> impl IntoView {
    let query = use_query_map();
    let q = move |key: &str| query.read().get(key).unwrap_or_default();

    // The resource source is the full filter tuple pulled from the URL
    // (tracked); the fetcher itself is untracked by design.
    let page: Resource<Result<AuditPage, ViewerError>> = Resource::new(
        move || {
            (
                q("from"),
                q("to"),
                q("patient"),
                q("agent"),
                q("outcome"),
                q("action"),
                q("page").parse::<u32>().unwrap_or(0),
            )
        },
        |(from, to, patient, agent, outcome, action, page)| async move {
            search_audit(from, to, patient, agent, outcome, action, page * PAGE_SIZE).await
        },
    );

    let filter_section = filter_form(query);
    let table_section = audit_table(page, query);

    view! {
        <Title text="Audit log" />
        <div class="p-6">
            <PageHeader
                title="Audit log"
                subtitle="The ATNA security audit trail: who accessed what, with what outcome (IHE ITI-81 retrieval)."
            />
            {filter_section}
            {table_section}
        </div>
    }
}

/// The GET filter form (router `<Form>`): submits to `/audit` with the
/// filters as query parameters — shareable, refresh-safe, and functional
/// before WASM loads. Initial values come from the current URL so a reload
/// keeps the form filled.
fn filter_form(query: Memo<leptos_router::params::ParamsMap>) -> AnyView {
    let initial = move |key: &str| query.read_untracked().get(key).unwrap_or_default();
    let select_class = "rounded-control border border-edge bg-raised px-2 py-1.5 text-sm text-ink";
    view! {
        <leptos_router::components::Form method="GET" action="/audit" attr:class="mb-4">
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "From (instant)"
                    <input
                        type="text"
                        name="from"
                        class=format!("w-56 {INPUT}")
                        placeholder="2026-07-01T00:00:00Z"
                        value=initial("from")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "To (instant)"
                    <input
                        type="text"
                        name="to"
                        class=format!("w-56 {INPUT}")
                        placeholder="2026-07-18T00:00:00Z"
                        value=initial("to")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Patient id"
                    <input
                        type="text"
                        name="patient"
                        class=format!("w-40 {INPUT}")
                        value=initial("patient")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Principal"
                    <input
                        type="text"
                        name="agent"
                        class=format!("w-40 {INPUT}")
                        value=initial("agent")
                    />
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    // Initial selection via the option `selected` attribute:
                    // it server-renders (a `prop:` would not) and the select
                    // stays uncontrolled — the GET form owns the state.
                    "Outcome" <select name="outcome" class=select_class>
                        <option value="" selected=initial("outcome").is_empty()>
                            "any"
                        </option>
                        <option value="0" selected=initial("outcome") == "0">
                            "success"
                        </option>
                        <option value="4" selected=initial("outcome") == "4">
                            "minor failure"
                        </option>
                        <option value="8" selected=initial("outcome") == "8">
                            "serious failure"
                        </option>
                        <option value="12" selected=initial("outcome") == "12">
                            "major failure"
                        </option>
                    </select>
                </label>
                <label class="flex flex-col gap-1 text-xs text-ink-muted">
                    "Action" <select name="action" class=select_class>
                        <option value="" selected=initial("action").is_empty()>
                            "any"
                        </option>
                        <option value="C" selected=initial("action") == "C">
                            "create"
                        </option>
                        <option value="R" selected=initial("action") == "R">
                            "read"
                        </option>
                        <option value="U" selected=initial("action") == "U">
                            "update"
                        </option>
                        <option value="D" selected=initial("action") == "D">
                            "delete"
                        </option>
                        <option value="E" selected=initial("action") == "E">
                            "execute"
                        </option>
                    </select>
                </label>
                <thaw::Button
                    appearance=thaw::ButtonAppearance::Primary
                    button_type=thaw::ButtonType::Submit
                >
                    "Filter"
                </thaw::Button>
            </div>
        </leptos_router::components::Form>
    }
    .into_any()
}

/// The record table: the page resource under `<Transition>` (old rows stay
/// visible while a filter/page change loads), resolving the `Result`
/// inside the `Suspend`.
fn audit_table(
    page: Resource<Result<AuditPage, ViewerError>>,
    query: Memo<leptos_router::params::ParamsMap>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match page.await {
                    Ok(page) => page_view(page, query),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render one loaded page: the store-disabled state, the empty state, or the
/// table + pagination footer.
fn page_view(page: AuditPage, query: Memo<leptos_router::params::ParamsMap>) -> AnyView {
    if !page.available {
        return view! {
            <EmptyState
                icon=icondata_lu::LuShieldOff
                message="The local audit repository is disabled"
                hint="Enable [audit.store] on the CDR to record and browse the ATNA audit trail here."
            />
        }
        .into_any();
    }
    if page.rows.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuShieldCheck
                message="No audit records match"
                hint="Widen the time window or clear a filter — every audited operation lands here."
            />
        }
        .into_any();
    }

    let body = page
        .rows
        .into_iter()
        .map(row_view)
        .collect_view()
        .into_any();
    let footer = pagination(page.total, page.offset, query);
    view! {
        {table_shell(
            &[
                "Recorded",
                "Event",
                "Operation",
                "Outcome",
                "Principal",
                "Patient",
                "Resource",
                "Record",
            ],
            body,
        )}
        {footer}
    }
    .into_any()
}

/// One record row. The raw stored `AuditEvent` opens in a native
/// `<details>` disclosure (valid HTML, zero script).
fn row_view(row: AuditRow) -> impl IntoView {
    let outcome_class = if row.success {
        "px-3 py-2 align-top text-success"
    } else {
        "px-3 py-2 align-top text-danger"
    };
    let event = format!("{} · {}", row.family, row.action);
    view! {
        <tr class=ROW>
            <td class=CELL_MONO>{row.recorded}</td>
            <td class=CELL>{event}</td>
            <td class=CELL_MONO>{row.operation}</td>
            <td class=outcome_class>{row.outcome}</td>
            <td class=CELL_MONO>{row.principal}</td>
            <td class=CELL_MONO>{row.patient}</td>
            <td class=CELL_MONO>{row.resource}</td>
            <td class=CELL>
                <details>
                    <summary class="cursor-pointer text-accent hover:underline text-xs">
                        "view"
                    </summary>
                    <pre class="mt-2 max-h-80 max-w-xl overflow-auto rounded-card border border-edge bg-sunken p-2 font-mono text-xs">
                        {row.raw}
                    </pre>
                </details>
            </td>
        </tr>
    }
}

/// The pagination footer: total + prev/next links that preserve the current
/// filters in the URL (plain router anchors — WASM-optional).
fn pagination(total: u32, offset: u32, query: Memo<leptos_router::params::ParamsMap>) -> AnyView {
    #[expect(
        clippy::integer_division,
        reason = "a page index IS the truncating quotient of a page-aligned offset"
    )]
    let page_index = offset / PAGE_SIZE;
    let shown_from = offset + 1;
    let shown_to = (offset + PAGE_SIZE).min(total);
    let href_for = move |page: u32| {
        let map = query.read_untracked();
        let mut params: Vec<String> = Vec::new();
        for key in ["from", "to", "patient", "agent", "outcome", "action"] {
            if let Some(value) = map.get(key)
                && !value.is_empty()
            {
                params.push(format!("{key}={}", urlencoding::encode(&value)));
            }
        }
        params.push(format!("page={page}"));
        format!("/audit?{}", params.join("&"))
    };
    let prev = (page_index > 0).then(|| {
        view! {
            <leptos_router::components::A
                href=href_for(page_index - 1)
                attr:class="text-accent hover:underline"
            >
                <leptos_icons::Icon icon=icondata_lu::LuArrowLeft width="12" height="12" />
                " Newer"
            </leptos_router::components::A>
        }
    });
    let next = (shown_to < total).then(|| {
        view! {
            <leptos_router::components::A
                href=href_for(page_index + 1)
                attr:class="text-accent hover:underline"
            >
                "Older "
                <leptos_icons::Icon icon=icondata_lu::LuArrowRight width="12" height="12" />
            </leptos_router::components::A>
        }
    });
    view! {
        <div class="mt-3 flex items-center gap-4 text-sm text-ink-muted">
            <span>{format!("{shown_from}–{shown_to} of {total} records")}</span>
            {prev}
            {next}
        </div>
    }
    .into_any()
}

#[cfg(all(test, feature = "ssr"))]
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
