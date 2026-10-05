// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `RESULT_SET` export over a native `<form method="post">` download.
//!
//! A plain BFF axum route (NOT a `#[server]` fn), so the download itself is a
//! form submission the client router never intercepts. It is not a no-JS path
//! end to end: the forms only render on a hydrated results pane and their
//! hidden inputs are `prop:value`-bound (nothing lands in SSR markup), so a
//! WASM-less browser never reaches a filled form.
//!
//! The route enforces the viewer session exactly like every server fn (the
//! public-endpoint rule) and runs the AQL through the same CDR client.
//!
//! The query is sent to the CDR as-is (no `fetch`/`offset` paging): a
//! query carrying its own `LIMIT` exports that window, and an unbounded
//! query is capped by the CDR's server-side default fetch limit.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::error::ViewerError;

/// The export form body (`q` + optional JSON parameters + the format).
#[derive(Debug, serde::Deserialize)]
pub struct ExportForm {
    /// The AQL text.
    pub q: String,
    /// `query_parameters` as a JSON object (empty = none).
    #[serde(default)]
    pub parameters_json: String,
    /// `csv` or `json`.
    pub format: String,
}

/// `POST /export/aql` — run the query and answer with a file download.
///
/// Callers without a viewer session are redirected to `/login` (the form is
/// a full-page navigation, so a redirect is the correct UX and the correct
/// security answer) — a missing, tampered or idle-expired cookie all read as
/// no session. A CDR-side refusal of the session's credential instead
/// answers the error branch below (the download never navigates away on a
/// mid-flight CDR 401).
pub async fn export_aql(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<ExportForm>,
) -> Response {
    let session = crate::session::unseal(
        &state.session_keys,
        &headers,
        state.config.session.idle_minutes,
    );
    let Some(admin) = session.get::<crate::session::ViewerSession>(crate::session::SESSION_KEY)
    else {
        return axum::response::Redirect::to("/login").into_response();
    };
    match run(&state, &admin, &form).await {
        Ok(response) => response,
        Err(ViewerError::Unauthenticated) => axum::response::Redirect::to("/login").into_response(),
        Err(e) => plain(StatusCode::BAD_GATEWAY, &format!("export failed: {e}")),
    }
}

async fn run(
    state: &crate::state::AppState,
    admin: &crate::session::ViewerSession,
    form: &ExportForm,
) -> Result<Response, ViewerError> {
    let parameters: serde_json::Value = if form.parameters_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&form.parameters_json)
            .map_err(|e| ViewerError::Invalid(format!("parameters must be a JSON object: {e}")))?
    };
    if !parameters.is_object() {
        return Err(ViewerError::Invalid(
            "parameters must be a JSON object".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1("query/aql");
    let body = serde_json::json!({ "q": form.q, "query_parameters": parameters }).to_string();
    let response = state
        .cdr
        .post(
            &admin.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;

    match form.format.as_str() {
        "json" => Ok(download(
            "application/json",
            "aql-export.json",
            pretty_json(&response.body),
        )),
        "csv" => {
            let csv = result_set_to_csv(&response.body)?;
            Ok(download("text/csv; charset=utf-8", "aql-export.csv", csv))
        }
        other => Err(ViewerError::Invalid(format!(
            "unknown export format `{other}` (csv | json)"
        ))),
    }
}

/// Pretty-print the `RESULT_SET` when it parses; fall back to the raw body.
fn pretty_json(body: &str) -> Vec<u8> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| serde_json::to_vec_pretty(&v).ok())
        .unwrap_or_else(|| body.as_bytes().to_vec())
}

/// Flatten the `RESULT_SET` into CSV: the column names (falling back to the
/// column path, then to a 1-based position) as the header row, scalar
/// cells verbatim, null as empty, and structured cells as compact JSON.
fn result_set_to_csv(body: &str) -> Result<Vec<u8>, ViewerError> {
    let doc: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("unparseable RESULT_SET: {e}")))?;
    let columns = doc
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let header: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            c.get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| c.get("path").and_then(serde_json::Value::as_str))
                .map_or_else(|| format!("column {}", i + 1), str::to_owned)
        })
        .collect();
    let mut writer = csv::Writer::from_writer(Vec::new());
    if !header.is_empty() {
        writer
            .write_record(&header)
            .map_err(|e| ViewerError::Internal(format!("csv: {e}")))?;
    }
    let empty = Vec::new();
    let rows = doc
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    for row in rows {
        let cells = row.as_array().cloned().unwrap_or_default();
        let record: Vec<String> = cells.iter().map(cell_text).collect();
        writer
            .write_record(&record)
            .map_err(|e| ViewerError::Internal(format!("csv: {e}")))?;
    }
    writer
        .into_inner()
        .map_err(|e| ViewerError::Internal(format!("csv: {e}")))
}

/// One CSV cell: scalars verbatim, null empty, structures as compact JSON.
fn cell_text(cell: &serde_json::Value) -> String {
    match cell {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn download(content_type: &str, filename: &str, body: Vec<u8>) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

fn plain(status: StatusCode, message: &str) -> Response {
    (status, message.to_owned()).into_response()
}

#[cfg(test)]
mod tests {
    use super::{cell_text, result_set_to_csv};

    #[test]
    fn csv_flattens_scalars_nulls_and_structures() {
        let body = r#"{
            "columns": [{"name": "uid"}, {"name": "", "path": "/context/start_time"}, {}],
            "rows": [
                ["a::b::1", 42.5, {"_type": "DV_TEXT", "value": "x"}],
                [null, true, "plain, quoted"]
            ]
        }"#;
        let csv = String::from_utf8(result_set_to_csv(body).expect("csv")).expect("utf8");
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().expect("header"),
            "uid,/context/start_time,column 3"
        );
        assert_eq!(
            lines.next().expect("row 1"),
            "a::b::1,42.5,\"{\"\"_type\"\":\"\"DV_TEXT\"\",\"\"value\"\":\"\"x\"\"}\""
        );
        assert_eq!(lines.next().expect("row 2"), ",true,\"plain, quoted\"");
    }

    #[test]
    fn cell_text_keeps_scalars_verbatim() {
        assert_eq!(cell_text(&serde_json::json!("s")), "s");
        assert_eq!(cell_text(&serde_json::json!(7)), "7");
        assert_eq!(cell_text(&serde_json::Value::Null), "");
    }
}
