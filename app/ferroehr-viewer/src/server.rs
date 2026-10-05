// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's axum service: the router assembly, the pre-render session
//! guard, and the browser security headers.
//!
//! Lives in the lib (ssr-gated) rather than the binary so the integration
//! tests drive the REAL request path — guard, session layer, Leptos render —
//! with `tower::ServiceExt::oneshot`, the same way `app/ferroehr-server`
//! keeps a testable lib run path.

use axum::Extension;
use leptos::prelude::LeptosOptions;
use leptos_axum::LeptosRoutes;

use crate::session::{SESSION_KEY, ViewerSession};
use crate::state::AppState;

/// Assembles the viewer's full axum service.
///
/// OIDC routes, the export POST, the Leptos routes with the per-request
/// context, the static-file fallback, and the layer stack (state → session
/// guard → security headers). Sessions are a sealed cookie
/// ([`crate::session`]) — no session-store layer exists, so no state is
/// bound to any one instance.
pub fn router(app_state: AppState, leptos_options: LeptosOptions) -> axum::Router {
    let routes = leptos_axum::generate_route_list(crate::app::App);

    let context_state = app_state.clone();
    // One context closure for both render entry points (the routed app and the
    // 404 shell): the app state plus this request's CSP nonce.
    let provide_viewer_context = move || {
        leptos::context::provide_context(context_state.clone());
        provide_request_nonce();
    };
    let service = axum::Router::new()
        .route("/auth/oidc/login", axum::routing::get(crate::oidc::login))
        .route(
            "/auth/oidc/callback",
            axum::routing::get(crate::oidc::callback),
        )
        // Result export: a plain form-POST download (no WASM required); the
        // handler enforces the viewer session itself like every server fn.
        .route(
            "/export/aql",
            axum::routing::post(crate::export::export_aql),
        )
        .leptos_routes_with_context(&leptos_options, routes, provide_viewer_context.clone(), {
            let options = leptos_options.clone();
            move || crate::app::shell(options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler_with_context(
            provide_viewer_context,
            crate::app::shell,
        ))
        // Ahead of every render: a document request without a session never
        // reaches Leptos, and an authenticated response leaves with a
        // refreshed sliding-expiry cookie.
        .layer(axum::middleware::from_fn(login_guard))
        .layer(Extension(app_state));
    with_security_headers(service).with_state(leptos_options)
}

/// The pre-render session guard: an unauthenticated document request to a
/// guarded path answers `302 → /login` with an EMPTY body, before Leptos
/// renders anything.
///
/// The in-view guard (`crate::pages::shell::AppShell`) sets the same redirect on
/// the response line, but it cannot suppress the body: the chrome and
/// `<Outlet/>` deliberately live outside its `Suspense`, so without this layer a
/// signed-out hit renders the whole viewer and serializes every screen's
/// server-function failures into the response. The in-view guard remains the
/// client-side navigation gate.
///
/// Scope: `GET`/`HEAD` on everything except the public paths
/// (`is_public_path`). Server-function calls (`/api/…`) keep their own
/// typed refusals; the export POST enforces its own session.
///
/// Fail-closed: a request that reaches this layer without the app state
/// redirects rather than renders. This layer is also the sliding-expiry
/// writer: every response to an authenticated request leaves with a
/// re-sealed cookie carrying a fresh idle anchor — unless the handler
/// already set our cookie itself (login and logout do).
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
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::FOUND;
        response.headers_mut().insert(
            http::header::LOCATION,
            http::HeaderValue::from_static("/login"),
        );
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
        || path == "/favicon.ico"
        || path.starts_with("/auth/")
        || path.starts_with("/pkg/")
        || path.starts_with("/api/")
}

/// The browser security-header set for the viewer, per the OWASP HTTP Headers
/// Cheat Sheet.
///
/// This is a stricter problem than the API's: the viewer serves HTML, hydrates
/// WebAssembly, and holds a session cookie, so a policy here has to survive
/// contact with a real hydrating application.
///
/// - `Content-Security-Policy` — `'wasm-unsafe-eval'` is required, and is the
///   narrow modern replacement for `'unsafe-eval'`: it permits WebAssembly
///   compilation without permitting `eval` of JavaScript. `connect-src 'self'`
///   is correct because the viewer reaches the CDR through its own server
///   functions, never the browser calling the CDR directly (the BFF boundary in
///   `.claude/rules/leptos-ui.md`), so a policy that forbids cross-origin
///   connections costs nothing and forecloses exfiltration.
/// - `script-src` carries no inline allowance: the one inline script the
///   viewer emits is Leptos's hydration bootstrap, and it is authorized by a
///   per-request nonce ([`csp_nonce_layer`]) instead.
/// - `style-src 'unsafe-inline'` stays, and it is the honest remainder of this
///   policy rather than an oversight. `thaw` mounts its generated stylesheets
///   in the browser by creating `<style>` elements through the DOM with no
///   nonce attribute (`thaw_utils::dom::mount_style`), and both `thaw` and
///   `leptos-chartistry` render `style="…"` attributes, which `style-src-attr`
///   inherits. Adding a nonce here would make it strictly WORSE, not better:
///   CSP Level 3 says a nonce in a directive causes `'unsafe-inline'` in that
///   same directive to be ignored, so a nonced `style-src` would block every
///   one of those and leave the viewer unstyled.
/// - `X-Frame-Options: DENY` plus `frame-ancestors 'none'` — belt and braces
///   against clickjacking a viewer that performs administrative writes.
/// - `Cache-Control` — `no-store` on every document, because the viewer
///   renders patient data into HTML and because a nonced document must never
///   sit in a shared cache; the hashed `/pkg/*` bundle is the one exception
///   ([`cache_control_for`]).
///
/// `Strict-Transport-Security` is left to the TLS edge, for the same reason as
/// on the API: RFC 6797 §7.2 makes it inert over plain HTTP.
fn with_security_headers(router: axum::Router<LeptosOptions>) -> axum::Router<LeptosOptions> {
    use tower_http::set_header::SetResponseHeaderLayer;
    router
        .layer(axum::middleware::from_fn(csp_nonce_layer))
        .layer(SetResponseHeaderLayer::overriding(
            http::header::X_CONTENT_TYPE_OPTIONS,
            http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            http::header::REFERRER_POLICY,
            http::HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            http::header::X_FRAME_OPTIONS,
            http::HeaderValue::from_static("DENY"),
        ))
        .layer(axum::middleware::from_fn(cache_control_layer))
}

/// The hydration bundle's caching directive: RFC 9111 §5.2.2.9 `public` +
/// §5.2.2.1 `max-age`, and RFC 8246 `immutable`.
///
/// One year is the conventional ceiling for a content-addressed asset
/// (<https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cache-Control#caching_static_assets_with_hashed_filenames>),
/// and `immutable` is what stops the browser revalidating on a reload it would
/// otherwise treat as a reason to ask again (RFC 8246 §2).
const IMMUTABLE_ASSET_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// The viewer default: never store the response anywhere (RFC 9111 §5.2.2.5).
const NO_STORE_CACHE_CONTROL: &str = "no-store";

/// Stamps each response with the caching directive its path and status earn.
async fn cache_control_layer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let value = cache_control_for(&path, response.status());
    response
        .headers_mut()
        .insert(http::header::CACHE_CONTROL, value);
    response
}

/// Which `Cache-Control` value one response carries.
///
/// `/pkg/` is the cargo-leptos hydration bundle and NOTHING else: the WASM,
/// its JS glue, and the stylesheet. Their filenames carry a content hash
/// (`hash-files` in the crate manifest), so a changed asset is a changed URL
/// and a cached copy can never go stale — which is what makes `immutable` an
/// honest claim rather than a bet on the deploy cadence. The bundle holds no
/// patient data either: the viewer reaches the CDR through its own server
/// functions, so nothing clinical is ever compiled into it.
///
/// `/pkg/snippets/` is carved back out because cargo-leptos deliberately does
/// NOT hash those files — the WebAssembly looks for them by their unhashed
/// names — so their URLs are stable across builds and caching one for a year
/// would pin a stale copy. This build emits none; the carve-out keeps that from
/// becoming a silent defect if a dependency ever adds one.
///
/// Everything else keeps `no-store`: the viewer renders patient data into HTML,
/// and each document carries a per-request CSP nonce that must not be replayed
/// out of a cache.
///
/// A refused or missing asset is never cached: only a served body (2xx) and
/// the `304` a revalidation answers with take the immutable directive, so a
/// deploy racing a request cannot freeze a 404 into a browser for a year.
fn cache_control_for(path: &str, status: http::StatusCode) -> http::HeaderValue {
    let cacheable = status.is_success() || status == http::StatusCode::NOT_MODIFIED;
    let hashed_asset = path.starts_with("/pkg/") && !path.starts_with("/pkg/snippets/");
    if hashed_asset && cacheable {
        http::HeaderValue::from_static(IMMUTABLE_ASSET_CACHE_CONTROL)
    } else {
        http::HeaderValue::from_static(NO_STORE_CACHE_CONTROL)
    }
}

/// Mints this response's script nonce and answers with the policy that names
/// it.
///
/// The nonce is minted here and nowhere else: the layer puts it in the request
/// extensions on the way in, so the renderer stamps the same value on the
/// bootstrap script ([`provide_request_nonce`]), and writes the header on the
/// way out. A fresh value per request is the whole point — a nonce reused
/// across responses is guessable, and therefore worth no more than
/// `'unsafe-inline'`.
async fn csp_nonce_layer(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let nonce = leptos::nonce::Nonce::new();
    #[expect(
        clippy::expect_used,
        reason = "viewer_csp interpolates a base64url nonce into an ASCII template, so every byte is a legal header-value character; falling back to no policy would silently ship the viewer unprotected"
    )]
    let policy = http::HeaderValue::from_str(&viewer_csp(&nonce))
        .expect("an ASCII policy string should be a valid header value");
    request.extensions_mut().insert(nonce);
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(http::header::CONTENT_SECURITY_POLICY, policy);
    response
}

/// Re-provides this request's nonce into the Leptos render context.
///
/// `leptos_axum` provides a nonce of its own before the per-request context
/// closure runs, and that one is not the value the response header authorizes.
/// Overriding it here is what makes the two the same string; without it the
/// bootstrap script carries a nonce the browser has never been told to trust,
/// and the viewer renders but never hydrates.
fn provide_request_nonce() {
    let Some(parts) = leptos::context::use_context::<http::request::Parts>() else {
        return;
    };
    if let Some(nonce) = parts.extensions.get::<leptos::nonce::Nonce>() {
        leptos::context::provide_context(nonce.clone());
    }
}

/// The viewer's Content-Security-Policy for one request, naming that
/// request's script nonce.
fn viewer_csp(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         script-src 'self' 'wasm-unsafe-eval' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; \
         font-src 'self'; \
         connect-src 'self'; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        reason = "the exchange helpers panic to report a broken fixture, which is how a test fails (Book ch11); clippy's allow-*-in-tests scoping covers only the #[test] fns themselves"
    )]

    use super::{
        IMMUTABLE_ASSET_CACHE_CONTROL, NO_STORE_CACHE_CONTROL, cache_control_for, csp_nonce_layer,
        is_public_path, provide_request_nonce, viewer_csp,
    };
    use tower::util::ServiceExt;

    /// A sample policy for the directive assertions; the value stands in for a
    /// minted nonce and is deliberately unlike any other token in the string.
    const SAMPLE_NONCE: &str = "SAMPLENONCEVALUE";

    /// The response header the stub handler reports the render-context nonce
    /// through, so a test can compare it with the policy.
    const RENDERED_NONCE: &str = "x-test-rendered-nonce";

    /// The sign-in surface, the OIDC handshake, the hydration assets and the
    /// server-fn endpoints stay reachable without a session; every screen
    /// path is guarded.
    #[test]
    fn the_public_surface_is_exactly_the_sessionless_one() {
        for public in [
            "/login",
            "/favicon.ico",
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

    /// The hydration bundle is the ONLY thing that may be cached, and every
    /// document — the sign-in screen included — keeps `no-store`: the viewer
    /// renders patient data into HTML and stamps a per-request CSP nonce on
    /// it.
    #[test]
    fn only_the_hashed_bundle_is_cacheable() {
        let ok = http::StatusCode::OK;
        for asset in [
            "/pkg/ferroehr-viewer.RUlUrl0ZR7DGPjuVWkKtwA.wasm",
            "/pkg/ferroehr-viewer.RUlUrl0ZR7DGPjuVWkKtwA.js",
            "/pkg/ferroehr-viewer.RUlUrl0ZR7DGPjuVWkKtwA.css",
        ] {
            assert_eq!(
                cache_control_for(asset, ok),
                IMMUTABLE_ASSET_CACHE_CONTROL,
                "{asset} is content-hashed and must be cacheable"
            );
        }
        for document in [
            "/",
            "/login",
            "/ehrs",
            "/favicon.ico",
            "/api/current_session",
        ] {
            assert_eq!(
                cache_control_for(document, ok),
                NO_STORE_CACHE_CONTROL,
                "{document} may carry patient data or a nonce and must not be stored"
            );
        }
        assert_eq!(
            cache_control_for("/pkg/snippets/some-dep/inline0.js", ok),
            NO_STORE_CACHE_CONTROL,
            "cargo-leptos leaves snippet filenames unhashed, so their URLs repeat across builds"
        );
    }

    /// A missing or refused asset must never be frozen into a browser for a
    /// year; a revalidation's `304` must keep the directive it revalidated.
    #[test]
    fn a_refused_asset_is_never_cached() {
        for refused in [
            http::StatusCode::NOT_FOUND,
            http::StatusCode::FOUND,
            http::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                cache_control_for("/pkg/ferroehr-viewer.js", refused),
                NO_STORE_CACHE_CONTROL,
                "a {refused} for a /pkg path must not be cached"
            );
        }
        assert_eq!(
            cache_control_for("/pkg/ferroehr-viewer.js", http::StatusCode::NOT_MODIFIED),
            IMMUTABLE_ASSET_CACHE_CONTROL
        );
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

    /// Drives one request through the nonce layer, returning the policy the
    /// response carries and the nonce the render context saw.
    async fn one_exchange() -> (String, String) {
        let response = axum::Router::new()
            .route("/", axum::routing::get(render_nonce))
            .layer(axum::middleware::from_fn(csp_nonce_layer))
            .oneshot(
                http::Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let header = |name: http::HeaderName| {
            response
                .headers()
                .get(&name)
                .unwrap_or_else(|| panic!("the response must carry {name}"))
                .to_str()
                .unwrap()
                .to_owned()
        };
        (
            header(http::header::CONTENT_SECURITY_POLICY),
            header(http::HeaderName::from_static(RENDERED_NONCE)),
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
        let nonce = leptos::nonce::Nonce::new();
        let policy = viewer_csp(&nonce);
        let owner = leptos::prelude::Owner::new();
        let html = owner.with(|| {
            leptos::context::provide_context(nonce);
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

    /// The directives the policy must carry, each for a stated reason.
    #[test]
    fn the_policy_carries_the_audited_directives() {
        let policy = viewer_csp(SAMPLE_NONCE);
        for expected in [
            "default-src 'self'",
            // WebAssembly compilation, without permitting eval of JavaScript.
            "'wasm-unsafe-eval'",
            // The viewer reaches the CDR through its own server functions, so
            // the browser never needs a cross-origin connection.
            "connect-src 'self'",
            "object-src 'none'",
            "base-uri 'self'",
            "frame-ancestors 'none'",
        ] {
            assert!(
                policy.contains(expected),
                "the policy must carry {expected}"
            );
        }
    }

    /// `'unsafe-eval'` is what `'wasm-unsafe-eval'` exists to avoid, so the
    /// broad form must never appear — including by someone "fixing" a WASM
    /// error with the bigger hammer.
    #[test]
    fn the_policy_never_permits_eval() {
        assert!(!viewer_csp(SAMPLE_NONCE).contains("'unsafe-eval'"));
    }

    /// The script half is strict: a nonce and no inline allowance. Re-adding
    /// `'unsafe-inline'` here would not merely widen the policy, it would be
    /// ignored outright — CSP Level 3 drops it from any directive that also
    /// carries a nonce — so the regression would look like a working policy.
    #[test]
    fn script_src_is_nonce_only() {
        let policy = viewer_csp(SAMPLE_NONCE);
        let script_src = directive(&policy, "script-src");
        assert!(script_src.contains(&format!("'nonce-{SAMPLE_NONCE}'")));
        assert!(
            !script_src.contains("'unsafe-inline'"),
            "the nonce replaces the inline allowance: {script_src}"
        );
    }

    /// The style half keeps its inline allowance and must never gain a nonce:
    /// `thaw` creates its stylesheets through the DOM without one, so a nonced
    /// `style-src` would suppress `'unsafe-inline'` and unstyle the viewer.
    #[test]
    fn style_src_keeps_its_inline_allowance_and_no_nonce() {
        let policy = viewer_csp(SAMPLE_NONCE);
        let style_src = directive(&policy, "style-src");
        assert!(style_src.contains("'unsafe-inline'"));
        assert!(
            !style_src.contains("'nonce-"),
            "a nonce here would disable 'unsafe-inline': {style_src}"
        );
    }
}
