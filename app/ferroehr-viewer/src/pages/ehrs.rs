// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/ehrs` screen — the EHR finder.
//!
//! A lookup form (jump straight to an EHR by id) over a recent-EHRs table
//! sourced from an ad-hoc AQL query — ITS-REST has no unpaged EHR-list
//! endpoint, so listing via AQL is the spec-honest route. Paging is
//! URL-driven (`?offset=`): shareable, refresh-safe, and WASM-optional.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The wire it reads IS spec-bound: the AQL runs against `POST query/aql`
//! (`docs/specs/openehr/ITS-REST/docs/query/`). User input NEVER concatenates
//! into the AQL text — the fixed query is a validated const and the caller's
//! value travels as an AQL `query_parameters` binding.
//!
//! Every co-located `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first — server
//! functions are a public HTTP API — and the CDR credential never reaches
//! client-visible state.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::components::{A, Redirect};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::components::data_table::{CELL, CELL_MONO, PAGE_SIZE, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL};
use crate::components::notice::inline_error;
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::toast::toast_success;
use crate::error::ViewerError;

#[cfg(feature = "ssr")]
/// The fixed AQL that lists EHRs newest-first for the recent-EHRs table.
/// Validated by [`tests::fixed_aql_parses`]; never concatenated with user
/// input.
const LIST_EHRS_AQL: &str =
    "SELECT e/ehr_id/value, e/time_created/value FROM EHR e ORDER BY e/time_created/value DESC";

/// One page of an AQL `RESULT_SET`, flattened for rendering: the column
/// headers, the raw row cells, and the offset that produced it (so the view
/// can build prev/next links).
///
/// Shared across the EHR browse surfaces; carries only fixed-size ints so it
/// is WASM-safe over the server-fn boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultPage {
    /// The result-set column names (falling back to the column path).
    pub columns: Vec<String>,
    /// The result rows, each a vector of raw JSON cell values.
    pub rows: Vec<Vec<Value>>,
    /// The offset this page was fetched at (for prev/next paging).
    pub offset: u32,
}

#[cfg(feature = "ssr")]
/// Build the `POST query/aql` request body: the fixed AQL text, the caller's
/// parameter bindings (never string-interpolated into the query), and the
/// `fetch`/`offset` window.
pub(crate) fn aql_request_body(aql: &str, parameters: &Value, offset: u32) -> String {
    serde_json::json!({
        "q": aql,
        "query_parameters": parameters,
        "fetch": PAGE_SIZE,
        "offset": offset,
    })
    .to_string()
}

#[cfg(feature = "ssr")]
/// Parse an AQL `RESULT_SET` JSON body into a [`ResultPage`]. The result-set
/// shape (`columns: [{name, path}]`, `rows: [[…]]`) is the ITS-REST Query API
/// contract.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_result_set(body: &str, offset: u32) -> Result<ResultPage, ViewerError> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("AQL result JSON: {e}")))?;
    let columns = doc
        .get("columns")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .enumerate()
                .map(|(i, col)| {
                    col.get("name")
                        .and_then(Value::as_str)
                        .or_else(|| col.get("path").and_then(Value::as_str))
                        .map_or_else(|| format!("#{i}"), str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    let rows = doc
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_array().cloned().unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    Ok(ResultPage {
        columns,
        rows,
        offset,
    })
}

/// List EHRs newest-first via `LIST_EHRS_AQL`, one page at `offset`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the result set is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_ehrs(
    /// First row of the page to return.
    offset: u32,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("query/aql");
    let body = aql_request_body(LIST_EHRS_AQL, &serde_json::json!({}), offset);
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_result_set(&response.body, offset)
}

#[cfg(feature = "ssr")]
/// Build a subject-bound `EHR_STATUS` for `POST /ehr`: a `PARTY_SELF` whose
/// `external_ref` (a `PARTY_REF`) carries the subject `id.value` +
/// `namespace`, which is exactly what `GET /ehr?subject_id&subject_namespace`
/// matches against (ITS-REST EHR API `ehr_get_by_subject`; RM ehr master04
/// §EHR Status — `subject` is a `PARTY_SELF`, the subject is identified via
/// its `external_ref`, never a `PARTY_IDENTIFIED`). `is_queryable` /
/// `is_modifiable` default true.
///
/// `archetype_details` is not decoration: `EHR_STATUS` carries the invariant
/// `Is_archetype_root`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr_status.adoc`),
/// and `LOCATABLE`'s `Archetyped_valid` is
/// `is_archetype_root xor archetype_details = Void`
/// (`.../org.openehr.rm.common.locatable.adoc`) — so an `EHR_STATUS` without an
/// `ARCHETYPED` is invalid and the CDR refuses it `422`.
pub(crate) fn subject_ehr_status(subject_id: &str, subject_namespace: &str) -> Value {
    serde_json::json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {
                "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-EHR_STATUS.generic.v1"
            },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": subject_namespace,
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": subject_id }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

#[cfg(feature = "ssr")]
/// Pull the `ehr_id.value` out of an `EHR` resource body (the
/// `Prefer: return=representation` response of `POST /ehr` and the body of
/// `GET /ehr?subject_id…`).
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON or carries no
/// `ehr_id`.
pub(crate) fn parse_ehr_id(body: &str) -> Result<String, ViewerError> {
    let doc: Value =
        serde_json::from_str(body).map_err(|e| ViewerError::Internal(format!("EHR JSON: {e}")))?;
    doc.get("ehr_id")
        .and_then(|v| v.get("value"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ViewerError::Internal("EHR response carried no ehr_id".to_owned()))
}

/// Whether `value` is a UUID in the RFC 9562 §4 string representation
/// (`8-4-4-4-12` lower- or upper-case hex with hyphens).
///
/// The EHR API's client-supplied id "MUST be valid `HIER_OBJECT_ID` value. It is
/// strongly RECOMMENDED that an UUID always be used for this"
/// (`docs/specs/openehr/ITS-REST/specifications/operations/ehr_create_with_id.yaml`
/// §description), so the viewer requires a UUID and says so — a typed id is
/// checked before the round-trip and again in the server function.
///
/// Byte-wise (never `&value[..n]`, which can panic on a non-char boundary — the
/// `string_slice` reliability lint).
#[must_use]
pub(crate) fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            *byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    })
}

/// Create a new EHR.
///
/// `ehr_id` empty → the CDR mints the id (`POST /ehr`); a client-supplied
/// `ehr_id` creates that exact EHR (`PUT /ehr/{ehr_id}` —
/// `ehr_create_with_id`), and an id already in use is the CDR's `409`, which
/// the caller surfaces verbatim. Both subject fields empty → the CDR mints the
/// default `EHR_STATUS` (`PARTY_SELF` subject); both filled → a subject-bound
/// `EHR_STATUS` body (see `subject_ehr_status`). Exactly one filled is a
/// validation error (both or neither). Sends `Prefer: return=representation`
/// and returns the new `ehr_id`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when exactly one subject field is filled or the
/// supplied `ehr_id` is not a UUID; CDR transport errors pass through; a
/// non-2xx CDR answer (the `409` for an id already in use, and any validation
/// diagnostic, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_ehr(
    /// The EHR id to create under; empty lets the CDR assign one.
    ehr_id: String,
    /// The subject's external id.
    subject_id: String,
    /// The issuing namespace the subject id belongs to.
    subject_namespace: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let ehr_id = ehr_id.trim();
    let subject_id = subject_id.trim();
    let subject_namespace = subject_namespace.trim();
    let body = match (subject_id.is_empty(), subject_namespace.is_empty()) {
        // Neither: let the CDR mint the default EHR_STATUS (empty body — the
        // EHR API's request body is optional).
        (true, true) => String::new(),
        // Both: a subject-bound EHR_STATUS.
        (false, false) => subject_ehr_status(subject_id, subject_namespace).to_string(),
        // Exactly one: a bad request; the subject needs both parts.
        _ => {
            return Err(ViewerError::Invalid(
                "provide both a subject id and a subject namespace, or leave both empty".to_owned(),
            ));
        }
    };
    // A server function is a public endpoint: the client-side check is a
    // courtesy, this one is the guard.
    if !ehr_id.is_empty() && !is_uuid(ehr_id) {
        return Err(ViewerError::Invalid(format!(
            "{ehr_id:?} is not a UUID — the openEHR EHR API strongly recommends a UUID for a \
             client-supplied EHR id"
        )));
    }
    let response = if ehr_id.is_empty() {
        let url = state.cdr.rest_v1("ehr");
        state
            .cdr
            .post(
                &session.credential,
                &url,
                "application/json",
                "application/json",
                &[("Prefer", "return=representation")],
                body,
            )
            .await?
    } else {
        let url = state
            .cdr
            .rest_v1(&format!("ehr/{}", urlencoding::encode(ehr_id)));
        state
            .cdr
            .put(
                &session.credential,
                &url,
                "application/json",
                "application/json",
                &[("Prefer", "return=representation")],
                body,
            )
            .await?
    };
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let created = parse_ehr_id(&response.body);
    // The representation body names the created EHR; when a client supplied the
    // id, that id is its own answer even if the CDR returned no body.
    match created {
        Ok(id) => Ok(id),
        Err(_) if !ehr_id.is_empty() => Ok(ehr_id.to_owned()),
        Err(e) => Err(e),
    }
}

/// Look up an EHR by `subject_id` + `subject_namespace`
/// (`GET /ehr?subject_id&subject_namespace`). `Ok(Some(ehr_id))` when found,
/// `Ok(None)` on a `404` (no EHR for that subject — a first-class empty
/// state, not an error).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when either subject field is empty; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn find_ehr_by_subject(
    /// The subject's external id to look up.
    subject_id: String,
    /// The issuing namespace the subject id belongs to.
    subject_namespace: String,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let subject_id = subject_id.trim();
    let subject_namespace = subject_namespace.trim();
    if subject_id.is_empty() || subject_namespace.is_empty() {
        return Err(ViewerError::Invalid(
            "both a subject id and a subject namespace are required".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr?subject_id={}&subject_namespace={}",
        urlencoding::encode(subject_id),
        urlencoding::encode(subject_namespace)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_ehr_id(&response.body).map(Some)
}

/// The detail-route href for one EHR id. The id is user- or CDR-supplied, so
/// the path segment is percent-encoded with `urlencoding` — an id carrying `/`,
/// `#`, `?` or `%` would otherwise address a different route.
///
/// Encoding is also what keeps the `?find=` redirect safe: the returned path
/// becomes a `Location` header value server-side, and `urlencoding::encode`
/// emits only unreserved ASCII plus `%XX`, so a submitted id carrying a control
/// character or a non-ASCII byte can never reach the header builder.
#[must_use]
pub(crate) fn ehr_detail_href(ehr_id: &str) -> String {
    format!("/ehrs/{}", urlencoding::encode(ehr_id))
}

/// The `?find=` value the no-JS finder form submits, trimmed. Empty (or
/// absent) means "no lookup requested".
fn find_from_url() -> String {
    leptos_router::hooks::use_query_map()
        .with_untracked(|q| q.get("find").unwrap_or_default())
        .trim()
        .to_owned()
}

/// The `/ehrs` screen: a lookup form over a URL-paged recent-EHRs table.
///
/// `?find=<ehr_id>` — what the finder's GET form submits when WASM has not
/// loaded — short-circuits the whole screen into a [`Redirect`] to that EHR's
/// detail route, so find-by-id is a plain HTML round-trip with no JavaScript
/// at all (`Redirect` sets the response redirect server-side; the
/// authenticated routes render `SsrMode::Async`, so the header is still
/// Read untracked at setup: the parameter is a submitted request, not reactive
/// screen state, and this way the redirect case creates no resources and issues
/// no AQL. Untracked is sound only because every route that can carry `?find=`
/// is entered as a fresh render of this component — the hydrated form navigates
/// straight to the detail route, and a pasted URL is a full document load. A
/// same-path client-side navigation would NOT re-run this body, so anything
/// that adds an in-app `/ehrs?find=…` link must make this decision reactive
/// instead.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn EhrsPage() -> impl IntoView {
    let find = find_from_url();
    if !find.is_empty() {
        return view! {
            <Title text="EHRs" />
            <Redirect path=ehr_detail_href(&find) />
        }
        .into_any();
    }

    let toaster = thaw::ToasterInjection::expect_context();
    let create = create_ehr_section(toaster);
    let finder = finder_section();
    let query = leptos_router::hooks::use_query_map();
    let offset = offset_from_url();
    let table = recent_ehrs_section(offset, query);

    view! {
        <Title text="EHRs" />
        <div class="p-6">
            <PageHeader
                title="EHRs"
                subtitle="Create an EHR, find one by id or subject, or browse the most recent."
            />
            {create}
            {finder}
            {table}
        </div>
    }
    .into_any()
}

/// The Create-EHR card: an optional client-supplied EHR id, an optional subject
/// id + namespace (both or neither), a Create button dispatching the
/// [`create_ehr`] action, and — on success — a toast plus client-side navigation
/// to the new EHR's detail route. The both-or-neither rule and the UUID shape of
/// a supplied id are validated client-side before dispatch and re-checked
/// server-side; a CDR validation diagnostic surfaces inline verbatim.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the create card's inputs + validation + action wiring (rules §1)"
)]
fn create_ehr_section(toaster: thaw::ToasterInjection) -> AnyView {
    // UNCONTROLLED inputs, read at dispatch — a controlled input resets to
    // its empty signal at hydration, wiping pre-WASM typing (the login
    // form's proven pattern).
    let ehr_id_ref = NodeRef::<leptos::html::Input>::new();
    let subject_id_ref = NodeRef::<leptos::html::Input>::new();
    let subject_namespace_ref = NodeRef::<leptos::html::Input>::new();
    let validation = RwSignal::new(Option::<String>::None);
    let create = Action::new(|(ehr_id, id, ns): &(String, String, String)| {
        let ehr_id = ehr_id.clone();
        let id = id.clone();
        let ns = ns.clone();
        async move { create_ehr(ehr_id, id, ns).await }
    });

    // Report the outcome and, on success, navigate to the new EHR. Both are
    // outside-world side-effects (the router, the thaw toaster), so an Effect
    // is their correct home; it never runs on the server pass. The route
    // instance unmounts on navigation, cleaning the Effect up. Failure toasts
    // too (the viewer's mutation-feedback rule); the CDR's validation
    // diagnostic also stays inline below the form.
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match create.value().get() {
        Some(Ok(new_id)) => {
            toast_success(toaster, "EHR created", &format!("New EHR {new_id}"));
            navigate(
                &ehr_detail_href(&new_id),
                leptos_router::NavigateOptions::default(),
            );
        }
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(toaster, "Create failed", "the new EHR", &error);
        }
        None => {}
    });

    let on_click = move |_| {
        let ehr_id = ehr_id_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        let id = subject_id_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        let ns = subject_namespace_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if id.is_empty() != ns.is_empty() {
            validation.set(Some(
                "Provide both a subject id and a namespace, or leave both empty for an anonymous EHR."
                    .to_owned(),
            ));
            return;
        }
        if !ehr_id.is_empty() && !is_uuid(&ehr_id) {
            validation.set(Some(
                "An EHR id must be a UUID (8-4-4-4-12 hex, e.g. \
                 7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d). Leave it empty to let the CDR mint one."
                    .to_owned(),
            ));
            return;
        }
        validation.set(None);
        create.dispatch((ehr_id, id, ns));
    };

    view! {
        <section class=format!("{CARD_PAD} mb-6")>
            <h2 class=CARD_TITLE>"Create EHR"</h2>
            <div class="flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="ehr-create-id">
                        "EHR id (optional)"
                    </label>
                    <input
                        id="ehr-create-id"
                        type="text"
                        class=INPUT
                        placeholder="ehr_id (UUID) — blank lets the CDR mint one"
                        node_ref=ehr_id_ref
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="ehr-create-subject-id">
                        "Subject id (optional)"
                    </label>
                    <input
                        id="ehr-create-subject-id"
                        type="text"
                        class=INPUT
                        placeholder="external subject id"
                        node_ref=subject_id_ref
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="ehr-create-subject-namespace">
                        "Subject namespace (optional)"
                    </label>
                    <input
                        id="ehr-create-subject-namespace"
                        type="text"
                        class=INPUT
                        placeholder="namespace"
                        node_ref=subject_namespace_ref
                    />
                </div>
                <button
                    id="ehr-create-submit"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || create.pending().get())
                    on:click=on_click
                >
                    "Create EHR"
                </button>
            </div>
            <p class="mt-2 text-xs text-ink-muted">
                "Leave the subject fields blank for an anonymous EHR (the CDR mints the default EHR_STATUS), or set both to bind a subject. "
                "Supply an EHR id to create that exact EHR — it must be a UUID, and an id already in use is refused by the CDR."
            </p>
            <div class="mt-2 text-sm">
                <Show when=move || create.pending().get()>
                    <span class="text-ink-muted">"Creating…"</span>
                </Show>
                {move || {
                    validation
                        .get()
                        .map(|msg| {
                            view! {
                                <p
                                    role="alert"
                                    class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-danger"
                                >
                                    {msg}
                                </p>
                            }
                        })
                }}
                {move || match create.value().get() {
                    Some(Err(error)) => inline_error(&error),
                    _ => ().into_any(),
                }}
            </div>
        </section>
    }
    .into_any()
}

/// The offset the recent-EHRs table is paged at, read from `?offset=` and
/// clamped to a valid `u32` (bad input reads as 0). Deterministic from the
/// URL, so hydration-safe.
fn offset_from_url() -> Signal<u32> {
    let query = leptos_router::hooks::use_query_map();
    Signal::derive(move || {
        query
            .with(|q| q.get("offset"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    })
}

/// The finder: two modes in one card. Mode 1 is the by-id jump — a plain
/// `<form method="GET" action="/ehrs">` whose `find` field the page reads and
/// redirects on, so it works with no JavaScript at all; once hydrated, its
/// `on:submit` handler cancels the round-trip and navigates client-side
/// instead (identical UX, one hop instead of two). Mode 2 is the subject
/// lookup — `subject_id` + `subject_namespace` dispatched to
/// [`find_ehr_by_subject`], navigating to the detail route when found and
/// surfacing an inline not-found note on a `404`.
#[expect(
    clippy::too_many_lines,
    reason = "two finder modes assembled as one erased section (rules §1) — splitting would separate a mode from its state"
)]
fn finder_section() -> AnyView {
    // ── Mode 1: jump by EHR id ──────────────────────────────────────────────
    // A PLAIN <form>, not the router's <Form>: pre-WASM the browser submits it
    // natively (GET /ehrs?find=…, which the page turns into a server-side
    // redirect), and post-hydration our own `on:submit` listener prevents the
    // default and navigates to the detail route. UNCONTROLLED input, read at
    // submit: a controlled input resets to its empty signal at hydration,
    // wiping anything typed before the WASM loaded.
    let lookup_ref = NodeRef::<leptos::html::Input>::new();
    let by_id_navigate = leptos_router::hooks::use_navigate();
    let on_lookup = move |ev: leptos::ev::SubmitEvent| {
        // Hydrated: cancel the full-page GET and navigate client-side.
        ev.prevent_default();
        let id = lookup_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !id.is_empty() {
            by_id_navigate(
                &ehr_detail_href(&id),
                leptos_router::NavigateOptions::default(),
            );
        }
    };

    // ── Mode 2: find by subject id + namespace ──────────────────────────────
    let subject_id_ref = NodeRef::<leptos::html::Input>::new();
    let subject_namespace_ref = NodeRef::<leptos::html::Input>::new();
    let find = Action::new(|(id, ns): &(String, String)| {
        let id = id.clone();
        let ns = ns.clone();
        async move { find_ehr_by_subject(id, ns).await }
    });
    // Navigate to the found EHR is an outside-world side-effect.
    let by_subject_navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if let Some(Ok(Some(id))) = find.value().get() {
            by_subject_navigate(
                &ehr_detail_href(&id),
                leptos_router::NavigateOptions::default(),
            );
        }
    });
    let on_find_subject = move |_| {
        let id = subject_id_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        let ns = subject_namespace_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !id.is_empty() && !ns.is_empty() {
            find.dispatch((id, ns));
        }
    };

    view! {
        <section class=format!("{CARD_PAD} mb-6")>
            <h2 class=CARD_TITLE>"Find an EHR"</h2>
            // method=GET + action=/ehrs: the no-JS submission lands on
            // /ehrs?find=<id>, which EhrsPage answers with a redirect to the
            // detail route (lookup state travels in the URL).
            <form method="GET" action="/ehrs" on:submit=on_lookup>
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        // Plain label + explicit stable input id keep the SSR↔hydration
                        // association deterministic and preserve the E2E contract
                        // (`#ehr-lookup`).
                        <label class=LABEL r#for="ehr-lookup">
                            "EHR id"
                        </label>
                        <input
                            id="ehr-lookup"
                            name="find"
                            type="text"
                            class=INPUT
                            placeholder="ehr_id (UUID)"
                            node_ref=lookup_ref
                        />
                    </div>
                    <button id="ehr-find" type="submit" class=BTN_PRIMARY>
                        "Find"
                    </button>
                </div>
            </form>
            <div class="mt-3 flex flex-wrap items-end gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="ehr-subject-id">
                        "Subject id"
                    </label>
                    <input
                        id="ehr-subject-id"
                        type="text"
                        class=INPUT
                        placeholder="external subject id"
                        node_ref=subject_id_ref
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="ehr-subject-namespace">
                        "Subject namespace"
                    </label>
                    <input
                        id="ehr-subject-namespace"
                        type="text"
                        class=INPUT
                        placeholder="namespace"
                        node_ref=subject_namespace_ref
                    />
                </div>
                <button
                    id="ehr-subject-find"
                    type="button"
                    class=BTN_SECONDARY
                    disabled=Signal::derive(move || find.pending().get())
                    on:click=on_find_subject
                >
                    "Find by subject"
                </button>
            </div>
            <div class="mt-2 text-sm">
                <Show when=move || find.pending().get()>
                    <span class="text-ink-muted">"Searching…"</span>
                </Show>
                // The "not found" line is deliberately an inline note rather than
                // an EmptyState: it answers the search control it sits under
                // (like "Searching…" above it), and no data region went empty —
                // the recent-EHRs table below is untouched and keeps its own
                // empty state.
                {move || match find.value().get() {
                    Some(Ok(None)) => {
                        view! { <p class="text-ink-muted">"No EHR found for that subject."</p> }
                            .into_any()
                    }
                    Some(Err(error)) => inline_error(&error),
                    _ => ().into_any(),
                }}
            </div>
        </section>
    }
    .into_any()
}

/// The recent-EHRs table section: an AQL-backed [`Resource`] under a
/// `<Transition>` (old rows stay visible across paging) that resolves
/// its `Result` inside the transition (an SSR'd `ErrorBoundary` fallback
/// mismatches at hydration in leptos 0.8), and prev/next links.
fn recent_ehrs_section(
    offset: Signal<u32>,
    query: Memo<leptos_router::params::ParamsMap>,
) -> AnyView {
    let resource = Resource::new(
        move || offset.get(),
        |offset| async move { list_ehrs(offset).await },
    );
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(page) => ehrs_table(&page, query),
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// Render one page of EHRs: a table whose id cells link to the detail route,
/// plus prev/next paging links. The empty page is a first-class state.
fn ehrs_table(page: &ResultPage, query: Memo<leptos_router::params::ParamsMap>) -> AnyView {
    if page.rows.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuDatabase
                message="No EHRs found"
                hint="Create an EHR through the CDR, then browse it here."
            />
        }
        .into_any();
    }
    let rows = page.rows.clone();
    let body = view! {
        <For
            each=move || rows.clone()
            key=|row| row.first().map(cell_text).unwrap_or_default()
            let:row
        >
            {ehrs_row(&row)}
        </For>
    }
    .into_any();
    let paging = paging_controls(page.offset, page.rows.len(), "/ehrs", query);
    view! {
        {table_shell(&["EHR ID", "Created"], body)}
        {paging}
    }
    .into_any()
}

/// One EHR row: the first cell (`ehr_id`) links to `/ehrs/{id}`; the rest are
/// plain text.
fn ehrs_row(row: &[Value]) -> AnyView {
    let id = row.first().map(cell_text).unwrap_or_default();
    let cells = row
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let text = cell_text(value);
            if i == 0 {
                let href = ehr_detail_href(&id);
                view! {
                    <td class=CELL_MONO>
                        <A href=href attr:class="text-accent hover:underline">
                            {text}
                        </A>
                    </td>
                }
                .into_any()
            } else {
                view! { <td class=CELL>{text}</td> }.into_any()
            }
        })
        .collect::<Vec<_>>();
    view! { <tr class=ROW>{cells}</tr> }.into_any()
}

/// Prev/next paging links for an AQL-paged table at `base` (e.g. `/ehrs`).
/// Prev appears when `offset > 0`; next appears when the page is full (there
/// may be more). Offsets use saturating arithmetic.
///
/// `query` is the screen's whole query map, so a page link carries every OTHER
/// parameter across — the tab a table sits on, the filters that produced it —
/// instead of navigating back to a bare path. Encoding is the router's own
/// [`ParamsMap::to_query_string`], never a hand-rolled codec, and the default
/// offset is written as its ABSENCE so the first page's URL stays clean.
///
/// The map is read UNTRACKED: this control renders inside the table's
/// `Suspend`, which already re-runs on every query change, so tracking here
/// would only add a dependency the closure already has.
///
/// Each step carries the same `data-page` hook the shared table footer's steps
/// use (`prev`/`next`). The two never appear on the same screen: a table is
/// either server-windowed (this control) or paged from rows in hand (the
/// footer).
pub(crate) fn paging_controls(
    offset: u32,
    row_count: usize,
    base: &str,
    query: Memo<leptos_router::params::ParamsMap>,
) -> AnyView {
    let href_for = |next: u32| {
        let mut map = query.get_untracked();
        if next == 0 {
            drop(map.remove("offset"));
        } else {
            map.replace("offset", next.to_string());
        }
        format!("{base}{}", map.to_query_string())
    };
    let full = u32::try_from(row_count).unwrap_or(u32::MAX) >= PAGE_SIZE;
    let prev = (offset > 0).then(|| {
        let href = href_for(offset.saturating_sub(PAGE_SIZE));
        view! {
            <A href=href attr:class=BTN_SECONDARY attr:data-page="prev">
                "← Previous"
            </A>
        }
        .into_any()
    });
    let next = full.then(|| {
        let href = href_for(offset.saturating_add(PAGE_SIZE));
        view! {
            <A href=href attr:class=BTN_SECONDARY attr:data-page="next">
                "Next →"
            </A>
        }
        .into_any()
    });
    view! { <div class="mt-3 flex gap-2">{prev}{next}</div> }.into_any()
}

/// Render one raw AQL cell value as display text: strings verbatim, JSON null
/// as empty, anything else as compact JSON.
pub(crate) fn cell_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{
        LIST_EHRS_AQL, aql_request_body, cell_text, is_uuid, parse_ehr_id, parse_result_set,
        subject_ehr_status,
    };
    use crate::components::data_table::PAGE_SIZE;

    #[test]
    fn a_client_supplied_ehr_id_must_be_a_uuid() {
        assert!(is_uuid("7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d"));
        // Upper case hex is the same UUID (RFC 9562 §4 is case-insensitive).
        assert!(is_uuid("7D44AA01-0F9E-4A2C-9A0F-2A6A5F9B1C3D"));
        // Too short / too long / hyphens misplaced / non-hex / not ASCII.
        assert!(!is_uuid(""));
        assert!(!is_uuid("7d44aa01"));
        assert!(!is_uuid("7d44aa010f9e4a2c9a0f2a6a5f9b1c3d"));
        assert!(!is_uuid("7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d0"));
        assert!(!is_uuid("7d44aa010-f9e-4a2c-9a0f-2a6a5f9b1c3d"));
        assert!(!is_uuid("7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3z"));
        // A multi-byte string of the same char count is not 36 BYTES — the
        // check must not panic on it either (byte-wise, no slicing).
        assert!(!is_uuid("ééééééééééééééééééééééééééééééééééé"));
    }

    #[test]
    fn fixed_aql_parses() {
        openehr_query::parser::parse_str(LIST_EHRS_AQL)
            .expect("the recent-EHRs AQL const must parse");
    }

    #[test]
    fn request_body_carries_params_and_window() {
        let body = aql_request_body(
            "SELECT e/ehr_id/value FROM EHR e",
            &serde_json::json!({}),
            50,
        );
        let doc: serde_json::Value = serde_json::from_str(&body).expect("valid JSON body");
        assert_eq!(doc["q"], "SELECT e/ehr_id/value FROM EHR e");
        assert_eq!(doc["fetch"], serde_json::json!(PAGE_SIZE));
        assert_eq!(doc["offset"], serde_json::json!(50));
        assert!(doc["query_parameters"].is_object());
    }

    #[test]
    fn parses_result_set_columns_and_rows() {
        let body = r#"{
            "columns": [{"name": "ehr_id", "path": "e/ehr_id/value"}, {"path": "e/time_created/value"}],
            "rows": [["7d44", "2026-07-12T00:00:00Z"], ["ab01", null]]
        }"#;
        let page = parse_result_set(body, 25).expect("valid result set");
        assert_eq!(page.columns, vec!["ehr_id", "e/time_created/value"]);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.offset, 25);
    }

    #[test]
    fn cell_text_renders_scalars_and_null() {
        assert_eq!(cell_text(&serde_json::json!("hello")), "hello");
        assert_eq!(cell_text(&serde_json::json!(null)), "");
        assert_eq!(cell_text(&serde_json::json!(42)), "42");
    }

    #[test]
    fn subject_ehr_status_binds_the_external_ref() {
        let status = subject_ehr_status("patient-123", "patients");
        assert_eq!(status["_type"], "EHR_STATUS");
        assert_eq!(status["subject"]["_type"], "PARTY_SELF");
        // The subject id + namespace are what GET /ehr?subject_id&subject_namespace
        // matches against (external_ref.id.value + external_ref.namespace).
        assert_eq!(
            status["subject"]["external_ref"]["id"]["value"],
            "patient-123"
        );
        assert_eq!(status["subject"]["external_ref"]["namespace"], "patients");
        assert_eq!(status["subject"]["external_ref"]["type"], "PERSON");
        assert_eq!(status["is_queryable"].as_bool(), Some(true));
        assert_eq!(status["is_modifiable"].as_bool(), Some(true));
        // EHR_STATUS is an archetype root (RM ehr_status `Is_archetype_root`),
        // and LOCATABLE's `Archetyped_valid` then makes ARCHETYPED mandatory —
        // a body without it is refused `422`.
        assert_eq!(status["archetype_details"]["_type"], "ARCHETYPED");
        assert_eq!(
            status["archetype_details"]["archetype_id"]["value"],
            "openEHR-EHR-EHR_STATUS.generic.v1"
        );
    }

    #[test]
    fn parse_ehr_id_reads_the_value_or_errors() {
        let body = r#"{"_type":"EHR","ehr_id":{"_type":"HIER_OBJECT_ID","value":"7d44aa01"}}"#;
        assert_eq!(parse_ehr_id(body).expect("ehr_id"), "7d44aa01");
        assert!(parse_ehr_id(r#"{"ehr_id":{}}"#).is_err());
        assert!(parse_ehr_id("not json").is_err());
    }
}
