// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer through its REAL service, in process: the router the binary
//! serves, a session made by signing in through the sign-in form, and every
//! screen drawn whole on the server, as islands mode draws it.
//!
//! The CDR is a replay of a recording (`tests/fixtures/cdr/recording.json`):
//! every exchange the viewer made with a running FerroEHR 5.3.2 while it drew
//! these screens over the data `tests/fixtures/cdr/manifest.json` names. A
//! request matches the first recorded exchange with its method, path and,
//! for a post, its body, then the first with its method and path; the sign-in
//! probe answers by the credential it carries. A screen whose requests change
//! needs a new recording (`tests/fixtures/README.md`).
//!
//! What these tests pin is the server half of every page: the heading, the
//! fields and their names the forms post, the words a reader reads, and the
//! posts that sign in, sign out, run a query and keep a preference. The
//! browser journeys (`e2e_*`) drive the same pages with WebAssembly.

#![cfg(not(target_arch = "wasm32"))]
#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction: a loopback listener, the recorded \
              exchanges, a static request; a fixture that cannot be built is a broken harness, \
              not a test outcome"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the recording is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use std::sync::Arc;

use base64::Engine as _;
use leptos::prelude::LeptosOptions;
use serde_json::Value;
use tower::util::ServiceExt;

/// The recorded exchanges, read once.
fn recording() -> Arc<[Value]> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cdr/recording.json");
    let text = std::fs::read_to_string(path).expect("the recording should be readable");
    let value: Value = serde_json::from_str(&text).expect("the recording should be JSON");
    Arc::<[Value]>::from(
        value
            .as_array()
            .cloned()
            .expect("the recording should be a list"),
    )
}

/// The ids the recording's data carries.
fn manifest() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cdr/manifest.json");
    let text = std::fs::read_to_string(path).expect("the manifest should be readable");
    serde_json::from_str(&text).expect("the manifest should be JSON")
}

/// One id of the manifest.
fn id(key: &str) -> String {
    manifest()
        .get(key)
        .and_then(Value::as_str)
        .expect("the manifest should name every id")
        .to_owned()
}

/// Whether a request carries the recording's own credential.
fn signed(headers: &http::HeaderMap) -> bool {
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("ferroehr:ferroehr")
    );
    headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| value == expected)
}

/// The recorded exchange that answers a request, by the rules the module
/// docs name.
fn answer<'a>(
    recorded: &'a [Value],
    method: &str,
    path: &str,
    body: &str,
    credential_ok: bool,
) -> Option<&'a Value> {
    let same = |exchange: &&Value| {
        exchange["method"].as_str() == Some(method) && exchange["path"].as_str() == Some(path)
    };
    let wanted_status = |exchange: &&Value| {
        let refused = exchange["status"]
            .as_u64()
            .and_then(|status| u16::try_from(status).ok())
            .and_then(|status| http::StatusCode::from_u16(status).ok())
            == Some(http::StatusCode::UNAUTHORIZED);
        credential_ok != refused
    };
    let query_of = |text: &str| {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value.get("q").cloned())
    };
    recorded
        .iter()
        .filter(same)
        .filter(wanted_status)
        .find(|exchange| exchange["body"].as_str() == Some(body))
        .or_else(|| {
            let asked = query_of(body)?;
            recorded
                .iter()
                .filter(same)
                .filter(wanted_status)
                .find(|exchange| {
                    query_of(exchange["body"].as_str().unwrap_or("")) == Some(asked.clone())
                })
        })
        .or_else(|| recorded.iter().filter(same).find(wanted_status))
}

/// Starts the replayed CDR with `stubs`, exchanges in the recording's shape
/// that answer before it, and returns its base URL.
///
/// A stub stands in for a CDR answer the recording has no screen for (a
/// demographic template, for example); it is written by the test that needs
/// it, never added to the recording.
async fn replayed_cdr_with(stubs: Vec<Value>) -> String {
    let recorded: Arc<[Value]> = stubs
        .into_iter()
        .chain(recording().iter().cloned())
        .collect();
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let recorded = Arc::clone(&recorded);
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 1 << 20)
                .await
                .expect("a test request body fits");
            let body = String::from_utf8_lossy(&bytes);
            let path = parts
                .uri
                .path_and_query()
                .map_or_else(|| parts.uri.path().to_owned(), |pq| pq.as_str().to_owned());
            let found = answer(
                &recorded,
                parts.method.as_str(),
                &path,
                &body,
                signed(&parts.headers),
            );
            let Some(exchange) = found else {
                return http::Response::builder()
                    .status(http::StatusCode::NOT_FOUND)
                    .body(axum::body::Body::from(format!("not recorded: {path}")))
                    .expect("a static response builds");
            };
            let mut response = http::Response::builder()
                .status(u16::try_from(exchange["status"].as_u64().unwrap_or(500)).unwrap_or(500));
            if let Some(headers) = exchange["headers"].as_object() {
                for (name, value) in headers {
                    if let Some(value) = value.as_str() {
                        response = response.header(name.as_str(), value);
                    }
                }
            }
            response
                .body(axum::body::Body::from(
                    exchange["response"].as_str().unwrap_or("").to_owned(),
                ))
                .expect("a recorded response builds")
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the replayed CDR serves until the test ends");
    });
    format!("http://{addr}")
}

/// The viewer's state, pointed at `cdr`, with a fixed session key.
fn app_state(cdr: &str) -> ferroehr_viewer::state::AppState {
    let config = ferroehr_viewer::config::ViewerConfig {
        cdr: ferroehr_viewer::config::CdrConfig {
            base_url: cdr.to_owned(),
            management_base_url: format!("{cdr}/management"),
            ..Default::default()
        },
        ..Default::default()
    };
    ferroehr_viewer::state::AppState {
        cdr: ferroehr_viewer::cdr::CdrClient::new(&config.cdr)
            .expect("the CDR client should build from a well-formed base URL"),
        config: Arc::new(config),
        oidc: None,
        session_keys: ferroehr_viewer::session::SessionKeys::from_secret("")
            .expect("an empty secret always yields an ephemeral key"),
    }
}

/// The viewer's service over the replayed CDR.
struct Viewer {
    /// The router the binary serves.
    service: axum::Router,
}

/// One answer of the viewer: its status, its headers and its body.
struct Answer {
    /// The status.
    status: http::StatusCode,
    /// The headers.
    headers: http::HeaderMap,
    /// The body, as text.
    body: String,
}

impl Answer {
    /// The `Location` it sends the browser to.
    fn location(&self) -> &str {
        self.headers
            .get(http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    }

    /// The cookies it sets, as `name=value` pairs.
    fn cookies(&self) -> Vec<String> {
        self.headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.split(';').next().map(str::to_owned))
            .collect()
    }

    /// The page's main region, the part a screen draws inside the shell.
    fn main(&self) -> &str {
        let start = self.body.find("<main").unwrap_or(0);
        let end = self.body.find("</main>").unwrap_or(self.body.len());
        self.body.get(start..end).unwrap_or(&self.body)
    }
}

impl Viewer {
    /// The viewer over a fresh replayed CDR.
    async fn start() -> Self {
        Self::start_with(Vec::new()).await
    }

    /// The viewer over a fresh replayed CDR answering `stubs` first.
    async fn start_with(stubs: Vec<Value>) -> Self {
        // Initializes the reactive executor the way the viewer's own server does.
        drop(leptos_axum::generate_route_list(|| ()));
        let cdr = replayed_cdr_with(stubs).await;
        let options = LeptosOptions::builder()
            .output_name(env!("CARGO_PKG_NAME"))
            .build();
        Self {
            service: ferroehr_viewer::server::router(app_state(&cdr), options),
        }
    }

    /// Sends one request; `cookie` is the session, when there is one.
    async fn send(
        &self,
        method: http::Method,
        path: &str,
        cookie: Option<&str>,
        form: Option<&[(&str, &str)]>,
    ) -> Answer {
        let mut request = http::Request::builder()
            .method(method)
            .uri(path)
            .header(http::header::ACCEPT, "text/html")
            .header(http::header::HOST, "viewer.test");
        if let Some(cookie) = cookie {
            request = request.header(http::header::COOKIE, cookie);
        }
        let body = match form {
            Some(fields) => {
                request = request
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .header(http::header::ORIGIN, "http://viewer.test");
                fields
                    .iter()
                    .map(|(name, value)| {
                        format!(
                            "{}={}",
                            urlencoding::encode(name),
                            urlencoding::encode(value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("&")
            }
            None => String::new(),
        };
        let response = self
            .service
            .clone()
            .oneshot(
                request
                    .body(axum::body::Body::from(body))
                    .expect("the request fixture builds"),
            )
            .await
            .expect("the service is infallible");
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, 16 << 20)
            .await
            .expect("a page fits in memory");
        Answer {
            status: parts.status,
            headers: parts.headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// Signs in through the form and returns the session cookie.
    async fn sign_in(&self) -> String {
        let answer = self
            .send(
                http::Method::POST,
                "/login",
                None,
                Some(&[
                    ("username", "ferroehr"),
                    ("password", "ferroehr"),
                    ("next", "/"),
                ]),
            )
            .await;
        assert_eq!(answer.status, http::StatusCode::FOUND, "{}", answer.body);
        answer
            .cookies()
            .into_iter()
            .find(|cookie| cookie.starts_with(ferroehr_viewer::session::SESSION_COOKIE))
            .expect("an accepted sign-in sets the session cookie")
    }

    /// A page, signed in as `cookie`.
    async fn page(&self, cookie: &str, path: &str) -> Answer {
        self.send(http::Method::GET, path, Some(cookie), None).await
    }
}

#[tokio::test]
async fn the_sign_in_page_is_served_without_a_session_and_posts_its_own_fields() {
    let viewer = Viewer::start().await;
    let page = viewer.send(http::Method::GET, "/login", None, None).await;
    assert_eq!(page.status, http::StatusCode::OK);
    for field in [r#"name="username""#, r#"name="password""#, r#"name="next""#] {
        assert!(page.body.contains(field), "{field} in {}", page.body);
    }
    assert!(page.body.contains(r#"action="/login""#), "{}", page.body);
}

#[tokio::test]
async fn a_wrong_password_draws_the_refusal_and_sets_no_session() {
    let viewer = Viewer::start().await;
    let answer = viewer
        .send(
            http::Method::POST,
            "/login",
            None,
            Some(&[
                ("username", "ferroehr"),
                ("password", "wrong"),
                ("next", "/ehrs"),
            ]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert!(
        answer
            .body
            .contains("the CDR refused that username or password"),
        "{}",
        answer.body
    );
    assert!(
        answer
            .cookies()
            .iter()
            .all(|cookie| !cookie.starts_with(ferroehr_viewer::session::SESSION_COOKIE)),
        "a refused sign-in sets no session"
    );
    assert!(
        answer.body.contains(r#"value="ferroehr""#),
        "the username is kept"
    );
}

#[tokio::test]
async fn an_accepted_sign_in_lands_on_a_same_origin_destination_only() {
    let viewer = Viewer::start().await;
    let kept = viewer
        .send(
            http::Method::POST,
            "/login",
            None,
            Some(&[
                ("username", "ferroehr"),
                ("password", "ferroehr"),
                ("next", "/ehrs?x=1"),
            ]),
        )
        .await;
    assert_eq!(kept.location(), "/ehrs?x=1");
    let dropped = viewer
        .send(
            http::Method::POST,
            "/login",
            None,
            Some(&[
                ("username", "ferroehr"),
                ("password", "ferroehr"),
                ("next", "https://evil.example/"),
            ]),
        )
        .await;
    assert_eq!(dropped.location(), "/");
}

#[tokio::test]
async fn a_guarded_page_without_a_session_redirects_to_the_sign_in_keeping_its_address() {
    let viewer = Viewer::start().await;
    let answer = viewer.send(http::Method::GET, "/ehrs", None, None).await;
    assert!(answer.status.is_redirection(), "{}", answer.status);
    assert_eq!(answer.location(), "/login?next=%2Fehrs");
}

#[tokio::test]
async fn a_form_of_another_origin_is_refused() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("/query/aql")
        .header(http::header::HOST, "viewer.test")
        .header(http::header::ORIGIN, "https://evil.example")
        .header(http::header::COOKIE, &cookie)
        .header(
            http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(axum::body::Body::from("intent=run&aql=SELECT+1"))
        .expect("the request fixture builds");
    let response = viewer
        .service
        .clone()
        .oneshot(request)
        .await
        .expect("the service is infallible");
    assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_shell_carries_the_lockup_the_sidebar_the_user_menu_and_the_footer() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/").await;
    assert_eq!(page.status, http::StatusCode::OK);
    let body = &page.body;
    assert!(
        body.contains("<title>Dashboard · FerroEHR Viewer</title>"),
        "{body}"
    );
    for link in ["/ferrohealth-ui.css", "/theme.css", "/favicon.svg"] {
        assert!(body.contains(link), "{link} in the head");
    }
    // The sidebar names every screen group, in order.
    let mut at = 0;
    for entry in [
        "Dashboard",
        "EHRs",
        "Parties",
        "Templates",
        "Terminology",
        "Stored queries",
        "Query",
        "Server",
        "Audit log",
        "Configuration",
        "About",
    ] {
        let found = body
            .get(at..)
            .and_then(|rest| rest.find(&format!(">{entry}<")))
            .map(|i| i + at);
        assert!(found.is_some(), "{entry} after the entries before it");
        at = found.unwrap_or(at);
    }
    // The user menu holds Your preferences, the theme and Sign out; the top
    // bar has no theme toggle of its own (FerroUI 0.3.0).
    assert!(body.contains(r#"href="/preferences""#), "{body}");
    assert!(body.contains(r#"action="/logout""#), "{body}");
    assert!(
        !body.contains("fui-theme-toggle"),
        "no theme toggle in the top bar"
    );
    // The footer names the viewer, its version and the manufacturer.
    assert!(
        body.contains(env!("CARGO_PKG_VERSION")),
        "the version in the footer"
    );
    assert!(
        body.contains("Cadasto B.V."),
        "the manufacturer in the footer"
    );
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one table of every screen and the words it must say"
)]
async fn every_screen_draws_its_heading_and_its_reading() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let composition = id("composition");
    let party = id("party");
    let relationship = id("relationship");
    let screens: Vec<(String, Vec<&str>)> = vec![
        ("/".to_owned(), vec!["Dashboard", "Compositions per day"]),
        (
            "/ehrs".to_owned(),
            vec!["Every EHR the CDR holds, newest first."],
        ),
        (
            format!("/ehrs/{ehr}"),
            vec!["New composition", "Minimal", "Compositions"],
        ),
        (
            format!("/ehrs/{ehr}/status"),
            vec!["This version", "EHR_STATUS, version 1"],
        ),
        (
            format!("/ehrs/{ehr}/directory"),
            vec!["Episodes", "Versions"],
        ),
        (
            format!("/ehrs/{ehr}/contributions"),
            vec!["Contributions per day, last 30 days"],
        ),
        (
            format!("/ehrs/{ehr}/tags"),
            vec!["reviewed", "Tags in this EHR"],
        ),
        (
            format!("/ehrs/{ehr}/compositions/{composition}"),
            vec!["Edit as a new version", "minimal_evaluation.en.v1"],
        ),
        (
            format!("/ehrs/{ehr}/compositions/{composition}?tab=versions"),
            vec!["The versioned composition", "Version 2"],
        ),
        (
            format!("/ehrs/{ehr}/compositions/{composition}?tab=tags"),
            vec!["Tags of the composition", "reviewed"],
        ),
        (
            "/parties".to_owned(),
            vec!["The demographic tag index", "follow-up"],
        ),
        (
            format!("/parties/person/{party}"),
            vec!["Demo Person 0417", "Identities"],
        ),
        (
            format!("/parties/relationships/{relationship}"),
            vec!["employment", "Organisation"],
        ),
        (
            "/templates".to_owned(),
            vec!["minimal_evaluation.en.v1", "Root archetype"],
        ),
        (
            "/templates/minimal_evaluation.en.v1".to_owned(),
            vec!["Minimal evaluation", "Identity"],
        ),
        ("/queries".to_owned(), vec!["org.example.cardio", "latest"]),
        (
            "/queries/org.example.cardio::latest/1.0.0".to_owned(),
            vec!["org.example.cardio::latest · version 1.0.0", "Definition"],
        ),
        (
            "/query/aql".to_owned(),
            vec!["Ways to write the query", "Parameters"],
        ),
        (
            "/server".to_owned(),
            vec!["How the CDR was built and deployed.", "Sandbox"],
        ),
        (
            "/about".to_owned(),
            vec!["ferrohealth-ui 0.3.6", "The conformance manifest"],
        ),
        ("/access".to_owned(), vec!["Signed in as", "ferroehr"]),
        (
            "/preferences".to_owned(),
            vec!["Only for you, in this browser.", "Theme"],
        ),
        (
            "/configuration".to_owned(),
            vec!["deployment_profile", "sandbox"],
        ),
    ];
    for (path, expected) in screens {
        let page = viewer.page(&cookie, &path).await;
        assert_eq!(page.status, http::StatusCode::OK, "{path}: {}", page.body);
        for words in expected {
            assert!(
                page.main().contains(words),
                "{path} says {words:?}:\n{}",
                page.main()
            );
        }
    }
}

/// An editor that decides its decision bar only after its reads still docks
/// it: the shell waits for the page's promise.
#[tokio::test]
async fn an_editor_docks_its_decision_bar_after_its_reads() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let composition = id("composition");
    for path in [
        "/ehrs/new".to_owned(),
        format!("/ehrs/{ehr}/status/edit"),
        format!("/ehrs/{ehr}/compositions/{composition}/edit"),
    ] {
        let page = viewer.page(&cookie, &path).await;
        assert_eq!(page.status, http::StatusCode::OK, "{path}: {}", page.body);
        assert!(
            page.body.contains("id=\"editor-dock\""),
            "{path} docks its decision bar:\n{}",
            page.main()
        );
    }
}

/// The shell's user menu lists Your access, and the sandbox band's Details
/// leads to Server's Overview from every page but that one.
#[tokio::test]
async fn the_shell_links_your_access_and_the_sandbox_details() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let details = r#"<a href="/server">Details</a>"#;
    let dashboard = viewer.page(&cookie, "/").await;
    assert_eq!(dashboard.status, http::StatusCode::OK);
    let menu = dashboard
        .body
        .find(r#"id="user-menu-menu""#)
        .and_then(|start| dashboard.body.get(start..))
        .unwrap_or_default();
    let menu = menu
        .get(..menu.find("</div>").unwrap_or(menu.len()))
        .unwrap_or_default();
    let preferences = menu
        .find(r#"href="/preferences""#)
        .expect("Your preferences");
    let access = menu
        .find(r#"href="/access""#)
        .expect("Your access in the menu");
    assert!(
        preferences < access,
        "Your access follows Your preferences: {menu}"
    );
    assert!(dashboard.body.contains(details), "{}", dashboard.body);
    let server = viewer.page(&cookie, "/server").await;
    assert_eq!(server.status, http::StatusCode::OK);
    assert!(server.body.contains("Sandbox"), "the band is drawn");
    assert!(!server.body.contains(details), "{}", server.body);
}

/// An EHR's More menu copies its id, and an editor's Discard leaves without
/// the form's checks.
#[tokio::test]
async fn an_ehr_menu_copies_its_id_and_discard_skips_the_checks() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let page = viewer.page(&cookie, &format!("/ehrs/{ehr}")).await;
    assert_eq!(page.status, http::StatusCode::OK);
    assert!(
        page.body.contains(&format!(r#"data-fui-copy="{ehr}""#)),
        "{}",
        page.main()
    );
    let new = viewer.page(&cookie, "/ehrs/new").await;
    assert_eq!(new.status, http::StatusCode::OK);
    let discard = new
        .body
        .find(r#"value="discard""#)
        .and_then(|at| {
            let start = new.body.get(..at)?.rfind("<button")?;
            let end = at + new.body.get(at..)?.find('>')?;
            new.body.get(start..end)
        })
        .expect("Discard");
    assert!(discard.contains("formnovalidate"), "{discard}");
    let save = new
        .body
        .find(r#"value="save""#)
        .and_then(|at| {
            let start = new.body.get(..at)?.rfind("<button")?;
            let end = at + new.body.get(at..)?.find('>')?;
            new.body.get(start..end)
        })
        .expect("Save");
    assert!(!save.contains("formnovalidate"), "{save}");
}

/// A template's More menu offers its Download, which answers the stored
/// operational template as a file, and only to a signed-in reader.
#[tokio::test]
async fn a_template_downloads_as_its_stored_source() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let href = "/download/template/adl14/minimal_evaluation.en.v1";
    let page = viewer
        .page(&cookie, "/templates/minimal_evaluation.en.v1")
        .await;
    assert_eq!(page.status, http::StatusCode::OK);
    assert!(
        page.body.contains(&format!(r#"href="{href}""#)),
        "{}",
        page.main()
    );
    let file = viewer.page(&cookie, href).await;
    assert_eq!(file.status, http::StatusCode::OK, "{}", file.body);
    assert_eq!(
        file.headers
            .get(http::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok()),
        Some(r#"attachment; filename="minimal_evaluation.en.v1.opt""#)
    );
    assert!(
        file.body.contains("minimal_evaluation.en.v1"),
        "{}",
        file.body
    );
    for crafted in [
        "/download/template/adl14/..%2F..%2Fehr",
        "/download/template/adl14/..",
        "/download/template/adl14/a%5Cb",
        "/download/template/..%2Fadl1.4/minimal_evaluation.en.v1",
    ] {
        let refused = viewer.page(&cookie, crafted).await;
        assert_eq!(
            refused.status,
            http::StatusCode::NOT_FOUND,
            "{crafted} names no template"
        );
    }
    let anonymous = viewer.send(http::Method::GET, href, None, None).await;
    assert!(
        anonymous.status.is_redirection(),
        "no session, no file: {}",
        anonymous.status
    );
}

#[tokio::test]
async fn an_ehr_shows_its_subject_masked_until_the_reader_asks() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let masked = viewer.page(&cookie, &format!("/ehrs/{ehr}")).await;
    assert!(
        !masked.main().contains("p-0417"),
        "the subject id is masked"
    );
    assert!(masked.main().contains("417"), "its tail is shown");
    let revealed = viewer.page(&cookie, &format!("/ehrs/{ehr}?reveal=1")).await;
    assert!(revealed.main().contains("p-0417"), "{}", revealed.main());
}

#[tokio::test]
async fn a_run_draws_the_rows_and_the_exports_post_the_query_again() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let aql =
        "SELECT e/ehr_id/value AS ehr, c/name/value AS name FROM EHR e CONTAINS COMPOSITION c";
    let answer = viewer
        .send(
            http::Method::POST,
            "/query/aql",
            Some(&cookie),
            Some(&[("default-intent", "run"), ("aql", aql)]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK);
    let main = answer.main();
    assert!(main.contains(&id("ehr")), "{main}");
    let csv = main
        .find(r#"type="submit" formaction="/export/aql/csv" role="menuitem""#)
        .unwrap_or_else(|| panic!("Export's CSV item submits the run's form: {main}"));
    let json = main
        .find(r#"type="submit" formaction="/export/aql/json" role="menuitem""#)
        .unwrap_or_else(|| panic!("Export's JSON item submits the run's form: {main}"));
    assert!(csv < json, "Export lists CSV before JSON: {main}");
    assert!(
        main.contains(r#"aria-haspopup="menu" aria-controls="results-export-"#),
        "the exports sit behind one Export menu: {main}"
    );
    assert!(
        main.contains(r#"name="default-shown" value="table""#),
        "the next Run keeps the view: {main}"
    );
    assert!(
        !main.contains(r#"name="shown""#),
        "a result with no numeric column offers no chart: {main}"
    );
}

#[tokio::test]
async fn find_answers_an_ehr_id_with_the_ehr() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let answer = viewer
        .send(
            http::Method::POST,
            "/find",
            Some(&cookie),
            Some(&[("q", &ehr)]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert!(
        answer.main().contains(&format!("/ehrs/{ehr}")),
        "{}",
        answer.main()
    );
}

#[tokio::test]
async fn a_preference_is_kept_in_its_own_cookie() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let answer = viewer
        .send(
            http::Method::POST,
            "/preference",
            Some(&cookie),
            Some(&[("theme", "dark"), ("back", "/preferences")]),
        )
        .await;
    assert!(answer.status.is_redirection(), "{}", answer.body);
    assert!(
        answer
            .cookies()
            .iter()
            .any(|cookie| cookie == "ferroehr-viewer-theme=dark"),
        "{:?}",
        answer.cookies()
    );
    let page = viewer
        .page(&format!("{cookie}; ferroehr-viewer-theme=dark"), "/")
        .await;
    assert!(
        page.body.contains(r#"data-theme="dark""#),
        "the page is drawn dark"
    );
}

#[tokio::test]
async fn sign_out_ends_the_session_and_says_so() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let answer = viewer
        .send(http::Method::POST, "/logout", Some(&cookie), Some(&[]))
        .await;
    assert!(answer.status.is_redirection(), "{}", answer.status);
    assert!(
        answer.location().starts_with("/login?signed-out=1"),
        "{}",
        answer.location()
    );
    assert!(
        answer
            .cookies()
            .iter()
            .any(|cookie| cookie.starts_with(ferroehr_viewer::session::SESSION_COOKIE)),
        "the session cookie is cleared"
    );
}

#[tokio::test]
async fn an_unknown_address_answers_404_with_the_way_back() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/no-such-screen").await;
    assert_eq!(page.status, http::StatusCode::NOT_FOUND);
    assert!(page.body.contains("No such screen"), "{}", page.body);
}

/// A stubbed CDR exchange: a `GET` of `path` answered `200` with `content_type`.
fn stub(path: &str, content_type: &str, response: &str) -> Value {
    serde_json::json!({
        "method": "GET",
        "path": path,
        "body": "",
        "status": 200,
        "headers": { "content-type": content_type },
        "response": response,
    })
}

/// The format options an Example tab's selector offers.
fn example_format_options(main: &str) -> Vec<&str> {
    let Some(start) = main.find("name=\"format\"") else {
        return Vec::new();
    };
    let select = main.get(start..).unwrap_or_default();
    let select = select
        .get(..select.find("</select>").unwrap_or(select.len()))
        .unwrap_or_default();
    select
        .match_indices("<option value=\"")
        .filter_map(|(at, open)| {
            let value = select.get(at + open.len()..)?;
            value.get(..value.find('"')?)
        })
        .collect()
}

/// A COMPOSITION template's Example tab offers the four representations of
/// an example COMPOSITION.
#[tokio::test]
async fn a_composition_template_offers_every_example_format() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, "/templates/minimal_evaluation.en.v1?tab=example")
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert_eq!(
        example_format_options(page.main()),
        ["json", "xml", "flat", "structured"],
        "{}",
        page.main()
    );
}

/// A demographic template's example is a party, which comes in canonical JSON
/// and XML only (the Simplified Formats RM mapping maps no party class), so the
/// Example tab offers no FLAT or STRUCTURED and draws the party (#3995).
#[tokio::test]
async fn a_demographic_template_offers_only_the_canonical_example_formats() {
    let base = "/ferroehr/rest/openehr/v1/definition/template/adl1.4/Role.v2";
    let opt = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/fixtures/opt/valid/role.v2.opt"),
    )
    .expect("the Role.v2 fixture should be readable");
    let example = r#"{"_type":"ROLE","name":{"_type":"DV_TEXT","value":"Rol"},"archetype_node_id":"openEHR-DEMOGRAPHIC-ROLE.Role.v1","identities":[],"performer":{"_type":"PARTY_REF","namespace":"demographic","type":"PERSON","id":{"_type":"HIER_OBJECT_ID","value":"example-performer"}}}"#;
    let viewer = Viewer::start_with(vec![
        stub(base, "application/xml", &opt),
        stub(
            &format!("{base}/example?detail_level=required&type=input"),
            "application/json",
            example,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/templates/Role.v2?tab=example").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert_eq!(
        example_format_options(page.main()),
        ["json", "xml"],
        "{}",
        page.main()
    );
    assert!(page.main().contains("example-performer"), "{}", page.main());

    let paths = viewer.page(&cookie, "/templates/Role.v2?tab=paths").await;
    assert_eq!(paths.status, http::StatusCode::OK, "{}", paths.body);
    // The CAPABILITY node (at0003) is named by its term, "Geschiktheid".
    assert!(paths.main().contains("Geschiktheid"), "{}", paths.main());
}
