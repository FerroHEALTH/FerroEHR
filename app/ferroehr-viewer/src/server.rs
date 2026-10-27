// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's axum service: the router assembly and the pre-render session
//! guard.
//!
//! The family's console headers and same-origin check
//! (`ferrohealth_server::console`, `ferrohealth_server::same_origin`) wrap
//! the routes, and the family stack wraps them with its health routes.
//!
//! Lives in the lib (ssr-gated) rather than the binary so the integration
//! tests drive the REAL request path — guard, session layer, Leptos render —
//! with `tower::ServiceExt::oneshot`, the same way `app/ferroehr-server`
//! keeps a testable lib run path.

use axum::Extension;
use ferrohealth_server::console::{self, ConsoleHeaders, CspNonce};
use ferrohealth_server::health::Health;
use ferrohealth_server::same_origin::{self, SameOrigin};
use ferrohealth_server::stack::{self, StackPolicy};
use leptos::prelude::LeptosOptions;
use leptos_axum::LeptosRoutes;

use crate::session::{SESSION_KEY, ViewerSession};
use crate::state::AppState;

/// The viewer as it serves: [`router`] and the health routes inside the
/// family stack (`ferrohealth_server::stack::app`), which applies the
/// request timeout, the body limit and the overload bound of `[server]`.
pub fn app(app_state: AppState, leptos_options: LeptosOptions, health: Health) -> axum::Router {
    let section = app_state.config.server.clone();
    stack::app(
        router(app_state, leptos_options),
        health,
        &section,
        StackPolicy::default(),
    )
}

/// Assembles the viewer's own routes.
///
/// The OIDC routes, the export post, the preference, sidebar and sign-out
/// posts, FerroUI's stylesheet and the FerroEHR theme, the Leptos routes with
/// the per-request context, each route's form post drawn by the same page
/// (`post_page`), the static-file fallback, and the layer stack (state →
/// session guard → same-origin check → console headers). Sessions are a
/// sealed cookie ([`crate::session`]) — no session-store layer exists, so no
/// state is bound to any one instance.
pub fn router(app_state: AppState, leptos_options: LeptosOptions) -> axum::Router {
    let routes = leptos_axum::generate_route_list(crate::app::App);
    let origin = same_origin_check(&app_state);

    let context_state = app_state.clone();
    // One context closure for both render entry points (the routed app and the
    // 404 shell): the app state plus this request's CSP nonce.
    let provide_viewer_context = move || {
        leptos::context::provide_context(context_state.clone());
        provide_request_nonce();
    };
    let pkg_route = format!("/{}", leptos_options.site_pkg_dir);
    let headers = ConsoleHeaders::new("'self'").with_bundle(pkg_route.clone());
    let pkg_dir = format!(
        "{}/{}",
        leptos_options.site_root, leptos_options.site_pkg_dir
    );
    let mut service = axum::Router::new()
        // The hydration bundle, served on its own: a file the bundle lacks
        // answers 404 and a read failure 500, never the rendered app or the
        // sign-in guard's redirect, and either is logged with its status.
        .nest_service(&pkg_route, pkg_service(&pkg_dir))
        .route("/auth/oidc/login", axum::routing::get(crate::oidc::login))
        .route(
            "/auth/oidc/callback",
            axum::routing::get(crate::oidc::callback),
        )
        // Result export: a plain form-POST download (no WASM required); the
        // handler enforces the viewer session itself like every server fn.
        .route(
            "/export/aql/{format}",
            axum::routing::post(crate::export::export_aql),
        )
        // FerroUI's stylesheet and its FerroEHR theme, served from this origin
        // so `style-src 'self'` holds.
        .route(
            crate::app::STYLESHEET_PATH,
            axum::routing::get(|| async { css(ferrohealth_ui::style::STYLESHEET) }),
        )
        .route(
            crate::app::THEME_PATH,
            axum::routing::get(|| async { css(ferrohealth_ui::style::Brand::Ehr.theme()) }),
        )
        .route(
            crate::prefs::PREFERENCE_ROUTE,
            axum::routing::post(crate::prefs::choose_preference),
        )
        .route(
            crate::prefs::RESET_ROUTE,
            axum::routing::post(crate::prefs::reset_preferences),
        )
        .route(
            crate::prefs::SIDEBAR_ROUTE,
            axum::routing::post(crate::prefs::choose_sidebar),
        )
        .route(crate::shell::SIGN_OUT_ROUTE, axum::routing::post(sign_out))
        .route(
            crate::paging::PAGING_ROUTE,
            axum::routing::post(crate::paging::choose_paging),
        )
        // A template's Download: the stored source as a file, read with the
        // session's credential.
        .route(
            "/download/template/{family}/{id}",
            axum::routing::get(crate::download::download_template),
        )
        .route(
            crate::pages::about::DOWNLOAD_ROUTE,
            axum::routing::get(download_manifest),
        )
        // The FerroEHR mark as the tab icon and in the lockup: the SVGs the
        // pages link, and the raster set at the path a browser requests
        // unasked.
        .route(
            crate::app::ICON_PATH,
            axum::routing::get(|| async { icon("image/svg+xml", FAVICON_SVG) }),
        )
        .route(
            crate::shell::MARK_PATH,
            axum::routing::get(|| async { icon("image/svg+xml", MARK_SVG) }),
        )
        .route(
            FAVICON_ICO_PATH,
            axum::routing::get(|| async { icon("image/x-icon", FAVICON_ICO) }),
        );
    // Every page answers its own form posts: the same page drawn again with
    // the posted fields in context (`crate::posted`).
    for listing in &routes {
        service = service.route(listing.path(), axum::routing::post(post_page));
    }
    let service = service
        .leptos_routes_with_context(&leptos_options, routes, provide_viewer_context.clone(), {
            let options = leptos_options.clone();
            move || crate::app::document(options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler_with_context(
            provide_viewer_context,
            crate::app::document,
        ))
        // Ahead of every render: a document request without a session never
        // reaches Leptos, and an authenticated response leaves with a
        // refreshed sliding-expiry cookie.
        .layer(axum::middleware::from_fn(login_guard))
        .layer(Extension(app_state))
        .layer(axum::middleware::from_fn(move |request, next| {
            same_origin::only(origin.clone(), request, next)
        }))
        .layer(axum::middleware::from_fn(move |request, next| {
            console::headers(headers.clone(), request, next)
        }));
    service.with_state(leptos_options)
}

/// The same-origin check of the viewer's unsafe requests: `Sec-Fetch-Site`
/// decides, and without it `Origin` or `Referer` must name the public URL
/// `auth.oidc.public_base_url` gives, when one is set.
///
/// A public URL that does not parse was refused when the configuration was
/// read; one built by hand that does not is logged and left out, so only
/// `Sec-Fetch-Site` passes.
fn same_origin_check(state: &AppState) -> SameOrigin {
    let public = state.config.auth.oidc.public_base_url.as_str();
    if public.is_empty() {
        return SameOrigin::new();
    }
    let checked = crate::config::public_origin(public)
        .map_err(|error| error.to_string())
        .and_then(|url| {
            SameOrigin::new()
                .with_origin(&url)
                .map_err(|error| error.to_string())
        });
    checked.unwrap_or_else(|error| {
        tracing::error!(%error, "the same-origin check knows no public origin");
        SameOrigin::new()
    })
}

/// Draws the page a form posted to, with the posted fields in context.
///
/// The fields are read as `application/x-www-form-urlencoded`, at most
/// `server.body_limit_bytes`; the page reads them through
/// [`crate::posted::posted`] and answers with a redirect when its work
/// succeeded or with itself, the fields as posted and the refusal beside
/// them, when it did not. A post without a session (but to the sign-in page)
/// goes to the sign-in page, which returns to the page posted to.
async fn post_page(
    axum::extract::State(options): axum::extract::State<LeptosOptions>,
    Extension(state): Extension<AppState>,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let signed_in = crate::session::unseal(
        &state.session_keys,
        &parts.headers,
        state.config.session.idle_minutes,
    )
    .get::<ViewerSession>(SESSION_KEY)
    .is_some();
    if !signed_in && path != "/login" {
        let to = crate::session_client::signed_out_url(crate::request::return_path(&path), "");
        return axum::response::Redirect::to(&to).into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(body, state.config.server.body_limit_bytes).await else {
        return (http::StatusCode::PAYLOAD_TOO_LARGE, "the form is too large").into_response();
    };
    let form = axum::extract::Request::from_parts(parts.clone(), axum::body::Body::from(bytes));
    let fields = match <axum::extract::Form<Vec<(String, String)>> as axum::extract::FromRequest<
        (),
    >>::from_request(form, &())
    .await
    {
        Ok(axum::extract::Form(fields)) => fields,
        Err(rejection) => return rejection.into_response(),
    };
    let posted = crate::posted::Posted::new(fields);
    let request = axum::extract::Request::from_parts(parts, axum::body::Body::empty());
    let handler = leptos_axum::render_app_async_with_context(
        move || {
            leptos::context::provide_context(state.clone());
            leptos::context::provide_context(posted.clone());
            provide_request_nonce();
        },
        move || crate::app::document(options.clone()),
    );
    handler(request).await
}

/// Answers About's Download: the CDR's conformance manifest as it sent it,
/// as a file. The guard has already refused a request without a session.
async fn download_manifest(Extension(state): Extension<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    match crate::system_api::fetch_conformance_document(&state).await {
        Ok(body) => (
            [
                (
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                ),
                (
                    http::header::CONTENT_DISPOSITION,
                    http::HeaderValue::from_static("attachment; filename=\"options.json\""),
                ),
            ],
            body,
        )
            .into_response(),
        Err(error) => (http::StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    }
}

/// Signs the reader out: the session cookie is cleared and the browser goes
/// to the sign-in page, which says the reader signed out.
async fn sign_out(Extension(state): Extension<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let mut response = axum::response::Redirect::to("/login?signed-out=1").into_response();
    match crate::session::set_cookie(
        &state.session_keys,
        &crate::session::CookieSession::default(),
        state.config.session.cookie_secure,
        state.config.session.idle_minutes,
    ) {
        Ok(cookie) => {
            response
                .headers_mut()
                .append(http::header::SET_COOKIE, cookie);
            response
        }
        Err(_failed) => http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The service of the hydration bundle under `dir`: its files as they stand,
/// or their brotli and gzip variants when present and asked for, with a warn
/// for any answer that is not a served body.
///
/// The bundle never reaches the Leptos static fallback, which renders the
/// whole app in place of any answer other than `200`
/// (`leptos_axum::file_and_error_handler`), so a file it cannot serve answers
/// its own status and never the shell's session redirect.
fn pkg_service(dir: &str) -> axum::Router {
    let files = tower_http::services::ServeDir::new(dir)
        .precompressed_br()
        .precompressed_gzip();
    axum::Router::new()
        .fallback_service(files)
        .layer(axum::middleware::from_fn(warn_unserved_asset))
}

/// Logs a bundle request that did not answer with a body (`2xx`) or a
/// revalidation (`304`), naming the path and the status it got.
async fn warn_unserved_asset(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map_or_else(
            || request.uri().path().to_owned(),
            |uri| uri.path().to_owned(),
        );
    let response = next.run(request).await;
    let status = response.status();
    if !status.is_success() && status != http::StatusCode::NOT_MODIFIED {
        tracing::warn!(%path, %status, "a hydration bundle request was not served");
    }
    response
}

/// The path a browser requests the tab icon from when a page links none.
const FAVICON_ICO_PATH: &str = "/favicon.ico";

/// The FerroEHR favicon master, the tile with `Fe` (`assets/brand/README.md`).
const FAVICON_SVG: &[u8] = include_bytes!("../../../assets/brand/favicon.svg");

/// The FerroEHR icon the lockup draws (`assets/brand/README.md`).
const MARK_SVG: &[u8] = include_bytes!("../../../assets/brand/ferroehr-icon.svg");

/// The raster favicon set (48, 32 and 16 px) rendered from [`FAVICON_SVG`].
const FAVICON_ICO: &[u8] = include_bytes!("../../../assets/brand/favicon.ico");

/// An icon response of the given media type.
fn icon(content_type: &'static str, bytes: &'static [u8]) -> axum::response::Response {
    axum::response::IntoResponse::into_response((
        [(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(content_type),
        )],
        bytes,
    ))
}

/// A stylesheet response.
fn css(text: &'static str) -> axum::response::Response {
    axum::response::IntoResponse::into_response((
        [(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        text,
    ))
}

/// The pre-render session guard.
///
/// A document request (`GET`/`HEAD`) for a guarded path without a live
/// session goes to the sign-in page, which returns the reader to the page
/// they asked for after signing in, and says the session ended when the
/// request carried a session cookie that no longer opens. An authenticated
/// response leaves with the sliding idle anchor moved, unless the handler
/// wrote the session itself.
pub async fn login_guard(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let state = request.extensions().get::<AppState>().cloned();
    let session = state.as_ref().map(|s| {
        crate::session::unseal(
            &s.session_keys,
            request.headers(),
            s.config.session.idle_minutes,
        )
    });
    let authenticated = session
        .as_ref()
        .is_some_and(|s| s.get::<ViewerSession>(SESSION_KEY).is_some());

    let method = request.method();
    let guarded = (method == http::Method::GET || method == http::Method::HEAD)
        && !is_public_path(request.uri().path());
    if guarded && !authenticated {
        let had_cookie = carries_session_cookie(request.headers());
        let path_and_query = request
            .uri()
            .path_and_query()
            .map_or_else(|| "/".to_owned(), ToString::to_string);
        let location = sign_in_location(&path_and_query, had_cookie);
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::FOUND;
        match http::HeaderValue::from_str(&location) {
            Ok(value) => {
                response.headers_mut().insert(http::header::LOCATION, value);
            }
            Err(_invalid) => {
                response.headers_mut().insert(
                    http::header::LOCATION,
                    http::HeaderValue::from_static("/login"),
                );
            }
        }
        return response;
    }

    let mut response = next.run(request).await;
    if authenticated
        && !sets_session_cookie(response.headers())
        && let (Some(state), Some(session)) = (state, session)
        && let Ok(refreshed) = crate::session::set_cookie(
            &state.session_keys,
            &session,
            state.config.session.cookie_secure,
            state.config.session.idle_minutes,
        )
    {
        response
            .headers_mut()
            .append(http::header::SET_COOKIE, refreshed);
    }
    response
}

/// Whether a request carries the viewer's session cookie at all.
fn carries_session_cookie(headers: &http::HeaderMap) -> bool {
    headers
        .get_all(http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|cookies| {
            ferrohealth_ui::theme::cookie(cookies, crate::session::SESSION_COOKIE).is_some()
        })
}

/// The sign-in page a guarded request without a session goes to: it keeps
/// the page asked for as `next`, and says the session ended when the request
/// carried a session cookie.
#[must_use]
pub fn sign_in_location(path_and_query: &str, had_cookie: bool) -> String {
    let next = crate::auth::same_origin_next(Some(path_and_query));
    let expired = if had_cookie { "expired=1&" } else { "" };
    if next == "/" && !had_cookie {
        "/login".to_owned()
    } else {
        format!("/login?{expired}next={}", urlencoding::encode(&next))
    }
}

/// Whether a response already sets our session cookie (the handler wrote the
/// session itself — its value wins over the guard's sliding refresh).
fn sets_session_cookie(headers: &http::HeaderMap) -> bool {
    headers.get_all(http::header::SET_COOKIE).iter().any(|v| {
        v.to_str()
            .is_ok_and(|s| s.starts_with(crate::session::SESSION_COOKIE))
    })
}

/// Whether a path is served without a viewer session: the sign-in screen,
/// the OIDC handshake, the hydration assets, the favicon, and the server-fn
/// endpoints (each of which enforces the session itself with a typed
/// refusal, which its WASM caller expects instead of a redirect).
fn is_public_path(path: &str) -> bool {
    path == "/login"
        || path == FAVICON_ICO_PATH
        || path == crate::app::ICON_PATH
        || path == crate::app::STYLESHEET_PATH
        || path == crate::app::THEME_PATH
        || path == crate::shell::MARK_PATH
        || path.starts_with("/auth/")
        || path.starts_with("/pkg/")
        || path.starts_with("/api/")
}

/// Re-provides this request's nonce into the Leptos render context.
///
/// `leptos_axum` provides a nonce of its own before the per-request context
/// closure runs, and that one is not the value the response header authorizes.
/// Overriding it with the one `ferrohealth_server::console` minted is what
/// makes the two the same string; without it the bootstrap script carries a
/// nonce the browser has never been told to trust, and the viewer renders but
/// never hydrates.
fn provide_request_nonce() {
    let Some(parts) = leptos::context::use_context::<http::request::Parts>() else {
        return;
    };
    if let Some(nonce) = parts.extensions.get::<CspNonce>() {
        leptos::context::provide_context(leptos::nonce::Nonce::from_value(nonce.as_str()));
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        reason = "the exchange helpers panic to report a broken fixture, which is how a test fails (Book ch11); clippy's allow-*-in-tests scoping covers only the #[test] fns themselves"
    )]

    use ferrohealth_server::console::{self, ConsoleHeaders, CspNonce};
    use tower::util::ServiceExt;

    use super::{is_public_path, provide_request_nonce};

    /// The response header the stub handler reports the render-context nonce
    /// through, so a test can compare it with the policy.
    const RENDERED_NONCE: &str = "x-test-rendered-nonce";

    /// The console headers as the viewer mounts them.
    fn viewer_headers() -> ConsoleHeaders {
        ConsoleHeaders::new("'self'").with_bundle("/pkg")
    }

    /// The sign-in surface, the OIDC handshake, the hydration assets and the
    /// server-fn endpoints stay reachable without a session; every screen
    /// path is guarded.
    #[test]
    fn the_public_surface_is_exactly_the_sessionless_one() {
        for public in [
            "/login",
            "/favicon.ico",
            "/favicon.svg",
            "/auth/oidc/login",
            "/auth/oidc/callback",
            "/pkg/ferroehr-viewer.css",
            "/api/current_session",
        ] {
            assert!(is_public_path(public), "{public} must be public");
        }
        for guarded in ["/", "/templates", "/queries/builder", "/ehrs", "/nonsense"] {
            assert!(!is_public_path(guarded), "{guarded} must be guarded");
        }
    }

    /// Returns the named directive of `policy`, panicking if it is absent.
    fn directive<'a>(policy: &'a str, name: &str) -> &'a str {
        policy
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with(name))
            .unwrap_or_else(|| panic!("the policy must carry a {name} directive"))
    }

    /// Stands in for the Leptos render: it takes the request's `Parts` into a
    /// reactive owner the way `leptos_axum` does, runs the viewer's context
    /// step, and reports whatever `use_nonce` then answers.
    async fn render_nonce(request: axum::extract::Request) -> axum::response::Response {
        let (parts, _body) = request.into_parts();
        let owner = leptos::prelude::Owner::new();
        let rendered = owner.with(|| {
            leptos::context::provide_context(parts);
            provide_request_nonce();
            leptos::nonce::use_nonce().map(|nonce| nonce.to_string())
        });
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        if let Some(rendered) = rendered {
            response.headers_mut().insert(
                http::HeaderName::from_static(RENDERED_NONCE),
                http::HeaderValue::from_str(&rendered).unwrap(),
            );
        }
        response
    }

    /// One response of a stub router at `uri` under the viewer's console
    /// headers.
    async fn exchange(uri: &str) -> axum::response::Response {
        let headers = viewer_headers();
        axum::Router::new()
            .route("/", axum::routing::get(render_nonce))
            .route(
                "/pkg/ferroehr-viewer.RUlUrl0ZR7DGPjuVWkKtwA.wasm",
                axum::routing::get(|| async { "wasm" }),
            )
            .layer(axum::middleware::from_fn(move |request, next| {
                console::headers(headers.clone(), request, next)
            }))
            .oneshot(
                http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// The named response header as text.
    fn header(response: &axum::response::Response, name: &str) -> String {
        response
            .headers()
            .get(name)
            .unwrap_or_else(|| panic!("the response must carry {name}"))
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Drives one request through the console headers, returning the policy
    /// the response carries and the nonce the render context saw.
    async fn one_exchange() -> (String, String) {
        let response = exchange("/").await;
        (
            header(&response, "content-security-policy"),
            header(&response, RENDERED_NONCE),
        )
    }

    /// The whole point of the exercise: the value the renderer stamps on the
    /// hydration bootstrap is the value the response header authorizes. A
    /// mismatch is invisible server-side and leaves the viewer dead in the
    /// browser.
    #[tokio::test]
    async fn the_rendered_nonce_is_the_one_the_header_authorizes() {
        let (policy, rendered) = one_exchange().await;
        assert!(
            directive(&policy, "script-src").contains(&format!("'nonce-{rendered}'")),
            "script-src must authorize the rendered nonce {rendered}: {policy}"
        );
    }

    /// Renders the component that actually stamps the attribute, and pins that
    /// the `nonce=` it emits is the value the header authorizes. The context
    /// test above covers our half of the handoff; this covers Leptos's half, so
    /// an upgrade that stopped stamping the bootstrap script fails here instead
    /// of in a browser.
    #[test]
    fn the_bootstrap_script_carries_the_authorized_nonce() {
        let nonce = CspNonce::mint();
        let policy = viewer_headers().policy(&nonce);
        let owner = leptos::prelude::Owner::new();
        let html = owner.with(|| {
            leptos::context::provide_context(leptos::nonce::Nonce::from_value(nonce.as_str()));
            let options = leptos::config::LeptosOptions::builder()
                .output_name("ferroehr-viewer")
                .build();
            leptos::prelude::RenderHtml::to_html(
                leptos::view! { <leptos::hydration::HydrationScripts options /> },
            )
        });
        let stamped = html
            .split_once("<script")
            .and_then(|(_, rest)| rest.split_once("nonce=\""))
            .and_then(|(_, rest)| rest.split_once('"'));
        let Some((stamped, _)) = stamped else {
            panic!("the bootstrap script must carry a nonce: {html}")
        };
        assert!(
            directive(&policy, "script-src").contains(&format!("'nonce-{stamped}'")),
            "the stamped nonce {stamped} is not the one script-src authorizes: {policy}"
        );
    }

    /// A nonce reused across responses is guessable, and then worth no more
    /// than the `'unsafe-inline'` it replaced.
    #[tokio::test]
    async fn every_response_carries_a_fresh_nonce() {
        let (first_policy, first) = one_exchange().await;
        let (second_policy, second) = one_exchange().await;
        assert_ne!(first, second, "two responses reused one nonce");
        assert_ne!(first_policy, second_policy);
    }

    /// The directives the policy the viewer serves must carry, each for a
    /// stated reason, and the ones it must never carry.
    #[tokio::test]
    async fn the_policy_carries_the_audited_directives() {
        let (policy, rendered) = one_exchange().await;
        for expected in [
            "default-src 'self'",
            // WebAssembly compilation, without permitting eval of JavaScript.
            "'wasm-unsafe-eval'",
            // The viewer reaches the CDR through its own server functions, so
            // the browser never needs a cross-origin connection.
            "connect-src 'self'",
            "object-src 'none'",
            "base-uri 'self'",
            "form-action 'self'",
            "frame-ancestors 'none'",
        ] {
            assert!(
                policy.contains(expected),
                "the policy must carry {expected}"
            );
        }
        assert!(!policy.contains("'unsafe-eval'"), "{policy}");
        let script_src = directive(&policy, "script-src");
        assert!(script_src.contains(&format!("'nonce-{rendered}'")));
        assert!(
            !script_src.contains("'unsafe-inline'"),
            "the nonce replaces the inline allowance: {script_src}"
        );
        // Every style is a stylesheet of the viewer's own origin.
        assert_eq!(directive(&policy, "style-src"), "style-src 'self'");
    }

    /// Every document keeps `no-store`, since the viewer renders patient data
    /// into HTML and stamps a per-request nonce on it; the hashed bundle is
    /// revalidated (`no-cache`), and a refused bundle answer is never kept.
    #[tokio::test]
    async fn only_the_bundle_is_kept_and_then_revalidated() {
        assert_eq!("no-store", header(&exchange("/").await, "cache-control"));
        assert_eq!(
            "no-cache",
            header(
                &exchange("/pkg/ferroehr-viewer.RUlUrl0ZR7DGPjuVWkKtwA.wasm").await,
                "cache-control"
            )
        );
        assert_eq!(
            "no-store",
            header(
                &exchange("/pkg/ferroehr-viewer.NoSuchHash0000000000.wasm").await,
                "cache-control"
            ),
            "a missing bundle file must not be kept"
        );
    }
}
