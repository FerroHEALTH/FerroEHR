// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/system` screen.
//!
//! An operational panel over the CDR — server status, the openEHR System API
//! conformance manifest, SMART service-discovery, repository usage, the CDR's
//! own served `OpenAPI` documents per API family (rendered by our own
//! grouped-endpoint component, never a Swagger embed), the redacted runtime
//! configuration, and a link into the `/audit` activity browser.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The wire it reads IS spec-bound: the conformance manifest is the STABLE
//! ITS-REST 1.1.0 System API (`OPTIONS {base_path}` — see
//! [`crate::system_api`], whose fetcher this screen shares with the
//! admin-capability probe rather than fetching the manifest twice), and the
//! SMART discovery document follows
//! `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`.
//!
//! One failing card never blanks the page: each card draws its own `Result`.
//! Every read guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches the page.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
use crate::locale::tr;
use crate::served_vocabulary::OPENAPI_FAMILIES;

/// Where the CDR serves its SMART service-discovery document, relative to the
/// CDR's product root.
///
/// The document hangs off the PLATFORM base URL, which this platform gives a
/// path segment: ITS-REST `smart_app_launch/master04-service_discovery.adoc`
/// §"the configuration endpoint" — "If the base URL includes a path segment
/// as `https://platform.example.com/gateway/v1`, then the configuration
/// should be accessible at
/// `https://platform.example.com/gateway/v1/.well-known/smart-configuration`".
/// That segment is deployment-configured (`cdr.base_path`), so the absolute
/// path is derived per deployment rather than spelled here.
const SMART_DISCOVERY_PATH: &str = ".well-known/smart-configuration";

/// The CDR's SMART service-discovery document, or `None` when the CDR
/// advertises none (a `404` is a first-class "SMART disabled" state, not an
/// error).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_smart_config() -> Result<Option<String>, ViewerError> {
    crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_root_url(SMART_DISCOVERY_PATH);
    let response = state.cdr.get_public(&url, "application/json").await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// Returns the OpenAPI documents the selector offers, as `(slug, label)`: the
/// complete surface first under the empty slug, then every API family.
///
/// NOTE: no openEHR spec governs an OAS-serving endpoint — our own design.
/// The CDR serves each family it filters out of its OWN generated document as
/// `api-docs/ferroehr-{slug}.openapi.json`; the family list is the shared
/// [`OPENAPI_FAMILIES`], which the server holds its own family set to, and a
/// deployment that does not serve a family answers `404`, a first-class state
/// on the card.
pub(crate) fn openapi_families() -> impl Iterator<Item = (&'static str, String)> {
    std::iter::once(("", tr(ConsoleMsg::CompleteSurface))).chain(
        OPENAPI_FAMILIES
            .into_iter()
            .map(|(slug, label)| (slug, label.to_owned())),
    )
}

/// One of the CDR's own natively served OpenAPI documents, raw JSON: the
/// complete surface for an empty `family`, otherwise that API family's
/// filtered document.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for a `family` the CDR has no document for; CDR
/// transport errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_openapi(
    // Which served OpenAPI document to read, as an API-family slug.
    family: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    // A page is a public endpoint: the family must be one of the documents
    // we know the CDR serves, never a caller-shaped path segment.
    let family = family.trim();
    if !openapi_families().any(|(slug, _)| slug == family) {
        return Err(ViewerError::Invalid(format!(
            "{family:?} is not an API family the CDR serves a document for"
        )));
    }
    // NOTE: no openEHR spec governs an OAS-serving endpoint — our own design;
    // the CDR serves only its own natively generated documents, under the
    // `api-docs/` directory of its product root.
    let url = if family.is_empty() {
        state.cdr.rest_root_url("api-docs/openapi.json")
    } else {
        state
            .cdr
            .rest_root_url(&format!("api-docs/ferroehr-{family}.openapi.json"))
    };
    let response = state.cdr.get_public(&url, "application/json").await?;
    // Public surface; if a deployment happens to gate it, retry with the
    // session credential before giving up.
    // NOTE: the credential's audience is exactly the configured CDR origin —
    // `url` is built by `rest_root_url`, so the retry can only ever re-send it
    // to the same host the session authenticated against.
    let response = if response.is(http::StatusCode::UNAUTHORIZED) {
        state
            .cdr
            .get(&session.credential, &url, "application/json")
            .await?
    } else {
        response
    };
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}

/// The redacted effective CDR configuration (`GET /admin/config` — the CDR's own
/// extension endpoint; secrets are redacted structurally server-side, this fn
/// only relays).
///
/// A `404` means the admin API is disabled and `401`/`403` that the session
/// lacks the ADMIN role — both first-class rendered states, not failures.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR
/// transport errors pass through; non-2xx answers normalize via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_admin_config() -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.extensions_v1("admin/config");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(serde_json::from_str::<serde_json::Value>(&body)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or(body))
}

/// One operation of an `OpenAPI` document: its method, its path and its
/// summary.
pub(crate) type Endpoint = (String, String, String);

/// Group an `OpenAPI` document's operations by their first `tag` (falling back
/// to the first path segment, then `"default"`), as sorted `(group, rows)`
/// where each row is `(METHOD, path)`. Sorted throughout so server and client
/// render the identical structure (no non-deterministic order).
pub(crate) fn group_openapi_paths(doc: &serde_json::Value) -> Vec<(String, Vec<Endpoint>)> {
    const METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "patch", "options", "head", "trace",
    ];
    let mut groups: std::collections::BTreeMap<String, Vec<Endpoint>> =
        std::collections::BTreeMap::new();
    let Some(paths) = doc.get("paths").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    for (path, item) in paths {
        let Some(methods) = item.as_object() else {
            continue;
        };
        for method in METHODS {
            let Some(op) = methods.get(method) else {
                continue;
            };
            let group = op
                .get("tags")
                .and_then(serde_json::Value::as_array)
                .and_then(|tags| tags.first())
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    path.trim_start_matches('/')
                        .split('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "default".to_owned());
            let summary = op
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            groups
                .entry(group)
                .or_default()
                .push((method.to_uppercase(), path.clone(), summary));
        }
    }
    let mut out: Vec<(String, Vec<Endpoint>)> = groups.into_iter().collect();
    for (_, rows) in &mut out {
        rows.sort();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{SMART_DISCOVERY_PATH, group_openapi_paths, openapi_families};
    use crate::served_vocabulary::OPENAPI_FAMILIES;

    /// The probe asks for the path the CDR actually serves: relative to the
    /// PLATFORM base URL, which carries a path segment here (ITS-REST
    /// `smart_app_launch/master04-service_discovery.adoc` §"the configuration
    /// endpoint"). The origin-level path is a different resource, and on a
    /// single-origin deployment it is not the CDR's at all. The constant is
    /// therefore root-relative, and `CdrClient::rest_root_url` supplies that
    /// segment from `cdr.base_path`.
    #[test]
    fn discovery_is_probed_under_the_platform_base_path() {
        assert_eq!(SMART_DISCOVERY_PATH, ".well-known/smart-configuration");
    }

    #[test]
    fn the_family_selector_offers_the_complete_document_first_and_unique_slugs() {
        assert_eq!(
            openapi_families().next().map(|(slug, _)| slug),
            Some(""),
            "the complete surface is the default (empty) slug"
        );
        let mut slugs: Vec<&str> = openapi_families().map(|(slug, _)| slug).collect();
        let count = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), count, "family slugs must be unique");
    }

    /// The selector offers every family the server serves (the shared list the
    /// server's own family set is held to), each under the server's selector
    /// name and in its order, `messaging` included.
    #[test]
    fn the_family_selector_offers_every_served_family() {
        let offered: Vec<(&str, String)> = openapi_families().skip(1).collect();
        let served: Vec<(&str, String)> = OPENAPI_FAMILIES
            .into_iter()
            .map(|(slug, label)| (slug, label.to_owned()))
            .collect();
        assert_eq!(offered, served);
        assert!(
            offered
                .iter()
                .any(|(slug, label)| *slug == "messaging" && *label == "FerroEHR: Messaging"),
            "{offered:?}"
        );
    }

    #[test]
    fn groups_by_tag_then_path_segment_sorted() {
        let doc = serde_json::json!({
            "paths": {
                "/ehr/{ehr_id}": {
                    "put": {"tags": ["EHR"], "summary": "Create an EHR with an id"},
                    "get": {"tags": ["EHR"]}
                },
                "/query/aql": {"post": {"tags": ["Query"]}},
                "/misc": {"get": {}},
                "/misc/thing": {"parameters": []}
            }
        });
        let groups = group_openapi_paths(&doc);
        let names: Vec<String> = groups.iter().map(|(g, _)| g.clone()).collect();
        // BTreeMap keys sorted: "EHR", "Query", then the path-segment fallback "misc".
        assert_eq!(names, vec!["EHR", "Query", "misc"]);
        let ehr = groups
            .iter()
            .find(|(g, _)| g == "EHR")
            .map(|(_, r)| r.clone())
            .expect("EHR group present");
        // Methods sorted within a path (GET before PUT).
        assert_eq!(
            ehr,
            vec![
                ("GET".to_owned(), "/ehr/{ehr_id}".to_owned(), String::new()),
                (
                    "PUT".to_owned(),
                    "/ehr/{ehr_id}".to_owned(),
                    "Create an EHR with an id".to_owned()
                ),
            ]
        );
        // A path item with only non-operation keys contributes nothing.
        assert!(
            groups
                .iter()
                .all(|(_, rows)| rows.iter().all(|(_, p, _)| p != "/misc/thing"))
        );
    }

    #[test]
    fn empty_document_yields_no_groups() {
        assert_eq!(
            group_openapi_paths(&serde_json::json!({})),
            Vec::<(String, Vec<(String, String, String)>)>::new()
        );
        assert_eq!(
            group_openapi_paths(&serde_json::json!({"paths": {}})),
            Vec::<(String, Vec<(String, String, String)>)>::new()
        );
    }
}
