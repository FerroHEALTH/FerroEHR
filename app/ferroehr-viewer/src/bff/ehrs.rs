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

#[cfg(not(target_arch = "wasm32"))]
use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::{Msg, tr, tr_with};

#[cfg(not(target_arch = "wasm32"))]
/// The fixed AQL that lists EHRs newest-first for the recent-EHRs table.
/// Validated by the `fixed_aql_parses` test; never concatenated with user
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

#[cfg(not(target_arch = "wasm32"))]
/// Build the `POST query/aql` request body: the fixed AQL text, the caller's
/// parameter bindings (never string-interpolated into the query), and the
/// `fetch`/`offset` window.
pub(crate) fn aql_request_body(aql: &str, parameters: &Value, offset: u32, fetch: u32) -> String {
    serde_json::json!({
        "q": aql,
        "query_parameters": parameters,
        "fetch": fetch,
        "offset": offset,
    })
    .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
/// Parse an AQL `RESULT_SET` JSON body into a [`ResultPage`]. The result-set
/// shape (`columns: [{name, path}]`, `rows: [[…]]`) is the ITS-REST Query API
/// contract.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_result_set(body: &str, offset: u32) -> Result<ResultPage, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::AqlResultJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
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
    /// How many rows the page holds.
    fetch: u32,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("query/aql");
    let body = aql_request_body(LIST_EHRS_AQL, &serde_json::json!({}), offset, fetch);
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

#[cfg(not(target_arch = "wasm32"))]
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
        "name": { "_type": "DV_TEXT", "value": tr(Msg::EhrStatus) },
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

#[cfg(not(target_arch = "wasm32"))]
/// Pull the `ehr_id.value` out of an `EHR` resource body (the
/// `Prefer: return=representation` response of `POST /ehr` and the body of
/// `GET /ehr?subject_id…`).
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON or carries no
/// `ehr_id`.
pub(crate) fn parse_ehr_id(body: &str) -> Result<String, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::EhrJson3, &[("e", (e).to_string().into())]))
    })?;
    doc.get("ehr_id")
        .and_then(|v| v.get("value"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ViewerError::Internal(tr(Msg::EhrResponseCarriedNoEhrId)))
}

#[cfg(not(target_arch = "wasm32"))]
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
            return Err(ViewerError::Invalid(tr_served(
                Msg::ProvideBothSubjectIdSubjectNamespace,
            )));
        }
    };
    // A server function is a public endpoint: the client-side check is a
    // courtesy, this one is the guard.
    if !ehr_id.is_empty() && !is_uuid(ehr_id) {
        return Err(ViewerError::Invalid(format!(
            "{ehr_id:?} is not a UUID: the openEHR EHR API strongly recommends a UUID for a \
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
        return Err(ViewerError::Invalid(tr_served(
            Msg::BothSubjectIdSubjectNamespaceRequired,
        )));
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

#[cfg(not(target_arch = "wasm32"))]
/// The detail-route href for one EHR id ([`crate::pages::ehr::ehr_href`]).
#[must_use]
pub(crate) fn ehr_detail_href(ehr_id: &str) -> String {
    crate::pages::ehr::ehr_href(ehr_id)
}

#[cfg(not(target_arch = "wasm32"))]
/// Render one raw AQL cell value as display text: strings verbatim, JSON null
/// as empty, anything else as compact JSON.
pub(crate) fn cell_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        LIST_EHRS_AQL, aql_request_body, cell_text, is_uuid, parse_ehr_id, parse_result_set,
        subject_ehr_status,
    };
    use crate::bff::PAGE_SIZE;

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
            PAGE_SIZE,
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
