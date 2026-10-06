// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! In-process SSR render tests for the persistent CHROME
//! ([`ferroehr_viewer::pages::shell`]).
//!
//! The sibling `ssr_components` renders the shared kit and `ssr_pages` renders
//! one screen at a time; neither can reach the shell. The shell is a LAYOUT,
//! not a screen: it renders the matched child through `<Outlet/>`, and
//! `<Outlet/>` panics ("Outlet used without `RouteContext`") outside a matched
//! `<Routes>` tree, which no public API constructs standalone. The only way in
//! is to render the WHOLE route tree for a concrete URL.
//!
//! So this module renders the viewer the way its own binary does. Every
//! authenticated route declares `ssr=leptos_router::SsrMode::Async`
//! ([`ferroehr_viewer::app::App`]), and `leptos_axum` answers that mode with
//! `app.to_html_stream_in_order().collect::<String>().await` over
//! [`ferroehr_viewer::app::shell`] — which is exactly what
//! `leptos_axum::render_app_async_with_context` is: the same handler
//! `main.rs`'s `leptos_routes_with_context` mounts, minus the router. It is
//! driven here with a hand-built `GET` request and no listener.
//!
//! Two consequences of taking the real path are load-bearing for what these
//! tests assert:
//!
//! * **Every resource RESOLVES.** This is not the fallback-only pass
//!   `RenderHtml::to_html` produces (that renders each `<Suspense>`'s fallback
//!   and never polls its body). The chrome's four probe gates, its session
//!   read and its health pill all reach a decided state here.
//! * **Nothing touches the network.** The harness provides no axum `Parts`, so
//!   `crate::session::http_session`'s `leptos_axum::extract` returns an error
//!   rather than a session, and every server fn behind
//!   `crate::session::require_session` — the four probes, `fetch_status`, and
//!   every screen read — fails before it builds a URL. The state's CDR client
//!   is still provided (a server fn body `expect_context`s it) and still
//!   points at an unroutable port, but no request is ever attempted.
//!
//! What that pins is the chrome a signed-out browser is served: the full static
//! sidebar with every gated entry hidden, the topbar reporting an unreachable
//! CDR, the footer, the matched screen inside `<main>` — and the session
//! guard's `302` to `/login` on the response itself, which is where a
//! server-side `<Redirect/>` lands (it renders no HTML; `leptos_router`'s
//! `Redirect` calls the `ServerRedirectFunction` in context, and `leptos_axum`
//! makes that set `Location` plus — for an `Accept: text/html` request — the
//! `302` status).
//!
//! `ssr`-gated like its siblings: the render path is the server one, and the
//! `ssr` feature is what puts `leptos-use`/`thaw`/`leptos-chartistry` on their
//! non-`WASM` code paths (`Cargo.toml` §features).

#![cfg(feature = "ssr")]

use leptos::prelude::LeptosOptions;

/// What one server pass produced: the response line that carries the session
/// guard's decision, and the document body that carries the chrome.
#[derive(Debug)]
struct ServerPass {
    /// The response status — `302` when the shell's guard redirected.
    status: http::StatusCode,
    /// The `Location` header, present exactly when a redirect was issued.
    location: Option<String>,
    /// The rendered document.
    html: String,
}

/// Renders one URL through the viewer's own async server pass.
///
/// `Accept: text/html` is a browser's own header, and `leptos_axum::redirect`
/// reads it to decide between setting only `Location` (a server-fn caller) and
/// setting the `302` as well (a document request) — so the request has to carry
/// it for the guard's real behaviour to be observable.
#[expect(
    clippy::expect_used,
    reason = "every step here is fixture construction over constants — a static URI, an empty \
              body, the UTF-8 the renderer just wrote; a fixture that cannot be built is a broken \
              harness, not a test outcome"
)]
async fn render_route(url: &str) -> ServerPass {
    // The one public `leptos_axum` entry point that initializes the reactive
    // executor (`Executor::init_tokio`) the way the viewer's own server does;
    // an app fn with no `<Router>` yields no routes and does no work.
    drop(leptos_axum::generate_route_list(|| ()));
    let options = LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .build();
    let handler = leptos_axum::render_app_async_with_context(
        move || leptos::prelude::provide_context(app_state()),
        move || ferroehr_viewer::app::shell(options.clone()),
    );
    let request = http::Request::builder()
        .uri(url)
        .header(http::header::ACCEPT, "text/html")
        .body(axum::body::Body::empty())
        .expect("the request fixture should build from a static URI and an empty body");
    let (parts, body) = handler(request).await.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("the rendered document should read back in full from an in-memory body");
    ServerPass {
        status: parts.status,
        location: parts
            .headers
            .get(http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        html: String::from_utf8(bytes.to_vec())
            .expect("the renderer writes UTF-8, so the document should decode"),
    }
}

/// The BFF state every `#[server]` fn resolves, aimed at a port nothing serves.
///
/// It is provided because a server fn body `expect_context`s it, not because
/// anything reaches the CDR: every fn the chrome calls guards on the session
/// first, and this harness has no session to find.
#[expect(
    clippy::expect_used,
    reason = "CdrClient::new fails only if reqwest cannot build a client from a fixed timeout and \
              redirect policy; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]
fn app_state() -> ferroehr_viewer::state::AppState {
    let config = ferroehr_viewer::config::ViewerConfig {
        cdr: ferroehr_viewer::config::CdrConfig {
            base_url: "http://127.0.0.1:9".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    ferroehr_viewer::state::AppState {
        cdr: ferroehr_viewer::cdr::CdrClient::new(&config.cdr)
            .expect("the CDR client should build from a well-formed base URL"),
        config: std::sync::Arc::new(config),
        oidc: None,
        session_keys: ferroehr_viewer::session::SessionKeys::from_secret("")
            .expect("an empty secret always yields an ephemeral key"),
    }
}

/// The document's `<aside>` — the sidebar and nothing else, so the topbar's
/// wordmark link cannot be mistaken for a nav entry.
fn sidebar(html: &str) -> &str {
    html.split_once("<aside")
        .and_then(|(_, rest)| rest.split_once("</aside>"))
        .map_or("", |(inside, _)| inside)
}

/// The sidebar's rendered entries in render order, one token each:
/// `"<href> <label>"`, with `" (active)"` appended to the entry the current
/// URL highlights.
///
/// Reading the anchors back out of the HTML — rather than asserting a bag of
/// `contains` calls — is what makes the CLOSED set assertable: an entry that
/// appears, disappears, moves, or loses its label changes this vector.
fn sidebar_entries(html: &str) -> Vec<String> {
    sidebar(html)
        .split("<li>")
        .filter_map(|item| {
            let (_, rest) = item.split_once("href=\"")?;
            let (href, rest) = rest.split_once('"')?;
            // Everything from the href to the anchor's close: the remaining
            // attributes (`aria-current` among them) and the anchor's content.
            let (body, _) = rest.split_once("</a>")?;
            // The label is the anchor's trailing text, after the icon `<svg>`
            // (and any marker comment leptos writes between siblings).
            let (_, label) = body.rsplit_once('>')?;
            let active = body.contains("aria-current=\"page\"");
            Some(format!(
                "{href} {label}{}",
                if active { " (active)" } else { "" }
            ))
        })
        .collect()
}

/// The eight entries every deployment shows, in the order the shell's `NAV_SLOTS`
/// table declares them — the domain group, then (across the divider) the meta
/// group with `System` last.
const STATIC_SIDEBAR: [&str; 8] = [
    "/ Dashboard",
    "/templates Templates",
    "/queries Queries",
    "/ehrs EHRs",
    "/demographics/person Demographics",
    "/terminology Terminology",
    "/audit Audit log",
    "/system System",
];

/// The three sidebar entries a CDR probe decides, each hidden when its probe
/// does not report the surface mounted.
const GATED_SIDEBAR_HREFS: [&str; 3] = ["/fhir", "/subscriptions", "/operations"];

/// Marks the entry at `index` of [`STATIC_SIDEBAR`] active and returns the whole
/// expected sidebar.
fn sidebar_with_active(index: usize) -> Vec<String> {
    STATIC_SIDEBAR
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            if i == index {
                format!("{entry} (active)")
            } else {
                (*entry).to_owned()
            }
        })
        .collect()
}

// ------------------------------------------------------------------ sidebar

#[tokio::test]
async fn the_shell_renders_its_whole_static_sidebar_around_the_matched_dashboard() {
    let pass = render_route("/").await;
    assert_eq!(sidebar_entries(&pass.html), sidebar_with_active(0));
    // The hairline between the domain group and the meta group, drawn once and
    // hidden from assistive technology.
    assert_eq!(
        sidebar(&pass.html)
            .matches("aria-hidden=\"true\" class=\"my-1.5 border-t border-edge\"")
            .count(),
        1,
        "{}",
        pass.html
    );
    // Exactly one entry is highlighted, and the nav is a real landmark.
    assert_eq!(
        sidebar(&pass.html).matches("aria-current=\"page\"").count(),
        1,
        "{}",
        pass.html
    );
    assert!(pass.html.contains("aria-label=\"Main\""), "{}", pass.html);
}

#[tokio::test]
async fn the_shell_renders_the_same_sidebar_around_a_second_screen_and_moves_the_highlight() {
    let pass = render_route("/templates").await;
    assert_eq!(sidebar_entries(&pass.html), sidebar_with_active(1));
}

/// The four optional CDR surfaces are probe-and-hide: this pass has no session,
/// so every probe fails before it reaches the CDR and the viewer offers no link
/// to a screen it cannot know is served.
#[tokio::test]
async fn a_probe_that_cannot_answer_leaves_its_nav_entry_out_of_the_sidebar() {
    let pass = render_route("/").await;
    let aside = sidebar(&pass.html);
    for href in GATED_SIDEBAR_HREFS {
        assert!(
            !aside.contains(&format!("href=\"{href}\"")),
            "{href} should not be offered: {aside}"
        );
    }
    // Hiding all four still leaves the divider with content on both sides.
    assert_eq!(sidebar_entries(&pass.html).len(), STATIC_SIDEBAR.len());
}

// ------------------------------------------------------------------- topbar

#[tokio::test]
async fn the_topbar_carries_the_wordmark_the_health_pill_the_user_menu_and_the_dark_toggle() {
    let pass = render_route("/").await;
    let header = pass
        .html
        .split_once("<header")
        .and_then(|(_, rest)| rest.split_once("</header>"))
        .map_or("", |(inside, _)| inside);
    // The mobile nav toggle and the wordmark home link.
    assert!(
        header.contains("aria-label=\"Toggle navigation\""),
        "{header}"
    );
    assert!(header.contains("href=\"/\""), "{header}");
    assert!(header.contains("FerroEHR"), "{header}");
    // The health pill is a <Transition>, so "checking…" is only ever the
    // reload fallback; an async pass always serves a DECIDED chip. This pass
    // carries no session, so the status read is refused before it ever reaches
    // the CDR — and the chip says the session ended rather than blaming the
    // CDR for a refusal that is not its.
    assert!(header.contains("Session ended"), "{header}");
    assert!(!header.contains("CDR DOWN"), "{header}");
    assert!(!header.contains("bg-danger"), "{header}");
    assert!(!header.contains("checking…"), "{header}");
    // The user menu's stable trigger id (the E2E journeys target it) and the
    // dark-mode control, which starts on the light theme's moon icon.
    assert!(header.contains("id=\"user-menu-trigger\""), "{header}");
    assert!(
        header.contains("aria-label=\"Toggle dark mode\""),
        "{header}"
    );
}

/// The access drawer is a `thaw::OverlayDrawer`, which mounts through a
/// `Teleport` gated on its open signal — so a closed drawer contributes nothing
/// to the server document, and its body arrives only after a click on
/// "View scopes" in the hydrated user menu.
#[tokio::test]
async fn the_closed_access_drawer_contributes_nothing_to_the_server_document() {
    let pass = render_route("/").await;
    assert!(!pass.html.contains("id=\"access-drawer\""), "{}", pass.html);
    assert!(!pass.html.contains("Access scopes"), "{}", pass.html);
}

// ------------------------------------------------------------------- footer

#[tokio::test]
async fn the_footer_reports_the_viewer_version_and_this_session_s_scope_count() {
    let pass = render_route("/").await;
    let footer = pass
        .html
        .split_once("<footer")
        .and_then(|(_, rest)| rest.split_once("</footer>"))
        .map_or("", |(inside, _)| inside);
    assert!(
        footer.contains(&format!("viewer v{}", env!("CARGO_PKG_VERSION"))),
        "{footer}"
    );
    // No session, so no scopes — the count is a fact about the session, not a
    // placeholder.
    assert!(footer.contains("0 scope(s)"), "{footer}");
}

/// The footer names the manufacturer and links to its details on `/system`
/// (Regulation (EU) 2025/327, `docs/law/eu/ehds/text.html` Art. 30(1)(g)).
#[tokio::test]
async fn the_footer_names_the_manufacturer() {
    let pass = render_route("/").await;
    let footer = pass
        .html
        .split_once("<footer")
        .and_then(|(_, rest)| rest.split_once("</footer>"))
        .map_or("", |(inside, _)| inside);
    assert!(footer.contains("Manufactured by Cadasto B.V."), "{footer}");
    assert!(footer.contains("href=\"/system#manufacturer\""), "{footer}");
}

// ------------------------------------------------------------------- outlet

#[tokio::test]
async fn the_matched_screen_renders_inside_the_shell_s_main_region() {
    for (url, heading, title) in [
        (
            "/",
            ">Dashboard</h1>",
            "<title>Dashboard · FerroEHR Viewer</title>",
        ),
        (
            "/templates",
            ">Templates</h1>",
            "<title>Templates · FerroEHR Viewer</title>",
        ),
    ] {
        let pass = render_route(url).await;
        // Inside <main>, not merely somewhere in the document: that is what
        // proves the <Outlet/> resolved rather than the screen being rendered
        // standalone.
        let main = pass
            .html
            .split_once("<main")
            .and_then(|(_, rest)| rest.split_once("</main>"))
            .map_or("", |(inside, _)| inside);
        assert!(main.contains(heading), "{url}: {main}");
        // The document is the real one the binary serves, head and all.
        assert!(pass.html.contains(title), "{url}: {}", pass.html);
    }
}

// ------------------------------------------------------------ session guard

/// One request through the REAL service — `ferroehr_viewer::server::router`
/// is the assembly the binary serves (session layer, the pre-render login
/// guard, the Leptos routes, the security headers) — with no session cookie.
#[expect(
    clippy::expect_used,
    reason = "every step here is fixture construction over constants — a static URI, an empty \
              body, an in-memory response, the UTF-8 the server just wrote; a fixture that \
              cannot be built is a broken harness, not a test outcome"
)]
async fn routed_response(url: &str) -> ServerPass {
    use tower::util::ServiceExt;

    // Initializes the reactive executor exactly as `render_route` does.
    drop(leptos_axum::generate_route_list(|| ()));
    let options = LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .build();
    let service = ferroehr_viewer::server::router(app_state(), options);
    let request = http::Request::builder()
        .uri(url)
        .header(http::header::ACCEPT, "text/html")
        .body(axum::body::Body::empty())
        .expect("the request fixture should build from a static URI and an empty body");
    let (parts, body) = service
        .oneshot(request)
        .await
        .expect("the service is infallible")
        .into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("the response body should read back in full from an in-memory body");
    ServerPass {
        status: parts.status,
        location: parts
            .headers
            .get(http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        html: String::from_utf8(bytes.to_vec())
            .expect("the response body is UTF-8, so the document should decode"),
    }
}

/// The signed-out `302` carries an EMPTY body (#2702): the pre-render login
/// guard (`server::login_guard`) answers before Leptos renders anything, so
/// no chrome, screen markup, or serialized server-fn diagnostics leave the
/// server — and no render work is spent on an unauthenticated hit. Driven
/// through the real service, because the guard is a router layer the bare
/// render handler (`render_route`) never passes.
#[tokio::test]
async fn a_guarded_url_without_a_session_answers_302_to_the_login_screen() {
    for url in ["/", "/templates", "/queries/builder", "/nonsense"] {
        let pass = routed_response(url).await;
        assert_eq!(pass.status, http::StatusCode::FOUND, "{url}");
        assert_eq!(pass.location.as_deref(), Some("/login"), "{url}");
        assert_eq!(pass.html, "", "{url}: the redirect body must be empty");
    }
}

/// The sign-in screen stays public through the same real service: the guard
/// exempts it, the render pass runs, and the full document is served.
#[tokio::test]
async fn the_login_screen_is_served_without_a_session() {
    let pass = routed_response("/login").await;
    assert_eq!(pass.status, http::StatusCode::OK);
    assert!(
        pass.html.contains("<title>"),
        "the login document renders: {}",
        pass.html
    );
}

/// The signed-out transition's landing: `?expired=1` opens the sign-in card
/// with the session-ended notice, and a plain visit to `/login` does not.
///
/// The notice is server-rendered from the URL alone, which is what keeps it
/// hydration-stable and what the expiry journeys wait on
/// (`#session-expired`).
#[tokio::test]
async fn the_login_screen_says_the_session_ended_only_when_the_url_says_so() {
    let expired = render_route("/login?expired=1&next=%2Fehrs").await;
    assert!(
        expired.html.contains("id=\"session-expired\""),
        "{}",
        expired.html
    );
    assert!(
        expired
            .html
            .contains("Your session ended. Sign in again to continue."),
        "{}",
        expired.html
    );
    // The destination survives the round trip, so signing in returns the user
    // to the screen the expiry took them off.
    assert!(expired.html.contains("value=\"/ehrs\""), "{}", expired.html);

    let plain = render_route("/login").await;
    assert!(
        !plain.html.contains("id=\"session-expired\""),
        "{}",
        plain.html
    );
}

/// The document links its stylesheet from the SHELL's `<head>`.
///
/// The link cannot come from a component body any more: with content-hashed
/// `/pkg` filenames its href is a build fact the server reads out of the
/// cargo-leptos hash manifest, which only the shell has (`LeptosOptions`).
/// Losing it renders a viewer that works and looks broken, and no other test
/// would notice — the screens assert their own markup, never the head.
#[tokio::test]
async fn the_shell_links_the_bundle_stylesheet_from_the_document_head() {
    let pass = render_route("/login").await;
    let head = pass
        .html
        .split_once("<body")
        .map_or(pass.html.as_str(), |(head, _)| head);
    assert!(
        head.contains(r#"rel="stylesheet""#) && head.contains("/pkg/ferroehr-viewer.css"),
        "the shell must link the bundle stylesheet in <head>: {head}"
    );
}

/// The in-view guard (`AppShell`'s `<Redirect/>`) still stands behind the
/// router layer: rendered bare (no axum service, so no login guard), a
/// guarded URL keeps answering `302 → /login` on the response line — the
/// client-side navigation gate the hydrated app relies on.
#[tokio::test]
async fn the_in_view_guard_still_redirects_without_the_router_layer() {
    for url in ["/", "/templates"] {
        let pass = render_route(url).await;
        assert_eq!(pass.status, http::StatusCode::FOUND, "{url}");
        assert_eq!(pass.location.as_deref(), Some("/login"), "{url}");
    }
}
