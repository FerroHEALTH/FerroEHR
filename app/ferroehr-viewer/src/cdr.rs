// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The BFF's CDR client.
//!
//! One `reqwest` client, the ITS-REST base path, credential injection, strict
//! `Accept`/`Content-Type` negotiation (`crate::format`), and error
//! normalization into [`crate::error::ViewerError`]. The viewer reaches the
//! CDR ONLY here.

mod diagnostic;

use crate::error::ViewerError;
use crate::session::Credential;
use diagnostic::{diagnostic_of, without_lead};

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
/// typed client: `ETag` is the state identifier a versioned resource carries
/// and `Location` is the created resource's URL
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §HTTP headers).
#[derive(Debug, Clone)]
pub struct CdrResponse {
    /// The HTTP status the CDR answered with.
    pub status: http::StatusCode,
    /// The `ETag` verbatim, weakness indicator and quotes included — read
    /// through [`Self::etag_version_uid`] rather than by hand.
    pub etag: Option<String>,
    /// The `Location` of a newly created resource (`201` answers only).
    pub location: Option<String>,
    /// The raw body bytes as text (all ITS-REST representations are text).
    pub body: String,
}

impl CdrResponse {
    /// This answer when it is `2xx`, otherwise the normalized error, extracting
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
    /// A `401` also ends the page's session ([`crate::session_client`]).
    ///
    /// # Errors
    /// [`ViewerError::CdrUnauthorized`] on 401, [`ViewerError::Forbidden`] on
    /// 403, [`ViewerError::Cdr`] on any other non-2xx status.
    pub fn ok(self) -> Result<Self, ViewerError> {
        if self.status.is_success() {
            return Ok(self);
        }
        let message = diagnostic_of(&self);
        match self.status {
            http::StatusCode::UNAUTHORIZED => {
                let refused = ViewerError::CdrUnauthorized(without_lead(&message, "unauthorized"));
                crate::session_client::note_if_over(&refused);
                Err(refused)
            }
            http::StatusCode::FORBIDDEN => {
                Err(ViewerError::Forbidden(without_lead(&message, "forbidden")))
            }
            status => Err(ViewerError::Cdr {
                status: status.as_u16(),
                message,
            }),
        }
    }

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
            etag: header_text(headers, http::header::ETAG.as_str()),
            location: header_text(headers, http::header::LOCATION.as_str()),
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

/// The CDR client could not be built from its configuration.
#[derive(Debug, thiserror::Error)]
pub enum CdrClientError {
    /// `[cdr.tls]` names material that does not read.
    #[error("the CDR client's TLS material could not be read")]
    Tls(#[from] ferrohealth_server::tls_client::TlsClientError),
    /// The HTTP client refused its settings.
    #[error("the CDR HTTP client could not be built")]
    Build(#[from] reqwest::Error),
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
    /// Build the client from configuration: the platform's trust roots, the
    /// extra roots and the client identity `[cdr.tls]` names, no redirect.
    ///
    /// # Errors
    /// [`CdrClientError::Tls`] when a `[cdr.tls]` file does not read or holds
    /// no PEM of its kind, and [`CdrClientError::Build`] when the HTTP client
    /// refuses its settings.
    pub fn new(cfg: &crate::config::CdrConfig) -> Result<Self, CdrClientError> {
        let tls = ferrohealth_server::tls_client::TlsClient::read("cdr.tls", &cfg.tls)?;
        let builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.request_timeout_secs))
            .redirect(reqwest::redirect::Policy::none());
        let http = tls.apply(builder)?.build()?;
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

    /// A URL under the CDR's extensions root (`{rest_root}/extensions/v1`),
    /// where the CDR serves its own API extensions beside the openEHR API:
    /// terminology, event subscriptions, party relationships, the archetype
    /// library and the admin extensions.
    #[must_use]
    pub fn extensions_v1(&self, path: &str) -> String {
        self.rest_root_url(&format!("extensions/v1/{}", path.trim_start_matches('/')))
    }

    /// A URL under the CDR's FHIR root (`{rest_root}/fhir/r4`), where it
    /// serves the IHE ITI-81 `AuditEvent` retrieval.
    #[must_use]
    pub fn fhir_r4(&self, path: &str) -> String {
        self.rest_root_url(&format!("fhir/r4/{}", path.trim_start_matches('/')))
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

    /// One request to `url` as `credential`, asking for `application/json`
    /// until [`CdrCall::accept`] says otherwise.
    pub fn call(&self, method: http::Method, url: &str, credential: &Credential) -> CdrCall {
        CdrCall::new(self.http.clone(), method, url, Some(credential.clone()))
    }

    /// One request to `url` with no credential, for the surfaces the CDR
    /// serves outside authentication: `/status`, the SMART discovery
    /// document and the System API's conformance manifest (ITS-REST System
    /// API, `security: []`).
    pub fn public(&self, method: http::Method, url: &str) -> CdrCall {
        CdrCall::new(self.http.clone(), method, url, None)
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
}

/// One request to the CDR, built up and sent once.
///
/// Every request asks for `application/json` unless [`Self::accept`] names
/// another representation, so a refusal carries the openEHR diagnostic in the
/// format the viewer reads.
#[derive(Debug)]
#[must_use]
pub struct CdrCall {
    http: reqwest::Client,
    method: http::Method,
    url: String,
    credential: Option<Credential>,
    accept: String,
    headers: Vec<(String, String)>,
    body: Option<(String, String)>,
}

impl CdrCall {
    fn new(
        http: reqwest::Client,
        method: http::Method,
        url: &str,
        credential: Option<Credential>,
    ) -> Self {
        Self {
            http,
            method,
            url: url.to_owned(),
            credential,
            accept: "application/json".to_owned(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Asks for `accept` in place of `application/json`.
    pub fn accept(mut self, accept: &str) -> Self {
        accept.clone_into(&mut self.accept);
        self
    }

    /// Adds the request headers `headers` (`If-Match`, `Prefer`,
    /// `openehr-template-id`).
    pub fn headers(mut self, headers: &[(&str, &str)]) -> Self {
        self.headers.extend(
            headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        self
    }

    /// Sends `body` as `content_type`.
    pub fn body(mut self, content_type: &str, body: String) -> Self {
        self.body = Some((content_type.to_owned(), body));
        self
    }

    /// Sends the request and returns the CDR's answer, whatever its status.
    ///
    /// # Errors
    /// [`ViewerError::CdrUnreachable`] when the CDR does not answer or its
    /// body does not read.
    pub async fn send(self) -> Result<CdrResponse, ViewerError> {
        let mut request = self
            .http
            .request(self.method, &self.url)
            .header(http::header::ACCEPT, self.accept);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some((content_type, body)) = self.body {
            request = request
                .header(http::header::CONTENT_TYPE, content_type)
                .body(body);
        }
        request = match &self.credential {
            Some(Credential::Basic { username, password }) => {
                request.basic_auth(username, Some(password))
            }
            Some(Credential::Bearer { access_token }) => request.bearer_auth(access_token),
            None => request,
        };
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

    /// Sends the request and returns a `2xx` answer.
    ///
    /// # Errors
    /// The errors of [`Self::send`], and any other answer normalized by
    /// [`CdrResponse::ok`].
    pub async fn ok(self) -> Result<CdrResponse, ViewerError> {
        self.send().await?.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{CdrClient, CdrResponse, strip_etag};
    use crate::error::ViewerError;

    /// A served version identifier, in the shape the overview's own `ETag`
    /// example carries.
    const UID: &str = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";

    fn response(status: http::StatusCode, body: &str) -> CdrResponse {
        CdrResponse {
            status,
            etag: None,
            location: None,
            body: body.to_owned(),
        }
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
        let unauthorized = response(
            http::StatusCode::UNAUTHORIZED,
            r#"{"status":401,"detail":"the bearer token has expired"}"#,
        )
        .ok()
        .unwrap_err();
        assert_eq!(
            unauthorized,
            ViewerError::CdrUnauthorized("the bearer token has expired".to_owned())
        );
        assert_eq!(
            unauthorized.status_code(),
            Some(http::StatusCode::UNAUTHORIZED)
        );

        let err = response(
            http::StatusCode::FORBIDDEN,
            r#"{"status":403,"detail":"scope"}"#,
        )
        .ok()
        .unwrap_err();
        assert_eq!(err, ViewerError::Forbidden("scope".to_owned()));
        assert_eq!(err.status_code(), Some(http::StatusCode::FORBIDDEN));

        // A diagnostic that repeats the refusal's own word reads once.
        let err = response(
            http::StatusCode::FORBIDDEN,
            r#"{"status":403,"detail":"forbidden: operation requires the 'ADMIN' role"}"#,
        )
        .ok()
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "forbidden: operation requires the 'ADMIN' role"
        );
        let err = response(
            http::StatusCode::FORBIDDEN,
            r#"{"status":403,"detail":"forbidden"}"#,
        )
        .ok()
        .unwrap_err();
        assert_eq!(err.to_string(), "forbidden: the CDR refused this session");

        let err = response(http::StatusCode::NOT_FOUND, "").ok().unwrap_err();
        assert_eq!(
            err,
            ViewerError::Cdr {
                status: 404,
                message: "HTTP 404".to_owned()
            }
        );
        // The typed status survives the round trip into the error's own reader.
        assert_eq!(err.status_code(), Some(http::StatusCode::NOT_FOUND));
        assert!(response(http::StatusCode::CREATED, "ok").ok().is_ok());
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
        assert_eq!(response.etag.as_deref(), Some("W/\"7d44::sys::3\""));
        assert_eq!(
            response.location.as_deref(),
            Some("https://cdr.example.org/ehr/1/composition/7d44")
        );
        // The If-Match round-trip reads the identifier, never the raw field.
        assert_eq!(response.etag_version_uid().as_deref(), Some("7d44::sys::3"));

        // An answer with no ETag at all offers no identifier to echo back.
        let bare =
            CdrResponse::from_parts(http::StatusCode::OK, &http::HeaderMap::new(), String::new());
        assert_eq!(bare.etag_version_uid(), None);
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
