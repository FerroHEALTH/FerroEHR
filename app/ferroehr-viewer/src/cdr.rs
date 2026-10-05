// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The BFF's CDR client.
//!
//! One `reqwest` client, the ITS-REST base path, credential injection, strict
//! `Accept`/`Content-Type` negotiation (`crate::format`), and error
//! normalization into [`crate::error::ViewerError`]. The viewer reaches the
//! CDR ONLY here.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use crate::error::ViewerError;
use crate::session::Credential;

/// How many CDR requests one screen's fan-out has in flight at once.
///
/// Every BFF fan-out shares this bound, so no single page load can open an
/// unbounded burst against a shared CDR; it is small on purpose, because the
/// fan-outs are summaries and history windows rather than latency-critical
/// paths.
///
/// NOTE: no openEHR spec governs the viewer's request pacing — our own
/// design/extension.
pub const FANOUT_CONCURRENCY: usize = 8;

/// A normalized CDR response: status, the named response headers the ITS-REST
/// wire contract defines, and the raw body.
///
/// The header subset is named rather than a whole map because the viewer is a
/// typed client: `ETag` and `Last-Modified` are the state identifiers a
/// versioned resource carries, `Location` is the created resource's URL, and
/// `Preference-Applied` reports which `Prefer` the CDR honoured
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §HTTP headers).
#[derive(Debug, Clone)]
pub struct CdrResponse {
    /// The HTTP status the CDR answered with.
    pub status: http::StatusCode,
    /// The `Content-Type` the CDR declared, if any.
    pub content_type: Option<String>,
    /// The `ETag` verbatim, weakness indicator and quotes included — read
    /// through [`Self::etag_version_uid`] rather than by hand.
    pub etag: Option<String>,
    /// The `Location` of a newly created resource (`201` answers only).
    pub location: Option<String>,
    /// The `Last-Modified` instant of the served resource, if any.
    pub last_modified: Option<String>,
    /// The `Preference-Applied` the CDR reports for the request's `Prefer`.
    pub preference_applied: Option<String>,
    /// The raw body bytes as text (all ITS-REST representations are text).
    pub body: String,
}

impl CdrResponse {
    /// Whether the response carries exactly this status.
    ///
    /// The status is typed all the way from `reqwest` to the branch that reads
    /// it, so callers name an [`http::StatusCode`] constant and a transposed
    /// code is a compile error rather than a silent wrong branch.
    #[must_use]
    pub fn is(&self, code: http::StatusCode) -> bool {
        self.status == code
    }

    /// The version identifier the CDR's own `ETag` names, unwrapped from its
    /// weak form — the value an `If-Match` round-trip sends back.
    ///
    /// The header "value is usually taken from e.g. `VERSIONED_OBJECT.uid.value`,
    /// `VERSION.uid.value`" and "MUST include a weakness indicator `W/`", which a
    /// pre-1.1.0 server may still omit
    /// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
    /// §`ETag` and `Last-Modified`), so both shapes reduce to the same identifier
    /// here. `None` when the CDR sent no `ETag`, or an empty one.
    #[must_use]
    pub fn etag_version_uid(&self) -> Option<String> {
        self.etag
            .as_deref()
            .map(strip_etag)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }

    /// Assemble the normalized response from the parts `reqwest` hands back.
    fn from_parts(status: http::StatusCode, headers: &http::HeaderMap, body: String) -> Self {
        Self {
            status,
            content_type: header_text(headers, http::header::CONTENT_TYPE.as_str()),
            etag: header_text(headers, http::header::ETAG.as_str()),
            location: header_text(headers, http::header::LOCATION.as_str()),
            last_modified: header_text(headers, http::header::LAST_MODIFIED.as_str()),
            // NOTE: `Preference-Applied` has no `http::header` constant; the
            // lookup is case-insensitive either way (`http::HeaderMap::get`).
            preference_applied: header_text(headers, "preference-applied"),
            body,
        }
    }
}

/// One response header as text, or `None` when it is absent or not ASCII.
fn header_text(headers: &http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Reduce an `ETag` / `If-Match` field value to the bare identifier it wraps:
/// the weakness indicator and the surrounding quotes come off.
///
/// The weak form is what the release mandates and the bare form is the
/// deprecated one it still permits
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §`ETag` and `Last-Modified`), so a viewer that reads either can talk to both.
fn strip_etag(raw: &str) -> &str {
    let trimmed = raw.trim();
    let unweak = trimmed
        .strip_prefix("W/")
        .or_else(|| trimmed.strip_prefix("w/"))
        .unwrap_or(trimmed);
    unweak.trim().trim_matches('"')
}

/// The one outbound HTTP client to the CDR.
#[derive(Debug, Clone)]
pub struct CdrClient {
    http: reqwest::Client,
    /// scheme://host:port — no trailing slash.
    origin: String,
    /// The CDR's ITS-REST base path — leading slash, no trailing slash
    /// ([`CdrConfig::rest_base_path`](crate::config::CdrConfig::rest_base_path)).
    base_path: String,
    /// The CDR's product root, where `/status`, `/api-docs/…` and the SMART
    /// discovery document live
    /// ([`CdrConfig::rest_root`](crate::config::CdrConfig::rest_root)).
    rest_root: String,
    /// The management surface's base URL including its base path — no trailing
    /// slash. Derived from the configuration
    /// ([`CdrConfig::management_base`](crate::config::CdrConfig::management_base)),
    /// so it may point at a different host/port than [`Self::origin`].
    management_base: String,
}

impl CdrClient {
    /// Build the client from configuration.
    ///
    /// # Errors
    /// [`ViewerError::Internal`] when the underlying client cannot be
    /// constructed.
    pub fn new(cfg: &crate::config::CdrConfig) -> Result<Self, ViewerError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.request_timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ViewerError::Internal(format!("CDR HTTP client: {e}")))?;
        Ok(Self {
            http,
            origin: cfg.base_url.trim_end_matches('/').to_owned(),
            base_path: cfg.rest_base_path().to_owned(),
            rest_root: cfg.rest_root().to_owned(),
            management_base: cfg.management_base(),
        })
    }

    /// The ITS-REST v1 base (`{origin}{base_path}`, default
    /// `/ferroehr/rest/openehr/v1`). Endpoint paths appended to this come from
    /// the generated `openehr-its` REST contract; the served `openapi.json` is
    /// the cross-check.
    #[must_use]
    pub fn rest_v1(&self, path: &str) -> String {
        format!(
            "{}{}/{}",
            self.origin,
            self.base_path,
            path.trim_start_matches('/')
        )
    }

    /// The ITS-REST v1 base path itself, with NO trailing slash — the System
    /// API's conformance-manifest resource (`OPTIONS {base_path}`). The server
    /// mounts that route at the exact base path above its normalization layer,
    /// so the trailing slash `rest_v1("")` would add must not be sent.
    #[must_use]
    pub fn rest_v1_root(&self) -> String {
        format!("{}{}", self.origin, self.base_path)
    }

    /// A URL under the CDR's product root — the surfaces that sit beside the
    /// openEHR API rather than inside it (`status`, `api-docs/openapi.json`,
    /// `.well-known/smart-configuration`).
    #[must_use]
    pub fn rest_root_url(&self, path: &str) -> String {
        format!(
            "{}{}/{}",
            self.origin,
            self.rest_root,
            path.trim_start_matches('/')
        )
    }

    /// A URL directly under the CDR origin (the always-on health family, and
    /// anything else the server mounts at the process root).
    #[must_use]
    pub fn origin_url(&self, path: &str) -> String {
        format!("{}/{}", self.origin, path.trim_start_matches('/'))
    }

    /// A URL under the CDR's **management** surface (`info`, `metrics`,
    /// `metrics/{name}`, `env`, `loggers`).
    ///
    /// NOTE: no openEHR spec governs a management surface — our own operational
    /// extension. It is a separate base URL because the CDR may serve it from
    /// its own internal listener; `path` is appended to the configured prefix,
    /// which already carries the CDR's `management.base_path`.
    #[must_use]
    pub fn management_url(&self, path: &str) -> String {
        format!("{}/{}", self.management_base, path.trim_start_matches('/'))
    }

    /// `OPTIONS url` with no credential, asking for `accept`.
    ///
    /// The System API's conformance manifest is served above the CORS layer and
    /// outside authentication (ITS-REST System API — `security: []`), so this
    /// probe carries no credential.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure; the response
    /// itself (any status) is returned for the caller to interpret.
    pub async fn options_public(
        &self,
        url: &str,
        accept: &str,
    ) -> Result<CdrResponse, ViewerError> {
        Self::finish(
            self.http
                .request(http::Method::OPTIONS, url)
                .header(http::header::ACCEPT, accept),
        )
        .await
    }

    /// GET `url` as `credential`, asking for `accept`.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure; the response
    /// itself (any status) is returned for the caller to interpret.
    pub async fn get(
        &self,
        credential: &Credential,
        url: &str,
        accept: &str,
    ) -> Result<CdrResponse, ViewerError> {
        let request = self.http.get(url).header(http::header::ACCEPT, accept);
        Self::finish(Self::authorize(request, credential)).await
    }

    /// GET `url` without any credential (pre-auth surfaces: `/ferroehr/rest/status`,
    /// the SMART discovery document).
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn get_public(&self, url: &str, accept: &str) -> Result<CdrResponse, ViewerError> {
        Self::finish(self.http.get(url).header(http::header::ACCEPT, accept)).await
    }

    /// The auth schemes the CDR advertises, as `(basic, bearer)`.
    ///
    /// An unauthenticated request to a protected endpoint is answered `401`
    /// with a `WWW-Authenticate` challenge listing exactly the enabled
    /// mechanisms (RFC 9110 §11.6.1; the ITS-REST overview requires the
    /// challenge). A non-401 answer means the CDR runs with auth disabled —
    /// every mechanism the viewer offers can then "succeed", so both count
    /// as advertised.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn advertised_schemes(&self) -> Result<(bool, bool), ViewerError> {
        let url = self.rest_v1("definition/template/adl1.4");
        let response = self
            .http
            .get(&url)
            .header(http::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| ViewerError::CdrUnreachable(e.to_string()))?;
        if response.status() != http::StatusCode::UNAUTHORIZED {
            return Ok((true, true));
        }
        let challenge = response
            .headers()
            .get(http::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        Ok((challenge.contains("Basic"), challenge.contains("Bearer")))
    }

    /// POST `body` to `url` as `credential` with the given `Content-Type` +
    /// `Accept` and extra headers (e.g. `openehr-template-id` on simplified
    /// commits, `Prefer: return=representation`).
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn post(
        &self,
        credential: &Credential,
        url: &str,
        content_type: &str,
        accept: &str,
        headers: &[(&str, &str)],
        body: String,
    ) -> Result<CdrResponse, ViewerError> {
        let mut request = self
            .http
            .post(url)
            .header(http::header::CONTENT_TYPE, content_type)
            .header(http::header::ACCEPT, accept)
            .body(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        Self::finish(Self::authorize(request, credential)).await
    }

    /// PUT `body` to `url` as `credential` with the given `Content-Type` +
    /// `Accept` and extra headers (e.g. `If-Match` = the preceding
    /// `version_uid` on a versioned COMPOSITION update, `openehr-template-id`
    /// on a simplified commit). Mirrors [`Self::post`] for the write paths
    /// that need conditional/representation headers on a PUT.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn put(
        &self,
        credential: &Credential,
        url: &str,
        content_type: &str,
        accept: &str,
        headers: &[(&str, &str)],
        body: String,
    ) -> Result<CdrResponse, ViewerError> {
        let mut request = self
            .http
            .put(url)
            .header(http::header::CONTENT_TYPE, content_type)
            .header(http::header::ACCEPT, accept)
            .body(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        Self::finish(Self::authorize(request, credential)).await
    }

    /// DELETE `url` as `credential` with the given extra headers (e.g.
    /// `If-Match` = the preceding `version_uid` on a versioned directory
    /// delete). No request body; asks for `application/json` so an error body
    /// (the openEHR diagnostic) is returned in the negotiated format.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn delete(
        &self,
        credential: &Credential,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<CdrResponse, ViewerError> {
        let mut request = self
            .http
            .delete(url)
            .header(http::header::ACCEPT, "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        Self::finish(Self::authorize(request, credential)).await
    }

    /// PUT a text body to `url` as `credential` (stored-query writes use
    /// `text/plain` AQL per the Definition API).
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] on transport failure.
    pub async fn put_text(
        &self,
        credential: &Credential,
        url: &str,
        content_type: &str,
        body: String,
    ) -> Result<CdrResponse, ViewerError> {
        let request = self
            .http
            .put(url)
            .header(http::header::CONTENT_TYPE, content_type)
            .header(http::header::ACCEPT, "application/json")
            .body(body);
        Self::finish(Self::authorize(request, credential)).await
    }

    /// Map a non-2xx [`CdrResponse`] into the normalized error, extracting
    /// the diagnostic from the openEHR error body when present. 2xx passes
    /// through.
    ///
    /// The two refusals stay apart, because the CDR sends them apart: a `401`
    /// means the request "lacks valid authentication credentials for the target
    /// resource" while a `403` means the service "understood the request but
    /// refuses to authorize it"
    /// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
    /// §HTTP status codes, mandated as a pair by §Authentication and
    /// authorization) — one asks the operator to sign in again, the other to
    /// sign in as someone else.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnauthorized`] on 401, [`ViewerError::Forbidden`] on
    /// 403, [`ViewerError::Cdr`] on any other non-2xx status.
    pub fn expect_success(response: CdrResponse) -> Result<CdrResponse, ViewerError> {
        if response.status.is_success() {
            return Ok(response);
        }
        let message = diagnostic_of(&response);
        match response.status {
            http::StatusCode::UNAUTHORIZED => Err(ViewerError::CdrUnauthorized(message)),
            http::StatusCode::FORBIDDEN => Err(ViewerError::Forbidden(message)),
            status => Err(ViewerError::Cdr {
                status: status.as_u16(),
                message,
            }),
        }
    }

    fn authorize(
        request: reqwest::RequestBuilder,
        credential: &Credential,
    ) -> reqwest::RequestBuilder {
        match credential {
            Credential::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
            Credential::Bearer { access_token } => request.bearer_auth(access_token),
        }
    }

    async fn finish(request: reqwest::RequestBuilder) -> Result<CdrResponse, ViewerError> {
        let response = request
            .send()
            .await
            .map_err(|e| ViewerError::CdrUnreachable(e.to_string()))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|e| ViewerError::CdrUnreachable(format!("reading body: {e}")))?;
        Ok(CdrResponse::from_parts(status, &headers, body))
    }
}

/// The `validationErrors` entries of an openEHR error body, one string each.
///
/// The released error shape declares the member as required and carries one
/// string per violation
/// (`docs/specs/openehr/ITS-REST/specifications/schemas/others/Error.yaml`),
/// so a `422` can name every offending path instead of one generic line.
fn validation_errors(value: &serde_json::Value) -> Vec<String> {
    value
        .get("validationErrors")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    entry
                        .as_str()
                        .map_or_else(|| entry.to_string(), str::to_owned)
                })
                .filter(|text| !text.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Pull the human diagnostic out of a refused body, whatever its vocabulary:
/// the openEHR error shape (`{"message": …, "validationErrors": […]}` per
/// ITS-REST — the message first, then one line per violation) or a FHIR
/// `OperationOutcome` (the connector authors its refusals as outcomes — their
/// `issue[].diagnostics` join into one line), falling back to the raw body or
/// the bare status. ONE reader for both vocabularies, so no screen ever hands
/// raw JSON to a toast.
fn diagnostic_of(response: &CdrResponse) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.body) {
        if value
            .get("resourceType")
            .and_then(serde_json::Value::as_str)
            == Some("OperationOutcome")
        {
            let joined = value
                .get("issue")
                .and_then(serde_json::Value::as_array)
                .map(|issues| {
                    issues
                        .iter()
                        .filter_map(|issue| {
                            issue
                                .get("diagnostics")
                                .or_else(|| issue.get("code"))
                                .and_then(serde_json::Value::as_str)
                                .filter(|text| !text.is_empty())
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            if !joined.is_empty() {
                return joined;
            }
        }
        let details = validation_errors(&value);
        for key in ["message", "error"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                return std::iter::once(text.to_owned())
                    .chain(details)
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
        if !details.is_empty() {
            return details.join("\n");
        }
    }
    if response.body.is_empty() {
        format!("HTTP {}", response.status.as_u16())
    } else {
        let mut text = response.body.clone();
        text.truncate(500);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{CdrClient, CdrResponse, diagnostic_of, strip_etag};
    use crate::error::ViewerError;

    /// A served version identifier, in the shape the overview's own `ETag`
    /// example carries.
    const UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";

    fn response(status: http::StatusCode, body: &str) -> CdrResponse {
        CdrResponse {
            status,
            content_type: Some("application/json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: body.to_owned(),
        }
    }

    #[test]
    fn an_operation_outcome_refusal_yields_its_diagnostics_never_raw_json() {
        // #2581: the shared reader speaks the FHIR refusal vocabulary too.
        let response = CdrResponse {
            status: http::StatusCode::BAD_REQUEST,
            content_type: Some("application/fhir+json".to_owned()),
            etag: None,
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"resourceType":"OperationOutcome","issue":[
                {"severity":"error","code":"invalid","diagnostics":"missing field `template_id`"},
                {"severity":"error","code":"structure"}]}"#
                .to_owned(),
        };
        let text = diagnostic_of(&response);
        assert_eq!(text, "missing field `template_id`; structure");
        assert!(!text.contains("resourceType"), "never raw JSON: {text}");
    }

    #[test]
    fn diagnostic_prefers_the_openehr_message_field() {
        let r = response(
            http::StatusCode::BAD_REQUEST,
            r#"{"error":"Bad Request","message":"AQL syntax error at line 1"}"#,
        );
        assert_eq!(diagnostic_of(&r), "AQL syntax error at line 1");
    }

    #[test]
    fn diagnostic_falls_back_to_error_then_raw_then_status() {
        assert_eq!(
            diagnostic_of(&response(
                http::StatusCode::CONFLICT,
                r#"{"error":"duplicate"}"#
            )),
            "duplicate"
        );
        assert_eq!(
            diagnostic_of(&response(http::StatusCode::IM_A_TEAPOT, "teapot")),
            "teapot"
        );
        assert_eq!(
            diagnostic_of(&response(http::StatusCode::BAD_GATEWAY, "")),
            "HTTP 502"
        );
    }

    #[test]
    fn every_validation_error_gets_its_own_line_under_the_message() {
        // `Error.yaml` declares `validationErrors` as a required member: a 422
        // names each offending path, so the reader renders all of them.
        let r = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"Validation failed","validationErrors":[
                "/content[0]/data: missing mandatory element",
                "/content[0]/items[1]: cardinality 1..1 violated",
                "/context/start_time: value is not a DV_DATE_TIME"]}"#,
        );
        assert_eq!(
            diagnostic_of(&r),
            "Validation failed\n/content[0]/data: missing mandatory element\n/content[0]/items[1]: \
             cardinality 1..1 violated\n/context/start_time: value is not a DV_DATE_TIME"
        );
    }

    #[test]
    fn validation_errors_render_even_without_a_message_member() {
        let r = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"validationErrors":["/content: missing"]}"#,
        );
        assert_eq!(diagnostic_of(&r), "/content: missing");
        // An empty list leaves the message exactly as it was.
        let plain = response(
            http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"message":"Validation failed","validationErrors":[]}"#,
        );
        assert_eq!(diagnostic_of(&plain), "Validation failed");
    }

    /// The viewer mirrors the CDR's own base-path rule: the API hangs off
    /// `cdr.base_path`, and the product root drops the trailing `v1` segment
    /// plus an `openehr` segment directly before it. Restated here because the
    /// viewer reaches the CDR strictly over ITS-REST and never links against
    /// its crates.
    #[test]
    fn every_url_family_follows_the_configured_base_path() {
        for (base_path, expected_root) in [
            ("/ferroehr/rest/openehr/v1", "/ferroehr/rest"),
            ("/ferroehr/v1", "/ferroehr"),
            ("/ferroehr/openehr/v1", "/ferroehr"),
            ("/ferroehr/cdr/v1", "/ferroehr/cdr"),
            // A trailing slash is normalized away rather than doubling a
            // separator into every derived URL.
            ("/ferroehr/v1/", "/ferroehr"),
        ] {
            let client = CdrClient::new(&crate::config::CdrConfig {
                base_url: "http://cdr:8080".to_owned(),
                base_path: base_path.to_owned(),
                ..crate::config::CdrConfig::default()
            })
            .expect("client");
            let base = base_path.trim_end_matches('/');
            assert_eq!(
                client.rest_v1("query/aql"),
                format!("http://cdr:8080{base}/query/aql")
            );
            assert_eq!(client.rest_v1_root(), format!("http://cdr:8080{base}"));
            assert_eq!(
                client.rest_root_url("status"),
                format!("http://cdr:8080{expected_root}/status")
            );
            // A leading slash on the path is not a second separator.
            assert_eq!(
                client.rest_root_url("/api-docs/openapi.json"),
                format!("http://cdr:8080{expected_root}/api-docs/openapi.json")
            );
        }
    }

    #[test]
    fn management_urls_hang_off_their_own_configurable_prefix() {
        let default = CdrClient::new(&crate::config::CdrConfig::default()).expect("client");
        assert_eq!(
            default.management_url("info"),
            "http://localhost:8080/management/info"
        );
        // A leading slash on the path is not a second separator.
        assert_eq!(
            default.management_url("/metrics/aql_queries_total"),
            "http://localhost:8080/management/metrics/aql_queries_total"
        );
        // The surface may live on its own internal listener, unrelated to the
        // API origin the rest of the viewer talks to.
        let split = CdrClient::new(&crate::config::CdrConfig {
            base_url: "http://cdr:8080".to_owned(),
            management_base_url: "http://cdr:9464/management".to_owned(),
            ..crate::config::CdrConfig::default()
        })
        .expect("client");
        assert_eq!(
            split.management_url("loggers"),
            "http://cdr:9464/management/loggers"
        );
        assert_eq!(
            split.rest_v1("query/aql"),
            "http://cdr:8080/ferroehr/rest/openehr/v1/query/aql"
        );
    }

    #[test]
    fn expect_success_keeps_the_401_and_403_refusals_apart() {
        // §HTTP status codes: 401 "lacks valid authentication credentials",
        // 403 "understood the request but refuses to authorize it" — two
        // different next actions, so two variants.
        let unauthorized = CdrClient::expect_success(response(
            http::StatusCode::UNAUTHORIZED,
            r#"{"message":"the bearer token has expired"}"#,
        ))
        .unwrap_err();
        assert_eq!(
            unauthorized,
            ViewerError::CdrUnauthorized("the bearer token has expired".to_owned())
        );
        assert_eq!(
            unauthorized.status_code(),
            Some(http::StatusCode::UNAUTHORIZED)
        );

        let err = CdrClient::expect_success(response(
            http::StatusCode::FORBIDDEN,
            r#"{"message":"scope"}"#,
        ))
        .unwrap_err();
        assert_eq!(err, ViewerError::Forbidden("scope".to_owned()));
        assert_eq!(err.status_code(), Some(http::StatusCode::FORBIDDEN));

        let err = CdrClient::expect_success(response(http::StatusCode::NOT_FOUND, "")).unwrap_err();
        assert_eq!(
            err,
            ViewerError::Cdr {
                status: 404,
                message: "HTTP 404".to_owned()
            }
        );
        // The typed status survives the round trip into the error's own reader.
        assert_eq!(err.status_code(), Some(http::StatusCode::NOT_FOUND));
        assert!(CdrClient::expect_success(response(http::StatusCode::CREATED, "ok")).is_ok());
    }

    #[test]
    fn an_etag_reduces_to_its_identifier_in_either_published_shape() {
        // The weak form is what Release-1.1.0 mandates; the bare one is the
        // deprecated shape it still permits (§`ETag` and `Last-Modified`).
        assert_eq!(strip_etag(&format!("W/\"{UID}\"")), UID);
        assert_eq!(strip_etag(&format!("\"{UID}\"")), UID);
        assert_eq!(strip_etag(&format!("  w/\"{UID}\"  ")), UID);
        assert_eq!(strip_etag(UID), UID);
        assert_eq!(strip_etag("\"\""), "");
        assert_eq!(strip_etag(""), "");
    }

    #[test]
    fn the_named_response_headers_reach_the_caller_whatever_their_case() {
        let mut headers = http::HeaderMap::new();
        for (name, value) in [
            ("Content-Type", "application/json"),
            ("ETag", "W/\"7d44::sys::3\""),
            ("LOCATION", "https://cdr.example.org/ehr/1/composition/7d44"),
            ("last-modified", "Wed, 22 Jul 2026 19:15:56 GMT"),
            ("Preference-Applied", "return=representation"),
        ] {
            let name = http::HeaderName::from_bytes(name.as_bytes()).expect("a header name");
            let value = http::HeaderValue::from_static(value);
            assert!(headers.insert(name, value).is_none());
        }
        let response =
            CdrResponse::from_parts(http::StatusCode::CREATED, &headers, "{}".to_owned());
        assert_eq!(response.content_type.as_deref(), Some("application/json"));
        assert_eq!(response.etag.as_deref(), Some("W/\"7d44::sys::3\""));
        assert_eq!(
            response.location.as_deref(),
            Some("https://cdr.example.org/ehr/1/composition/7d44")
        );
        assert_eq!(
            response.last_modified.as_deref(),
            Some("Wed, 22 Jul 2026 19:15:56 GMT")
        );
        assert_eq!(
            response.preference_applied.as_deref(),
            Some("return=representation")
        );
        // The If-Match round-trip reads the identifier, never the raw field.
        assert_eq!(response.etag_version_uid().as_deref(), Some("7d44::sys::3"));

        // An answer with no ETag at all offers no identifier to echo back.
        let bare =
            CdrResponse::from_parts(http::StatusCode::OK, &http::HeaderMap::new(), String::new());
        assert_eq!(bare.etag_version_uid(), None);
        assert_eq!(bare.content_type, None);
        // An empty ETag is an absent identifier, not an empty precondition.
        let mut empty = http::HeaderMap::new();
        assert!(
            empty
                .insert(http::header::ETAG, http::HeaderValue::from_static("\"\""))
                .is_none()
        );
        assert_eq!(
            CdrResponse::from_parts(http::StatusCode::OK, &empty, String::new()).etag_version_uid(),
            None
        );
    }
}
