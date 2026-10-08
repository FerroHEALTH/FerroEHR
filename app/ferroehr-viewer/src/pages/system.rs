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
//! One failing card never blanks the page: each resolves its `Result` inside
//! its own `<Suspense>` rather than through an `<ErrorBoundary>`, whose SSR'd
//! fallback mismatches at hydration in leptos 0.8. Every co-located
//! `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;

use leptos_router::components::A;

use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_SECONDARY, SELECT};
use crate::components::page_header::PageHeader;
use crate::components::surface::titled_card;
use crate::error::ViewerError;

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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_smart_config() -> Result<Option<String>, ViewerError> {
    crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_root_url(SMART_DISCOVERY_PATH);
    let response = state.cdr.get_public(&url, "application/json").await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

/// The API-family documents the CDR serves beside the complete one, as
/// `(slug, label)` with the empty slug standing for the complete surface.
///
/// NOTE: no openEHR spec governs an OAS-serving endpoint — our own design /
/// product extension. The CDR filters one document per API family out of its
/// OWN generated document (never a vendored OAS) and serves each as
/// `api-docs/ferroehr-{slug}.openapi.json`; a deployment that does not serve a
/// family answers `404`, which the card renders as a first-class state rather
/// than an error.
const OPENAPI_FAMILIES: &[(&str, &str)] = &[
    ("", "Complete surface"),
    ("ehr", "openEHR — EHR"),
    ("query", "openEHR — Query"),
    ("definition", "openEHR — Definition"),
    ("demographic", "openEHR — Demographic"),
    ("admin", "openEHR — Admin"),
    ("management", "FerroEHR — Status & Management"),
    ("terminology", "FerroEHR — Terminology"),
    ("relationships", "FerroEHR — Party Relationships"),
    ("events", "FerroEHR — Event Subscriptions"),
    ("audit", "FerroEHR — Audit"),
    ("smart", "FerroEHR — SMART Discovery"),
];

/// The known family slug `value` names, or the empty slug (the complete
/// document) for anything else — an unknown `?openapi=` value in the address
/// bar is user input, so it degrades to the default instead of failing.
#[must_use]
fn openapi_family_slug(value: &str) -> String {
    OPENAPI_FAMILIES
        .iter()
        .find(|(slug, _)| *slug == value)
        .map_or_else(String::new, |(slug, _)| (*slug).to_owned())
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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_openapi(
    /// Which served OpenAPI document to read, as an API-family slug.
    family: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    // A server fn is a public endpoint: the family must be one of the documents
    // we know the CDR serves, never a caller-shaped path segment.
    let family = family.trim();
    if !OPENAPI_FAMILIES.iter().any(|(slug, _)| *slug == family) {
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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_admin_config() -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1("admin/config");
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
#[cfg(feature = "ssr")]
const TEMPLATE_USAGE_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c \
                                  WHERE c/archetype_details/template_id/value = $template";

#[cfg(feature = "ssr")]
/// Build the `POST query/aql` request body counting the compositions committed
/// under `template_id`.
///
/// The AQL text is [`TEMPLATE_USAGE_AQL`] verbatim for every template; only the
/// `query_parameters` object differs.
fn template_usage_body(template_id: &str) -> String {
    crate::pages::ehrs::aql_request_body(
        TEMPLATE_USAGE_AQL,
        &serde_json::json!({ "template": template_id }),
        0,
    )
}

/// How many templates the repository-usage card counts.
#[cfg(feature = "ssr")]
const USAGE_TEMPLATES: usize = 25;

/// Count the compositions committed under one template, via `POST query/aql`.
#[cfg(feature = "ssr")]
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

/// Per-template composition counts ("repo usage") — one count AQL per
/// template, plain AQL (no CDR stats endpoint exists). Bounded to the first
/// `USAGE_TEMPLATES` templates, sorted by count descending; the second
/// member is the repository's total template count, so the card can say
/// when it is showing a truncated list.
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
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn template_usage() -> Result<(Vec<(String, i64)>, u32), ViewerError> {
    use futures::stream::{StreamExt, TryStreamExt};

    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let templates = crate::pages::templates::list_templates().await?;
    let total = u32::try_from(templates.len()).unwrap_or(u32::MAX);
    // Borrowed once outside the stream: each fan-out future captures the two
    // references by copy, which keeps the `#[server]` boundary's future sized.
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

/// Renders the `/system` screen: the CDR's status document, the
/// conformance-manifest card (product identity, claimed profile, mounted API
/// groups), the SMART discovery card, the served-OpenAPI viewer for the family
/// named by `?openapi=`, the redacted configuration, and the manufacturer card.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn SystemPage() -> impl IntoView {
    // Which OpenAPI document the card shows lives in the URL
    // (`?openapi=`): shareable, refresh-safe, and selectable before WASM
    // loads (the selector is a plain GET form — the audit filter's
    // pattern).
    let query = leptos_router::hooks::use_query_map();
    let family = Memo::new(move |_| {
        openapi_family_slug(&query.with(|q| q.get("openapi").unwrap_or_default()))
    });

    let status = status_card();
    let manifest = manifest_card();
    let smart = smart_card();
    let openapi = openapi_card(family);
    let activity = activity_log_card();
    let usage = usage_card();
    let config = config_card();
    let manufacturer = manufacturer_card();

    view! {
        <Title text="System" />
        <div class="p-6">
            <PageHeader
                title="System"
                subtitle="CDR status, the openEHR conformance manifest, SMART discovery, repository usage, the served OpenAPI documents, the redacted runtime configuration, and the manufacturer."
            />
            <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">
                {status} {manifest} {smart} {usage} {openapi} {config} {activity} {manufacturer}
            </div>
        </div>
    }
}

/// The manufacturer card: the name, postal address, single point of contact
/// and website of the manufacturer of this viewer, as Regulation (EU) 2025/327
/// (`docs/law/eu/ehds/text.html` Art. 30(1)(g)) asks the EHR system to show
/// them.
///
/// Static, compiled in from [`crate::manufacturer::MANUFACTURER`] on both
/// targets, so the server pass and hydration render the same markup and the
/// card needs no request. The footer links here through `#manufacturer`.
fn manufacturer_card() -> AnyView {
    let manufacturer = crate::manufacturer::MANUFACTURER;
    let body = view! {
        <dl id="manufacturer" class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
            <dt class="font-medium text-ink-muted">"name"</dt>
            <dd class="text-ink">{manufacturer.name}</dd>
            <dt class="font-medium text-ink-muted">"postal address"</dt>
            <dd class="text-ink">{manufacturer.postal_address}</dd>
            <dt class="font-medium text-ink-muted">"contact"</dt>
            <dd class="break-all">
                <a
                    href=format!("mailto:{}", manufacturer.email)
                    class="text-accent-ink hover:underline"
                >
                    {manufacturer.email}
                </a>
            </dd>
            <dt class="font-medium text-ink-muted">"website"</dt>
            <dd class="break-all">
                <a href=manufacturer.website class="text-accent-ink hover:underline">
                    {manufacturer.website}
                </a>
            </dd>
        </dl>
    }
    .into_any();
    titled_card("Manufacturer", false, body)
}

/// The conformance-manifest card: what the CDR advertises about ITSELF through
/// the openEHR **System API** (`OPTIONS {base_path}` → the `Options` document),
/// consuming the shared [`fetch_conformance_manifest`](crate::system_api::fetch_conformance_manifest)
/// — the same reader the admin-capability probe uses, never a second fetcher.
///
/// One reader per claim: the manifest and the status document both carry a
/// product version and an ITS-REST version, so this card shows only what the
/// System API alone knows — the product identity, the claimed conformance
/// profile, and the API groups the server mounts — and points at the Status card
/// for the versions.
fn manifest_card() -> AnyView {
    let resource = Resource::new(
        || (),
        |()| async move { crate::system_api::fetch_conformance_manifest().await },
    );
    let body = view! {
        <Suspense fallback=card_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(manifest) => manifest_body(&manifest),
                    Err(e) => card_error(&e),
                }
            })}
        </Suspense>
    }
    .into_any();
    titled_card("Conformance manifest", false, body)
}

/// Render the `Options` document: the product identity and claimed conformance
/// profile as a definition list, and one chip per mounted API group.
fn manifest_body(manifest: &crate::system_api::ConformanceManifest) -> AnyView {
    let facts = [
        ("solution", manifest.solution.clone()),
        ("vendor", manifest.vendor.clone()),
        ("conformance profile", manifest.conformance_profile.clone()),
    ]
    .into_iter()
    .map(|(label, value)| {
        let shown = if value.is_empty() {
            "—".to_owned()
        } else {
            value
        };
        view! {
            <dt class="font-medium text-ink-muted">{label}</dt>
            <dd class="font-mono break-all text-ink">{shown}</dd>
        }
    })
    .collect::<Vec<_>>();
    let groups = manifest
        .endpoints
        .clone()
        .into_iter()
        .map(|endpoint| {
            let hook = endpoint.clone();
            view! {
                <span
                    class="rounded-full bg-accent-subtle px-2 py-0.5 font-mono text-xs text-accent-ink"
                    data-manifest-endpoint=hook
                >
                    {endpoint}
                </span>
            }
        })
        .collect::<Vec<_>>();
    let empty_groups = manifest.endpoints.is_empty();
    view! {
        <div id="conformance-manifest">
            <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">{facts}</dl>
            <div class="mt-3">
                <div class="text-xs font-medium text-ink-muted mb-1">"Mounted API groups"</div>
                <div class="flex flex-wrap gap-1">{groups}</div>
                {empty_groups
                    .then(|| {
                        view! {
                            <p class="text-sm text-ink-muted">
                                "This CDR advertises no API groups in its manifest."
                            </p>
                        }
                    })}
            </div>
            <p class="mt-3 text-xs text-ink-muted">
                "The product and openEHR REST versions are in the Status card — the manifest and the status document report the same versions, so the viewer reads them in one place."
            </p>
        </div>
    }
    .into_any()
}

/// The repo-usage card: per-template composition counts.
fn usage_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { template_usage().await });
    let body = view! {
        <Suspense fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-24" />
                </thaw::Skeleton>
            }
        }>
            {move || {
                Suspend::new(async move {
                    match resource.await {
                        Ok((rows, _)) if rows.is_empty() => {
                            view! {
                                <EmptyState
                                    icon=icondata_lu::LuFileCode2
                                    message="No template usage yet"
                                    hint="Counts appear once compositions are committed against a template."
                                />
                            }
                                .into_any()
                        }
                        Ok((rows, total)) => {
                            let shown = rows.len();
                            let truncated = usize::try_from(total).is_ok_and(|t| t > shown);
                            let body = rows
                                .into_iter()
                                .map(|(template_id, count)| {
                                    view! {
                                        <tr class=crate::components::data_table::ROW>
                                            <td class=crate::components::data_table::CELL_MONO>
                                                {template_id}
                                            </td>
                                            <td class="px-3 py-2 text-right tabular-nums">{count}</td>
                                        </tr>
                                    }
                                        .into_any()
                                })
                                .collect_view()
                                .into_any();
                            let table = crate::components::data_table::table_shell(
                                &["Template", "Compositions"],
                                body,
                            );
                            if truncated {
                                view! {
                                    <div>
                                        {table}
                                        <p class="mt-2 text-xs text-ink-muted">
                                            {format!(
                                                "Showing the {shown} busiest of {total} templates.",
                                            )}
                                        </p>
                                    </div>
                                }
                                    .into_any()
                            } else {
                                table
                            }
                        }
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();
    titled_card("Repository usage", false, body)
}

/// The runtime-configuration card: the CDR's redacted effective config
/// (read-only). Admin-API-off (404) and insufficient-role (401/403) render
/// as first-class states.
fn config_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { fetch_admin_config().await });
    let body = view! {
        <Suspense fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-24" />
                </thaw::Skeleton>
            }
        }>
            {move || {
                Suspend::new(async move {
                    match resource.await {
                        Ok(config) => {
                            view! {
                                <pre class="max-h-96 overflow-auto rounded-card border border-edge bg-sunken p-3 font-mono text-xs leading-relaxed text-ink">
                                    {config}
                                </pre>
                            }
                                .into_any()
                        }
                        Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "The CDR's admin API is disabled — enable [admin] to expose the configuration view."
                                </p>
                            }
                                .into_any()
                        }
                        Err(ViewerError::Forbidden(_)) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "This session lacks the ADMIN role — the configuration view needs an admin sign-in."
                                </p>
                            }
                                .into_any()
                        }
                        Err(ViewerError::CdrUnauthorized(_)) => {
                            view! {
                                <p class="text-sm text-ink-muted">
                                    "The CDR no longer accepts this session — sign in again to read the configuration."
                                </p>
                            }
                                .into_any()
                        }
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })
            }}
        </Suspense>
    }
    .into_any();
    titled_card("Runtime configuration (redacted)", true, body)
}

/// The `<Suspense>` fallback shared by every data-backed card.
fn card_skeleton() -> impl IntoView {
    view! {
        <thaw::Skeleton>
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4" />
        </thaw::Skeleton>
    }
}

/// Status card: `fetch_status` JSON → a definition list. A transport or parse
/// failure resolves inside the `<Suspense>` (an SSR'd `ErrorBoundary` fallback
/// mismatches at hydration in leptos 0.8) as an explicit DOWN state.
fn status_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { crate::auth::fetch_status().await });
    let body = view! {
        <Suspense fallback=card_skeleton>
            {move || Suspend::new(async move {
                match resource.await.and_then(|body| status_body(&body)) {
                    Ok(view) => view,
                    Err(e) => {
                        // Resolve inside the Suspense: render the parsed
                        // status, or the explicit DOWN chip on transport/parse failure.
                        view! {
                            <div>
                                <span class="inline-flex items-center gap-1.5 rounded-full border border-edge bg-raised px-2.5 py-1 text-xs font-medium text-danger">
                                    <span class="h-1.5 w-1.5 rounded-full bg-danger"></span>
                                    "DOWN"
                                </span>
                                <p class="mt-2 text-sm text-danger">{e.to_string()}</p>
                            </div>
                        }
                            .into_any()
                    }
                }
            })}
        </Suspense>
    }
    .into_any();
    titled_card("Status", false, body)
}

/// The uniform card error state: the domain-error message in an error
/// `MessageBar` (the same shape the removed per-card `ErrorBoundary` fallbacks
/// rendered).
fn card_error(error: &ViewerError) -> AnyView {
    let message = error.to_string();
    view! {
        <thaw::MessageBar intent=thaw::MessageBarIntent::Error>
            <thaw::MessageBarBody>{message}</thaw::MessageBarBody>
        </thaw::MessageBar>
    }
    .into_any()
}

/// Render the parsed `{rest root}/status` document: an UP/DOWN pill plus every
/// scalar field as a definition list.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn status_body(body: &str) -> Result<AnyView, ViewerError> {
    let doc = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| ViewerError::Internal(format!("status JSON: {e}")))?;
    let up = doc
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|s| s.eq_ignore_ascii_case("UP"));
    let rows = scalar_rows(&doc)
        .into_iter()
        .map(|(k, v)| {
            view! {
                <dt class="font-medium text-ink-muted">{k}</dt>
                <dd class="font-mono break-all text-ink">{v}</dd>
            }
        })
        .collect::<Vec<_>>();
    let (dot, text_cls, label) = if up {
        ("bg-ok", "text-ink", "UP")
    } else {
        ("bg-danger", "text-danger", "DOWN")
    };
    Ok(view! {
        <div>
            <span class=format!(
                "inline-flex items-center gap-1.5 rounded-full border border-edge bg-raised px-2.5 py-1 text-xs font-medium {text_cls}",
            )>
                <span class=format!("h-1.5 w-1.5 rounded-full {dot}")></span>
                {label}
            </span>
            <dl class="mt-3 grid grid-cols-2 gap-x-4 gap-y-1 text-sm">{rows}</dl>
        </div>
    }
    .into_any())
}

/// SMART card: `fetch_smart_config` → a "disabled" state, or the advertised
/// endpoints (out-links) plus capability chips.
///
/// A failed probe renders [`smart_probe_failure_copy`] rather than the raw
/// error: "the CDR answered 302" tells a reader nothing they can act on.
fn smart_card() -> AnyView {
    let resource = Resource::new(|| (), |()| async move { fetch_smart_config().await });
    let body = view! {
        <Suspense fallback=card_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(config) => smart_body(config),
                    Err(e) => {
                        view! {
                            <thaw::MessageBar intent=thaw::MessageBarIntent::Warning>
                                <thaw::MessageBarBody>
                                    {smart_probe_failure_copy(&e)}
                                </thaw::MessageBarBody>
                            </thaw::MessageBar>
                        }
                            .into_any()
                    }
                }
            })}
        </Suspense>
    }
    .into_any();
    titled_card("SMART", false, body)
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
        ViewerError::CdrUnreachable(detail) => format!(
            "SMART discovery could not be read: the CDR is unreachable ({detail}). This says \
             nothing about whether SMART is enabled — check the CDR is up, then reload."
        ),
        ViewerError::CdrUnauthorized(_) | ViewerError::Unauthenticated => format!(
            "SMART discovery could not be read: the CDR refused this session at {endpoint}. Sign \
             in again, then reload."
        ),
        ViewerError::Forbidden(_) => format!(
            "SMART discovery could not be read: this session may not read {endpoint}. It needs an \
             account the CDR allows there."
        ),
        _ => {
            // A CDR body that only restates its own status ("HTTP 302") adds
            // nothing, so it is dropped rather than printed beside the code it
            // repeats.
            let detail = match error {
                ViewerError::Cdr { status, message } => {
                    let restatement = format!("HTTP {status}");
                    if message.trim().is_empty() || message.trim() == restatement {
                        format!("the CDR answered {status} there")
                    } else {
                        format!("the CDR answered {status} there: {message}")
                    }
                }
                other => other.to_string(),
            };
            format!(
                "SMART discovery could not be read from {endpoint} — {detail}. Whether SMART is \
                 enabled is therefore unknown; check that the CDR serves that path."
            )
        }
    }
}

/// Render the SMART discovery document. `None` is the neutral disabled state;
/// `Some(json)` renders the authentication endpoints as out-links and the
/// advertised capability lists as chips (master04-service_discovery.adoc
/// §"Authentication Endpoints").
fn smart_body(config: Option<String>) -> AnyView {
    let Some(raw) = config else {
        return view! {
            <thaw::MessageBar intent=thaw::MessageBarIntent::Info>
                <thaw::MessageBarBody>
                    "SMART is not enabled on this CDR: it serves no service-discovery document."
                </thaw::MessageBarBody>
            </thaw::MessageBar>
        }
        .into_any();
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return view! {
            <thaw::MessageBar intent=thaw::MessageBarIntent::Warning>
                <thaw::MessageBarBody>
                    "SMART configuration returned, but it is not valid JSON."
                </thaw::MessageBarBody>
            </thaw::MessageBar>
        }
        .into_any();
    };

    // Advertised authentication endpoints → out-links (present ones only).
    let endpoints = [
        ("authorization_endpoint", "Authorization"),
        ("token_endpoint", "Token"),
        ("jwks_uri", "JWKS"),
        ("introspection_endpoint", "Introspection"),
    ];
    let links = endpoints
        .iter()
        .filter_map(|(key, label)| {
            doc.get(*key)
                .and_then(serde_json::Value::as_str)
                .map(|url| {
                    let href = url.to_owned();
                    let label = *label;
                    view! {
                        <a
                            href=href
                            target="_blank"
                            rel="noreferrer"
                            class="text-sm text-accent hover:underline"
                        >
                            {label}
                            " ↗"
                        </a>
                    }
                })
        })
        .collect::<Vec<_>>();

    // Advertised capability lists → chips.
    let chip_groups = [
        ("grant_types_supported", "Grant types"),
        ("code_challenge_methods_supported", "PKCE methods"),
        ("scopes_supported", "Scopes"),
    ];
    let chips = chip_groups
        .iter()
        .filter_map(|(key, label)| {
            doc.get(*key)
                .and_then(serde_json::Value::as_array)
                .map(|arr| {
                    let tags = arr
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(|s| {
                            let s = s.to_owned();
                            view! { <span class="rounded-full bg-accent-subtle px-2 py-0.5 text-xs text-accent-ink">{s}</span> }
                        })
                        .collect::<Vec<_>>();
                    let label = *label;
                    view! {
                        <div class="mt-2">
                            <div class="text-xs font-medium text-ink-muted mb-1">{label}</div>
                            <div class="flex flex-wrap gap-1">{tags}</div>
                        </div>
                    }
                })
        })
        .collect::<Vec<_>>();

    view! {
        <div>
            <div class="flex flex-wrap gap-3">{links}</div>
            {chips}
        </div>
    }
    .into_any()
}

/// Served-OpenAPI card: the per-family document selector plus `fetch_openapi`
/// → our own grouped endpoint list (never a Swagger embed).
///
/// The selected family is URL state (`?openapi=`), so the resource's source is
/// the query memo and the selector is a plain GET form that works before WASM
/// loads. A `404` means this deployment does not serve that family document —
/// a first-class state, not an error.
fn openapi_card(family: Memo<String>) -> AnyView {
    let resource = Resource::new(
        move || family.get(),
        |family| async move { fetch_openapi(family).await },
    );
    let selector = openapi_selector(family);
    let document = view! {
        <Transition fallback=card_skeleton>
            {move || Suspend::new(async move {
                let rendered = resource
                    .await
                    .and_then(|doc_str| {
                        let doc = serde_json::from_str::<serde_json::Value>(&doc_str)
                            .map_err(|e| ViewerError::Internal(format!("openapi JSON: {e}")))?;
                        Ok::<_, ViewerError>(openapi_body(&doc))
                    });
                match rendered {
                    Ok(view) => view,
                    Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                        view! {
                            <p class="text-sm text-ink-muted">
                                "This CDR serves no document for that API family — pick another, or the complete surface."
                            </p>
                        }
                            .into_any()
                    }
                    Err(e) => card_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();
    let body =
        view! { <div>{selector} <div id="openapi-family-card">{document}</div></div> }.into_any();
    titled_card("Served OpenAPI", true, body)
}

/// The API-family selector: a GET form whose `openapi` field is the URL state
/// the card reads. Uncontrolled — the form owns the value and the `selected`
/// attribute server-renders the current choice (the audit filter's pattern; a
/// `prop:` would not render server-side).
fn openapi_selector(family: Memo<String>) -> AnyView {
    let options = OPENAPI_FAMILIES
        .iter()
        .map(|(slug, label)| {
            let slug = *slug;
            let selected = move || family.get() == slug;
            view! {
                <option value=slug selected=selected>
                    {*label}
                </option>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <leptos_router::components::Form method="GET" action="/system" attr:class="mb-3">
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex flex-col gap-1 text-xs text-ink-muted" r#for="openapi-family">
                    "API family"
                    <select id="openapi-family" name="openapi" class=SELECT>
                        {options}
                    </select>
                </label>
                <button id="openapi-family-show" type="submit" class=BTN_SECONDARY>
                    "Show"
                </button>
            </div>
        </leptos_router::components::Form>
    }
    .into_any()
}

/// Render the served `OpenAPI` document as a grouped, scrollable endpoint list.
fn openapi_body(doc: &serde_json::Value) -> AnyView {
    let groups = group_openapi_paths(doc);
    if groups.is_empty() {
        return view! {
            <EmptyState
                icon=icondata_lu::LuRoute
                message="No paths advertised"
                hint="The CDR served an OpenAPI document with no operations in it."
            />
        }
        .into_any();
    }
    let sections = groups
        .into_iter()
        .map(|(group, rows)| {
            let header = format!("{group} ({})", rows.len());
            let items = rows
                .into_iter()
                .map(|(method, path)| {
                    view! {
                        <li class="flex gap-2 py-0.5 text-sm">
                            <code class="font-mono font-semibold shrink-0 text-accent">
                                {method}
                            </code>
                            <span class="font-mono break-all text-ink">{path}</span>
                        </li>
                    }
                })
                .collect::<Vec<_>>();
            view! {
                <section class="mb-3">
                    <h3 class="text-xs font-semibold uppercase tracking-wide text-ink-muted mb-1">
                        {header}
                    </h3>
                    <ul>{items}</ul>
                </section>
            }
        })
        .collect::<Vec<_>>();
    view! { <div class="overflow-x-auto max-h-96 overflow-y-auto">{sections}</div> }.into_any()
}

/// Activity-log card: a compact pointer into the `/audit` screen, which is
/// the viewer's real activity browser (the CDR's local Audit Record
/// Repository over the RESTful-ATNA ITI-81 retrieval). This card deliberately
/// fetches nothing — duplicating a filterable, paged browser inside a panel
/// tile would be a second, worse audit surface.
fn activity_log_card() -> AnyView {
    let body = view! {
        <div class="flex flex-col items-start gap-3">
            <p class="text-sm text-ink-muted">
                "Who accessed what, with what outcome: the ATNA security audit trail, "
                "filterable by time window, patient, principal, outcome and action."
            </p>
            <A href="/audit" attr:class=BTN_SECONDARY>
                <leptos_icons::Icon icon=icondata_lu::LuActivity width="14" height="14" />
                " Open audit browser"
            </A>
        </div>
    }
    .into_any();
    titled_card("Audit log", false, body)
}

/// Every top-level scalar field of a JSON object as `(key, text)`, sorted by
/// key for a stable (hydration-safe) render order.
fn scalar_rows(doc: &serde_json::Value) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(obj) = doc.as_object() {
        for (key, value) in obj {
            let text = match value {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                serde_json::Value::Bool(b) => Some(b.to_string()),
                _ => None,
            };
            if let Some(text) = text {
                rows.push((key.clone(), text));
            }
        }
    }
    rows.sort();
    rows
}

/// Group an `OpenAPI` document's operations by their first `tag` (falling back
/// to the first path segment, then `"default"`), as sorted `(group, rows)`
/// where each row is `(METHOD, path)`. Sorted throughout so server and client
/// render the identical structure (no non-deterministic order).
fn group_openapi_paths(doc: &serde_json::Value) -> Vec<(String, Vec<(String, String)>)> {
    const METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "patch", "options", "head", "trace",
    ];
    let mut groups: std::collections::BTreeMap<String, Vec<(String, String)>> =
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
            groups
                .entry(group)
                .or_default()
                .push((method.to_uppercase(), path.clone()));
        }
    }
    let mut out: Vec<(String, Vec<(String, String)>)> = groups.into_iter().collect();
    for (_, rows) in &mut out {
        rows.sort();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        OPENAPI_FAMILIES, SMART_DISCOVERY_PATH, group_openapi_paths, openapi_family_slug,
        scalar_rows, smart_probe_failure_copy,
    };
    use crate::error::ViewerError;

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

    #[cfg(feature = "ssr")]
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

    #[cfg(feature = "ssr")]
    #[test]
    fn the_repository_usage_statement_is_valid_aql() {
        // The fixed text is parsed by the real grammar, so a typo in the
        // constant fails here rather than as a CDR 400 on the card.
        openehr_query::parser::parse_str(super::TEMPLATE_USAGE_AQL).expect("the count AQL parses");
    }

    #[test]
    fn the_family_selector_offers_the_complete_document_first_and_unique_slugs() {
        assert_eq!(
            OPENAPI_FAMILIES.first().map(|(slug, _)| *slug),
            Some(""),
            "the complete surface is the default (empty) slug"
        );
        let mut slugs: Vec<&str> = OPENAPI_FAMILIES.iter().map(|(slug, _)| *slug).collect();
        let count = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), count, "family slugs must be unique");
    }

    #[test]
    fn an_unknown_family_slug_degrades_to_the_complete_document() {
        assert_eq!(openapi_family_slug("ehr"), "ehr");
        assert_eq!(openapi_family_slug("query"), "query");
        // Hand-typed junk in `?openapi=` is user input, never a path segment.
        assert_eq!(openapi_family_slug("../../etc/passwd"), "");
        assert_eq!(openapi_family_slug("EHR"), "");
        assert_eq!(openapi_family_slug(""), "");
    }

    #[test]
    fn groups_by_tag_then_path_segment_sorted() {
        let doc = serde_json::json!({
            "paths": {
                "/ehr/{ehr_id}": {
                    "put": {"tags": ["EHR"]},
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
                ("GET".to_owned(), "/ehr/{ehr_id}".to_owned()),
                ("PUT".to_owned(), "/ehr/{ehr_id}".to_owned()),
            ]
        );
        // A path item with only non-operation keys contributes nothing.
        assert!(
            groups
                .iter()
                .all(|(_, rows)| rows.iter().all(|(_, p)| p != "/misc/thing"))
        );
    }

    #[test]
    fn empty_document_yields_no_groups() {
        assert!(group_openapi_paths(&serde_json::json!({})).is_empty());
        assert!(group_openapi_paths(&serde_json::json!({"paths": {}})).is_empty());
    }

    #[test]
    fn scalar_rows_are_scalar_only_and_key_sorted() {
        let doc = serde_json::json!({
            "status": "UP",
            "server_version": "3.1.1",
            "count": 5,
            "flag": true,
            "nested": {"x": 1},
            "list": [1, 2]
        });
        let rows = scalar_rows(&doc);
        assert_eq!(
            rows,
            vec![
                ("count".to_owned(), "5".to_owned()),
                ("flag".to_owned(), "true".to_owned()),
                ("server_version".to_owned(), "3.1.1".to_owned()),
                ("status".to_owned(), "UP".to_owned()),
            ]
        );
    }
}
