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

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use crate::error::ViewerError;
use crate::locale::console::ConsoleMsg;
use crate::locale::tr;
use crate::locale::tr_with;
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
    // A server fn is a public endpoint: the family must be one of the documents
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

/// The fixed count-AQL behind the repository-usage card: one row per template,
/// the template id supplied as the `$template` binding.
///
/// The id is never concatenated into the text. It is CDR-supplied but
/// operator-authored data, and AQL escapes inside a literal with a backslash
/// (`docs/specs/openehr/QUERY/docs/AQL/master03-syntax.adoc` §LIKE — "escaped
/// by using the backslash `\` character"), so no quote-doubling makes
/// interpolation safe; the binding travels in `query_parameters`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/query/Request.md`
/// §Query parameters) and is matched as data.
const TEMPLATE_USAGE_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c WHERE c/archetype_details/template_id/value = $template";

/// Build the `POST query/aql` request body counting the compositions committed
/// under `template_id`.
///
/// The AQL text is [`TEMPLATE_USAGE_AQL`] verbatim for every template; only the
/// `query_parameters` object differs.
fn template_usage_body(template_id: &str) -> String {
    crate::bff::ehrs::aql_request_body(
        TEMPLATE_USAGE_AQL,
        &serde_json::json!({ "template": template_id }),
        0,
        crate::bff::PAGE_SIZE,
    )
}

/// How many templates the repository-usage card counts.
const USAGE_TEMPLATES: usize = 25;

/// Count the compositions committed under one template, via `POST query/aql`.
async fn template_count(
    state: &crate::state::AppState,
    session: &crate::session::ViewerSession,
    template_id: String,
) -> Result<(String, i64), ViewerError> {
    let url = state.cdr.rest_v1("query/aql");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            template_usage_body(&template_id),
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let count = serde_json::from_str::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|v| {
            v.get("rows")?
                .as_array()?
                .first()?
                .as_array()?
                .first()?
                .as_i64()
        })
        .unwrap_or(0);
    Ok((template_id, count))
}

/// Per-template composition counts ("repo usage") — one count AQL per template,
/// plain AQL (no CDR stats endpoint exists).
///
/// Bounded to the first `USAGE_TEMPLATES` templates, sorted by count
/// descending; the second member is the repository's total template count, so
/// the card can say when it is showing a truncated list.
///
/// The counts fan out with the shared bounded concurrency
/// ([`FANOUT_CONCURRENCY`](crate::cdr::FANOUT_CONCURRENCY)) rather than a serial
/// await chain. `buffered` yields in input order, so the sort below still breaks
/// ties by the template listing's order, and `try_collect` keeps the serial
/// loop's short circuit.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR errors
/// normalized by the underlying calls.
pub async fn template_usage() -> Result<(Vec<(String, i64)>, u32), ViewerError> {
    use futures::stream::{StreamExt, TryStreamExt};

    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let templates = crate::bff::templates::list_templates().await?;
    let total = u32::try_from(templates.len()).unwrap_or(u32::MAX);
    // Borrowed once outside the stream: each fan-out future captures the two
    // references by copy, which keeps the read's future sized.
    let state = &state;
    let session = &session;
    let mut usage: Vec<(String, i64)> = futures::stream::iter(
        templates
            .into_iter()
            .take(USAGE_TEMPLATES)
            .map(|row| template_count(state, session, row.template_id)),
    )
    .buffered(crate::cdr::FANOUT_CONCURRENCY)
    .try_collect()
    .await?;
    usage.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    Ok((usage, total))
}

/// What to say when the SMART discovery probe could not be read.
///
/// Component-free and unit-tested (the viewer's failure-copy convention —
/// `crate::feedback::write_failure_copy`): every branch names what was asked
/// for and what to do next, and none of them echoes a bare status code back
/// at the reader.
#[must_use]
pub fn smart_probe_failure_copy(error: &ViewerError) -> String {
    let endpoint = format!("/{SMART_DISCOVERY_PATH}");
    match error {
        ViewerError::CdrUnreachable(detail) => tr_with(
            ConsoleMsg::SmartDiscoveryCouldNotReadCdr,
            &[("detail", detail.clone().into())],
        ),
        ViewerError::CdrUnauthorized(_) | ViewerError::Unauthenticated => tr_with(
            ConsoleMsg::SmartDiscoveryCouldNotReadCdr2,
            &[("endpoint", endpoint.clone().into())],
        ),
        ViewerError::Forbidden(_) => tr_with(
            ConsoleMsg::SmartDiscoveryCouldNotReadSession,
            &[("endpoint", endpoint.clone().into())],
        ),
        _ => {
            // A CDR body that only restates its own status ("HTTP 302") adds
            // nothing, so it is dropped rather than printed beside the code it
            // repeats.
            let detail = match error {
                ViewerError::Cdr { status, message } => {
                    let restatement =
                        tr_with(ConsoleMsg::Http, &[("status", (status).to_string().into())]);
                    if message.trim().is_empty() || message.trim() == restatement {
                        tr_with(
                            ConsoleMsg::TheCdrAnsweredThere,
                            &[("status", (status).to_string().into())],
                        )
                    } else {
                        tr_with(
                            ConsoleMsg::TheCdrAnsweredThere2,
                            &[
                                ("status", (status).to_string().into()),
                                ("message", message.clone().into()),
                            ],
                        )
                    }
                }
                other => other.to_string(),
            };
            tr_with(
                ConsoleMsg::SmartDiscoveryCouldNotReadFrom,
                &[
                    ("endpoint", endpoint.clone().into()),
                    ("detail", detail.clone().into()),
                ],
            )
        }
    }
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
    use super::{
        SMART_DISCOVERY_PATH, group_openapi_paths, openapi_families, smart_probe_failure_copy,
    };
    use crate::error::ViewerError;
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

    /// No failure branch echoes a bare status back at the reader; each names
    /// what was probed (or why that is moot) and what to do next.
    #[test]
    fn a_failed_probe_reads_as_actionable_copy_never_a_status_echo() {
        let cases = [
            ViewerError::CdrUnreachable("connection refused".to_owned()),
            ViewerError::CdrUnauthorized("expired".to_owned()),
            ViewerError::Forbidden("missing role".to_owned()),
            ViewerError::Unauthenticated,
            ViewerError::Cdr {
                status: 302,
                message: "HTTP 302".to_owned(),
            },
        ];
        for error in cases {
            let copy = smart_probe_failure_copy(&error);
            assert!(
                copy.starts_with("SMART discovery could not be read"),
                "{copy}"
            );
            assert!(!copy.contains("CDR answered 302: HTTP 302"), "{copy}");
            // Either the endpoint is named, or the branch explains why naming
            // it would not help (the CDR was never reached).
            assert!(
                copy.contains(SMART_DISCOVERY_PATH) || copy.contains("unreachable"),
                "{copy}"
            );
        }
    }

    /// An unexpected answer states that "enabled" is UNKNOWN — the one thing
    /// the card must not do is imply SMART is off when it could not tell.
    #[test]
    fn an_unexpected_answer_does_not_claim_smart_is_disabled() {
        let copy = smart_probe_failure_copy(&ViewerError::Cdr {
            status: 302,
            message: "HTTP 302".to_owned(),
        });
        assert!(copy.contains("unknown"), "{copy}");
        assert!(!copy.contains("not enabled"), "{copy}");
    }

    #[test]
    fn a_template_id_travels_as_a_binding_and_never_as_query_text() {
        use super::{TEMPLATE_USAGE_AQL, template_usage_body};

        let read = |body: &str| -> serde_json::Value {
            serde_json::from_str(body).expect("the request body is JSON")
        };
        let benign = read(&template_usage_body("vital_signs.v2"));
        // A quote and a backslash are AQL syntax, so an id carrying either one
        // must not change a single byte of the statement.
        for hostile in [
            "o'brien.v1",
            r"back\slash.v1",
            "x' OR 1=1 --",
            r"x\' OR 1=1 --",
        ] {
            let sent = read(&template_usage_body(hostile));
            assert_eq!(
                sent.get("q"),
                benign.get("q"),
                "the AQL text must be identical for {hostile:?}"
            );
            assert_eq!(sent["q"], serde_json::json!(TEMPLATE_USAGE_AQL));
            assert_eq!(sent["query_parameters"]["template"], hostile);
        }
        // The statement names the parameter, and quotes no value at all.
        assert!(
            TEMPLATE_USAGE_AQL.contains("= $template"),
            "{TEMPLATE_USAGE_AQL}"
        );
        assert!(!TEMPLATE_USAGE_AQL.contains('\''), "{TEMPLATE_USAGE_AQL}");
    }

    #[test]
    fn the_repository_usage_statement_is_valid_aql() {
        // The fixed text is parsed by the real grammar, so a typo in the
        // constant fails here rather than as a CDR 400 on the card.
        openehr_query::parser::parse_str(super::TEMPLATE_USAGE_AQL).expect("the count AQL parses");
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
