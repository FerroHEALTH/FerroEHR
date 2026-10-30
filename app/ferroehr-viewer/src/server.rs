// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's axum service: the router assembly, the page handler and the
//! session guard.
//!
//! Every page is a handler that draws one complete document
//! ([`page`]): it reads the request, the form a post carries and the CDR,
//! and answers with the page or with a redirect. FerroUI's stylesheet, the
//! FerroEHR theme and FerroUI's script modules are served by the library's
//! own asset router at `ferrohealth_ui::serve::PREFIX`. The family's console
//! headers and same-origin check (`ferrohealth_server::console`,
//! `ferrohealth_server::same_origin`) wrap the routes, the policy is the
//! server-rendered console's ([`POLICY`]), and the family stack wraps them
//! with its health routes.
//!
//! Lives in the lib rather than the binary so the integration tests drive the
//! REAL request path (guard, session, page) with `tower::ServiceExt::oneshot`,
//! the same way `app/ferroehr-server` keeps a testable lib run path.

use std::sync::Arc;

use axum::Extension;
use axum::response::IntoResponse as _;
use ferrohealth_server::console::{self, ConsoleHeaders};
use ferrohealth_server::health::Health;
use ferrohealth_server::same_origin::{self, SameOrigin};
use ferrohealth_server::stack::{self, StackPolicy};
use ferrohealth_telemetry::request_log::RequestLog;

use crate::scope::Scope;
use crate::session::{SESSION_KEY, ViewerSession};
use crate::state::AppState;

/// The Content-Security-Policy of every response.
///
/// Scripts and styles from the viewer's own origin alone, with no nonce, no
/// inline script or style and no `eval` (W3C Content Security Policy Level
/// 3; FerroUI `docs/consoles.md`, "How a console builds", step 6).
pub const POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self'; \
     img-src 'self' data:; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'self'; \
     form-action 'self'; \
     frame-ancestors 'none'";

/// The viewer as it serves: [`router`] and the health routes inside the
/// family stack (`ferrohealth_server::stack::app`).
///
/// The stack applies the request timeout, the body limit and the overload
/// bound of `[server]`, and writes `request_log`
/// ([`crate::telemetry::request_log`]).
pub fn app(app_state: AppState, health: Health, request_log: RequestLog) -> axum::Router {
    let section = app_state.config.server.clone();
    stack::app(
        router(app_state),
        health,
        &section,
        StackPolicy {
            request_log,
            ..StackPolicy::default()
        },
    )
}

/// Assembles the viewer's own routes.
///
/// FerroUI's assets, the OIDC routes, the export post, the preference,
/// sidebar, paging and sign-out posts, the downloads, the icons, the search
/// panel's answer route, every page ([`page`], for `GET` and its own form
/// posts), and the layer stack (state → session guard → same-origin check →
/// console headers → policy). Sessions are a sealed cookie
/// ([`crate::session`]): no session-store layer exists, so no state is bound
/// to any one instance.
pub fn router(app_state: AppState) -> axum::Router {
    let origin = same_origin_check(&app_state);
    let headers = ConsoleHeaders::new("'self'").with_bundle(ferrohealth_ui::serve::PREFIX);
    axum::Router::new()
        .nest(
            ferrohealth_ui::serve::PREFIX,
            ferrohealth_ui::serve::axum_router::router(),
        )
        .route("/auth/oidc/login", axum::routing::get(crate::oidc::login))
        .route(
            "/auth/oidc/callback",
            axum::routing::get(crate::oidc::callback),
        )
        // Result export: a plain form-POST download; the handler enforces the
        // viewer session itself like every page.
        .route(
            "/export/aql/{format}",
            axum::routing::post(crate::export::export_aql),
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
        // The search panel's answer, which the `search` module asks as the
        // words are typed (FerroUI `docs/consoles.md`, "The top bar's search").
        .route(
            crate::shell::FIND_ANSWER_ROUTE,
            axum::routing::post(find_answer),
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
        )
        .fallback(page)
        // Ahead of every page: a document request without a session never
        // reaches a handler, and an authenticated response leaves with a
        // refreshed sliding-expiry cookie.
        .layer(axum::middleware::from_fn(login_guard))
        .layer(Extension(app_state))
        .layer(axum::middleware::from_fn(move |request, next| {
            same_origin::only(origin.clone(), request, next)
        }))
        .layer(axum::middleware::from_fn(move |request, next| {
            console::headers(headers.clone(), request, next)
        }))
        .layer(axum::middleware::from_fn(policy))
}

/// Sets [`POLICY`] on every response, over the hydrated console's policy the
/// family's console headers write.
// TODO(#POLICYISSUE): take the policy from ferrohealth-server once it writes one for a server-rendered console.
async fn policy(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        http::header::CONTENT_SECURITY_POLICY,
        http::HeaderValue::from_static(POLICY),
    );
    response
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

/// The form a post to a page carries, read as
/// `application/x-www-form-urlencoded` up to `server.body_limit_bytes`.
async fn form_of(
    state: &AppState,
    parts: &http::request::Parts,
    body: axum::body::Body,
) -> Result<crate::posted::Posted, Box<axum::response::Response>> {
    let Ok(bytes) = axum::body::to_bytes(body, state.config.server.body_limit_bytes).await else {
        return Err(Box::new(
            (http::StatusCode::PAYLOAD_TOO_LARGE, "the form is too large").into_response(),
        ));
    };
    let form = axum::extract::Request::from_parts(parts.clone(), axum::body::Body::from(bytes));
    match <axum::extract::Form<Vec<(String, String)>> as axum::extract::FromRequest<()>>::from_request(
        form,
        &(),
    )
    .await
    {
        Ok(axum::extract::Form(fields)) => Ok(crate::posted::Posted::new(fields)),
        Err(rejection) => Err(Box::new(rejection.into_response())),
    }
}

/// Draws the page the request names: a `GET` reads it, and a `POST` draws
/// it with the form posted to it in reach ([`crate::posted::posted`]).
///
/// The page answers with a redirect when its work succeeded, or with itself,
/// the fields as posted and the refusal beside them, when it did not. A post
/// without a session (but to the sign-in page) goes to the sign-in page,
/// which returns to the page posted to.
async fn page(
    Extension(state): Extension<AppState>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let posted = if parts.method == http::Method::POST {
        let path = parts.uri.path().to_owned();
        let signed_in = crate::session::unseal(
            &state.session_keys,
            &parts.headers,
            state.config.session.idle_minutes,
        )
        .get::<ViewerSession>(SESSION_KEY)
        .is_some();
        if !signed_in && path != "/login" {
            let to =
                crate::session_client::signed_out_url(crate::request::return_path(&path), "");
            return axum::response::Redirect::to(&to).into_response();
        }
        match form_of(&state, &parts, body).await {
            Ok(posted) => Some(posted),
            Err(refusal) => return *refusal,
        }
    } else if parts.method == http::Method::GET || parts.method == http::Method::HEAD {
        None
    } else {
        return http::StatusCode::METHOD_NOT_ALLOWED.into_response();
    };
    let scope = Arc::new(Scope::new(state, parts, posted));
    let drawn = crate::scope::run(Arc::clone(&scope), async {
        let drawn = crate::routes::draw().await;
        if scope.answer(|answer| answer.redirect.is_some() || answer.session_over) {
            return None;
        }
        let body = if drawn.bare {
            crate::shell::centred_page(drawn.body)
        } else {
            crate::shell::framed(drawn.body).await
        };
        Some(crate::app::page_document(drawn.title, body))
    })
    .await;
    let mut answer = scope.take_answer();
    if answer.session_over {
        let path = scope.parts.uri.path();
        let search = scope.parts.uri.query().unwrap_or_default();
        answer.redirect = Some(crate::session_client::signed_out_url(path, search));
    }
    answered(answer, drawn)
}

/// The response of a page whose draw left `answer`: its redirect, else the
/// document `drawn` with its status, either with the cookies it set.
fn answered(
    answer: crate::scope::Answer,
    drawn: Option<ferrohealth_ui::html::Html>,
) -> axum::response::Response {
    let mut response = match (answer.redirect, drawn) {
        (Some(to), _) => axum::response::Redirect::to(&to).into_response(),
        (None, Some(document)) => {
            let mut response = axum::response::Html(String::from(document)).into_response();
            if let Some(status) = answer.status {
                *response.status_mut() = status;
            }
            response
        }
        (None, None) => http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    for cookie in answer.cookies {
        response
            .headers_mut()
            .append(http::header::SET_COOKIE, cookie);
    }
    response
}

/// Answers the search panel with what the typed words find, as JSON: `401`
/// without a session, so the page leaves for the sign-in page.
async fn find_answer(
    Extension(state): Extension<AppState>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = request.into_parts();
    let posted = match form_of(&state, &parts, body).await {
        Ok(posted) => posted,
        Err(refusal) => return *refusal,
    };
    let words = posted.text("q");
    let scope = Arc::new(Scope::new(state, parts, Some(posted)));
    crate::scope::run(scope, async move {
        match crate::pages::find::find_answer(words).await {
            Ok(answer) => axum::Json(answer).into_response(),
            Err(crate::error::ViewerError::Unauthenticated) => {
                http::StatusCode::UNAUTHORIZED.into_response()
            }
            Err(error) => (http::StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
        }
    })
    .await
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

/// The session guard, ahead of every page.
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
/// the OIDC handshake, FerroUI's assets, the icons, and the search panel's
/// answer route (which answers `401` itself, as its module expects, instead
/// of a redirect).
fn is_public_path(path: &str) -> bool {
    path == "/login"
        || path == FAVICON_ICO_PATH
        || path == crate::app::ICON_PATH
        || path == crate::shell::MARK_PATH
        || path.starts_with("/auth/")
        || path.starts_with(&format!("{}/", ferrohealth_ui::serve::PREFIX))
        || path.starts_with("/api/")
}


#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        reason = "the exchange helpers panic to report a broken fixture, which is how a test fails (Book ch11); clippy's allow-*-in-tests scoping covers only the #[test] fns themselves"
    )]

    use ferrohealth_server::console::{self, ConsoleHeaders};
    use tower::util::ServiceExt;

    use super::{POLICY, is_public_path};

    /// The console headers as the viewer mounts them.
    fn viewer_headers() -> ConsoleHeaders {
        ConsoleHeaders::new("'self'").with_bundle(ferrohealth_ui::serve::PREFIX)
    }

    /// The sign-in surface, the OIDC handshake, FerroUI's assets and the
    /// search panel's answer route stay reachable without a session; every
    /// screen path is guarded.
    #[test]
    fn the_public_surface_is_exactly_the_sessionless_one() {
        for public in [
            "/login",
            "/favicon.ico",
            "/favicon.svg",
            "/ferroehr-icon.svg",
            "/auth/oidc/login",
            "/auth/oidc/callback",
            "/ferrohealth-ui/0.6.0/ferrohealth-ui.css",
            "/ferrohealth-ui/0.6.0/scripts/fui.js",
            "/api/find_answer",
        ] {
            assert!(is_public_path(public), "{public} must be public");
        }
        for guarded in ["/", "/templates", "/queries/builder", "/ehrs", "/nonsense", "/pkg/x.wasm"] {
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

    /// One response of a stub router at `uri` under the viewer's headers.
    async fn exchange(uri: &str) -> axum::response::Response {
        let headers = viewer_headers();
        axum::Router::new()
            .route("/", axum::routing::get(|| async { "page" }))
            .nest(
                ferrohealth_ui::serve::PREFIX,
                ferrohealth_ui::serve::axum_router::router(),
            )
            .layer(axum::middleware::from_fn(move |request, next| {
                console::headers(headers.clone(), request, next)
            }))
            .layer(axum::middleware::from_fn(super::policy))
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

    /// The directives the policy the viewer serves must carry, each for a
    /// stated reason, and the ones it must never carry: scripts and styles
    /// come from the viewer's own origin, with no nonce and no WebAssembly.
    #[tokio::test]
    async fn the_policy_carries_the_audited_directives() {
        let policy = header(&exchange("/").await, "content-security-policy");
        assert_eq!(policy, POLICY);
        for expected in [
            "default-src 'self'",
            // FerroUI's search module asks the viewer's own answer route.
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
        assert_eq!(directive(&policy, "script-src"), "script-src 'self'");
        assert_eq!(directive(&policy, "style-src"), "style-src 'self'");
        assert!(!policy.contains("unsafe-inline"), "{policy}");
        assert!(!policy.contains("unsafe-eval"), "{policy}");
        assert!(!policy.contains("nonce-"), "{policy}");
    }

    /// Every document keeps `no-store`, since the viewer draws patient data
    /// into HTML; FerroUI's assets keep the library's own long cache, since a
    /// new pin is a new path.
    #[tokio::test]
    async fn only_the_assets_are_kept() {
        assert_eq!("no-store", header(&exchange("/").await, "cache-control"));
        let asset = exchange(&format!(
            "{}/{}/ferrohealth-ui.css",
            ferrohealth_ui::serve::PREFIX,
            ferrohealth_ui::serve::VERSION
        ))
        .await;
        assert!(asset.status().is_success(), "{}", asset.status());
        let caching = header(&asset, "cache-control");
        assert!(caching.contains("immutable"), "{caching}");
        assert_eq!(
            "no-store",
            header(
                &exchange(&format!("{}/no-such-file.js", ferrohealth_ui::serve::PREFIX)).await,
                "cache-control"
            ),
            "a missing asset must not be kept"
        );
    }
}
