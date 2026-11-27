// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART launch Tasks of an EHR's patient (#5930), through the viewer's
//! real service over the replayed CDR, signed in through a real OpenID
//! Connect sign-in at an in-process stub of FerroSMART.
//!
//! The stub is the OpenID Provider (its key set and its token endpoint,
//! answering the code exchange with an ES256 ID Token and the refresh grant
//! that names `resource`, RFC 8707 §2.2), the EHR launch API its discovery
//! document names as `launch_registration_endpoint`, and the FHIR facade's
//! Task search and read, which replay the SMART App Launch 2.2.0 examples
//! (`docs/specs/smart-app-launch/package/example`).

#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction: a loopback listener, a test key, a \
              static request; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the Task examples are replayed as raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::ssr_app::{Answer, CDR_BEARER, Viewer, id};

/// The viewer's client id at the OpenID Provider.
const SIGN_IN_CLIENT: &str = "ferroehr-viewer-sign-in";

/// The refresh token of the sign-in.
const REFRESH: &str = "synthetic-refresh-1";

/// The access token the provider issues for the facade.
const FACADE_BEARER: &str = "synthetic-facade-access";

/// The launch value the stub hands out.
const LAUNCH_VALUE: &str = "Zm9vYmFyLWxhdW5jaC12YWx1ZS1mb3ItdGhlLXRlc3Qtb25seQ";

/// The launch URL of the configured app an EHR launch Task names.
const GROWTH: &str = "https://apps.example.org/growth/launch?theme=light";

/// The launch URL of the configured app a standalone launch Task names.
const MEDS: &str = "https://apps.example.org/meds/launch";

/// The FHIR `Patient` id of the fixture EHR's subject.
const PATIENT: &str = "sandbox-patient-01";

/// A form as the stub received it.
type Form = Vec<(String, String)>;

/// What the stub received.
#[derive(Default)]
struct Seen {
    /// The `nonce` of the sign-in in flight.
    nonce: Option<String>,
    /// Every token request, in order.
    tokens: Vec<Form>,
    /// Every launch registration, in order.
    launches: Vec<Form>,
    /// Every facade request: its path and query, and its `Authorization`.
    facade: Vec<(String, String)>,
}

/// The stub FerroSMART and its facade.
struct Stub {
    /// Its issuer, `http://127.0.0.1:<port>`.
    issuer: String,
    /// What it received.
    seen: Arc<Mutex<Seen>>,
}

impl Stub {
    /// The facade's FHIR base.
    fn fhir_base(&self) -> String {
        format!("{}/fhir", self.issuer)
    }

    /// Every launch registration it received.
    fn launches(&self) -> Vec<Form> {
        self.seen
            .lock()
            .expect("the stub's record")
            .launches
            .clone()
    }
}

/// The SMART App Launch example `name`, as JSON.
fn example(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/specs/smart-app-launch/package/example")
        .join(name);
    let text = std::fs::read_to_string(path).expect("the example reads");
    serde_json::from_str(&text).expect("the example is JSON")
}

/// The example `name` with the id `id`, the app URL `app` and, when given,
/// `for` naming `patient`.
fn task(name: &str, id: &str, app: &str, patient: Option<&str>) -> Value {
    let mut task = example(name);
    let fields = task.as_object_mut().expect("the example is an object");
    fields.insert("id".to_owned(), Value::from(id));
    if let Some(patient) = patient {
        fields.insert(
            "for".to_owned(),
            serde_json::json!({ "reference": format!("Patient/{patient}") }),
        );
    }
    let inputs = fields
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .expect("the example has inputs");
    for input in inputs {
        let application =
            input.pointer("/type/coding/0/code") == Some(&Value::from("smartonfhir-application"));
        if let (true, Some(input)) = (application, input.as_object_mut()) {
            input.insert("valueUrl".to_owned(), Value::from(app));
        }
    }
    task
}

/// The Tasks the facade holds: an EHR launch and a standalone launch of the
/// fixture patient for configured apps, one for an app the viewer does not
/// launch, and the unmodified example's patient for a configured app.
fn tasks() -> Vec<Value> {
    vec![
        task(
            "Task-task-for-ehr-launch.json",
            "task-for-ehr-launch",
            GROWTH,
            Some(PATIENT),
        ),
        task(
            "Task-task-for-standalone-launch.json",
            "task-for-standalone-launch",
            MEDS,
            Some(PATIENT),
        ),
        task(
            "Task-task-for-ehr-launch.json",
            "task-unconfigured",
            "https://www.example.org/myapplication",
            Some(PATIENT),
        ),
        task(
            "Task-task-for-ehr-launch.json",
            "task-other-patient",
            GROWTH,
            None,
        ),
    ]
}

/// The provider's signing key: its private half and its JWK Set.
fn signing_key() -> (jsonwebtoken::EncodingKey, Value) {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("a P-256 key");
    let encoding = jsonwebtoken::EncodingKey::from_ec_der(&pair.serialize_der());
    let mut jwk = serde_json::to_value(
        jsonwebtoken::jwk::Jwk::from_encoding_key(&encoding, jsonwebtoken::Algorithm::ES256)
            .expect("a JWK"),
    )
    .expect("the JWK serializes");
    let members = jwk.as_object_mut().expect("a JWK is an object");
    members.insert("kid".to_owned(), Value::from("op-1"));
    members.insert("use".to_owned(), Value::from("sig"));
    (encoding, serde_json::json!({ "keys": [jwk] }))
}

/// An ID Token for the viewer from `issuer`, carrying `nonce`.
fn id_token(encoding: &jsonwebtoken::EncodingKey, issuer: &str, nonce: &str) -> String {
    let now = jiff::Timestamp::now().as_second();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some("op-1".to_owned());
    jsonwebtoken::encode(
        &header,
        &serde_json::json!({
            "iss": issuer,
            "aud": SIGN_IN_CLIENT,
            "sub": "synthetic-clinician",
            "nonce": nonce,
            "iat": now,
            "exp": now + 300,
            "name": "Synthetic Clinician"
        }),
        encoding,
    )
    .expect("the ID Token signs")
}

/// A JSON answer.
fn json(status: http::StatusCode, body: &Value) -> axum::response::Response {
    axum::response::IntoResponse::into_response((
        status,
        [(http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    ))
}

/// The `Authorization` of `headers`, or an empty string.
fn authorization(headers: &http::HeaderMap) -> String {
    headers
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The value of `name` in `form`.
fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The token endpoint: the code exchange answers the CDR token, the
/// sign-in's ID Token and the refresh token, and a refresh grant naming
/// `fhir_base` answers the facade token.
fn token_answer(
    form: &[(String, String)],
    encoding: &jsonwebtoken::EncodingKey,
    issuer: &str,
    nonce: Option<&str>,
    fhir_base: &str,
) -> axum::response::Response {
    match field(form, "grant_type") {
        Some("authorization_code") if field(form, "code") == Some("synthetic-code") => json(
            http::StatusCode::OK,
            &serde_json::json!({
                "access_token": CDR_BEARER,
                "token_type": "Bearer",
                "expires_in": 300,
                "refresh_token": REFRESH,
                "id_token": id_token(encoding, issuer, nonce.unwrap_or_default())
            }),
        ),
        Some("refresh_token")
            if field(form, "refresh_token") == Some(REFRESH)
                && field(form, "resource") == Some(fhir_base) =>
        {
            json(
                http::StatusCode::OK,
                &serde_json::json!({
                    "access_token": FACADE_BEARER,
                    "token_type": "Bearer",
                    "expires_in": 300,
                    "refresh_token": REFRESH
                }),
            )
        }
        _ => json(
            http::StatusCode::BAD_REQUEST,
            &serde_json::json!({ "error": "invalid_grant" }),
        ),
    }
}

/// The stub's OpenID Provider and EHR launch API at `issuer`, recording into
/// `seen`.
fn provider(issuer: &str, seen: &Arc<Mutex<Seen>>) -> axum::Router {
    let (encoding, jwks) = signing_key();
    let discovery = serde_json::json!({
        "issuer": issuer,
        "launch_registration_endpoint": format!("{issuer}/launch")
    });
    let on_token = {
        let seen = Arc::clone(seen);
        let issuer = issuer.to_owned();
        let fhir_base = format!("{issuer}/fhir");
        move |form: axum::extract::Form<Form>| {
            let mut seen = seen.lock().expect("the stub's record");
            seen.tokens.push(form.0.clone());
            let answer = token_answer(
                &form.0,
                &encoding,
                &issuer,
                seen.nonce.as_deref(),
                &fhir_base,
            );
            async move { answer }
        }
    };
    let on_launch = {
        let seen = Arc::clone(seen);
        move |form: axum::extract::Form<Form>| {
            seen.lock()
                .expect("the stub's record")
                .launches
                .push(form.0);
            async move {
                json(
                    http::StatusCode::CREATED,
                    &serde_json::json!({ "launch": LAUNCH_VALUE, "expires_in": 300 }),
                )
            }
        }
    };
    axum::Router::new()
        .route(
            "/jwks",
            axum::routing::get(move || {
                let jwks = jwks.clone();
                async move { json(http::StatusCode::OK, &jwks) }
            }),
        )
        .route("/token", axum::routing::post(on_token))
        .route(
            "/.well-known/smart-configuration",
            axum::routing::get(move || {
                let discovery = discovery.clone();
                async move { json(http::StatusCode::OK, &discovery) }
            }),
        )
        .route("/launch", axum::routing::post(on_launch))
}

/// The stub's FHIR facade, holding [`tasks`] and recording into `seen`.
fn facade(seen: &Arc<Mutex<Seen>>) -> axum::Router {
    let held = Arc::new(tasks());
    let on_search = {
        let seen = Arc::clone(seen);
        let held = Arc::clone(&held);
        move |uri: http::Uri, headers: http::HeaderMap| {
            seen.lock()
                .expect("the stub's record")
                .facade
                .push((uri.to_string(), authorization(&headers)));
            let entry: Vec<Value> = held
                .iter()
                .map(|task| serde_json::json!({ "resource": task, "search": { "mode": "match" } }))
                .collect();
            async move {
                json(
                    http::StatusCode::OK,
                    &serde_json::json!({ "resourceType": "Bundle", "type": "searchset", "entry": entry }),
                )
            }
        }
    };
    let on_read = {
        let seen = Arc::clone(seen);
        move |axum::extract::Path(task_id): axum::extract::Path<String>,
              uri: http::Uri,
              headers: http::HeaderMap| {
            seen.lock()
                .expect("the stub's record")
                .facade
                .push((uri.to_string(), authorization(&headers)));
            let found = held
                .iter()
                .find(|task| task.get("id") == Some(&Value::from(task_id.as_str())))
                .cloned();
            async move {
                match found {
                    Some(task) => json(http::StatusCode::OK, &task),
                    None => json(
                        http::StatusCode::NOT_FOUND,
                        &serde_json::json!({ "resourceType": "OperationOutcome" }),
                    ),
                }
            }
        }
    };
    axum::Router::new()
        .route("/fhir/Task", axum::routing::get(on_search))
        .route("/fhir/Task/{id}", axum::routing::get(on_read))
}

/// Starts the stub FerroSMART with its facade on a loopback port.
async fn ferrosmart() -> Stub {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let issuer = format!(
        "http://{}",
        listener.local_addr().expect("the bound address")
    );
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let app = provider(&issuer, &seen).merge(facade(&seen));
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the stub serves until the test ends");
    });
    Stub { issuer, seen }
}

/// A P-384 key for the viewer's ES384 client assertion, in a temporary file.
fn assertion_key() -> crate::launch::tempfile_path::Path {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("a P-384 key");
    crate::launch::tempfile_path::Path::new(&pair.serialize_pem())
}

/// `[oidc]` at the stub, every endpoint given, asking for a refresh token.
fn oidc(stub: &Stub) -> ferrohealth_session::config::OidcSection {
    let url = |path: &str| {
        Some(ferrohealth_secret::secret_url::SecretUrl::new(format!(
            "{}{path}",
            stub.issuer
        )))
    };
    ferrohealth_session::config::OidcSection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(stub.issuer.clone()),
        authorization_endpoint: url("/authorize"),
        token_endpoint: url("/token"),
        jwks_uri: url("/jwks"),
        client_id: SIGN_IN_CLIENT.to_owned(),
        client_secret: Some(ferrohealth_secret::secret::Secret::new(
            "synthetic-client-secret",
        )),
        redirect_uri: ferrohealth_secret::secret_url::SecretUrl::new(
            "https://viewer.test/auth/callback",
        ),
        scopes: vec!["openid".to_owned(), "profile".to_owned()],
        offline_access: true,
        ..ferroehr_viewer::config::ViewerConfig::default().oidc
    }
}

/// `[launch]` at the stub, listing Tasks at its facade and launching the
/// two configured apps.
fn launch_section(stub: &Stub, key: &std::path::Path) -> ferroehr_viewer::launch::LaunchSection {
    let app = |name: &str, client_id: &str, launch_url: &str| ferroehr_viewer::launch::LaunchApp {
        name: name.to_owned(),
        client_id: client_id.to_owned(),
        launch_url: launch_url.to_owned(),
    };
    ferroehr_viewer::launch::LaunchSection {
        issuer: stub.issuer.clone(),
        client_id: "ferroehr-viewer".to_owned(),
        private_key_file: Some(key.to_path_buf()),
        kid: "viewer-1".to_owned(),
        resource: "https://ehr.example.org/ferroehr/rest/openehr/v1".to_owned(),
        patient_namespaces: vec!["ferroehr-sandbox".to_owned()],
        fhir_base: stub.fhir_base(),
        app: vec![
            app("Growth chart", "growth-chart", GROWTH),
            app("Medications", "medications", MEDS),
        ],
        ..ferroehr_viewer::launch::LaunchSection::default()
    }
}

/// The cookies `answer` sets that carry a value, as one `Cookie` header.
fn kept_cookies(answer: &Answer) -> String {
    answer
        .cookies()
        .into_iter()
        .filter(|cookie| !cookie.ends_with('='))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The value of `name` in the query of `location`.
fn query_value(location: &str, name: &str) -> String {
    url::Url::parse(location)
        .expect("an absolute Location")
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

/// Signs in through the provider as a browser does, and returns the
/// session's `Cookie` header.
async fn sign_in(viewer: &Viewer, stub: &Stub) -> String {
    let begun = viewer
        .send(http::Method::GET, "/login?return=%2F", None, None)
        .await;
    assert_eq!(begun.status, http::StatusCode::SEE_OTHER, "{}", begun.body);
    let location = begun.location().to_owned();
    assert!(
        location.starts_with(&format!("{}/authorize?", stub.issuer)),
        "{location}"
    );
    stub.seen.lock().expect("the stub's record").nonce = Some(query_value(&location, "nonce"));
    let state = query_value(&location, "state");
    let back = viewer
        .send(
            http::Method::GET,
            &format!("/auth/callback?code=synthetic-code&state={state}"),
            Some(&kept_cookies(&begun)),
            None,
        )
        .await;
    assert_eq!(back.status, http::StatusCode::SEE_OTHER, "{}", back.body);
    let cookie = kept_cookies(&back);
    assert!(!cookie.is_empty(), "the sign-in sets the session cookie");
    cookie
}

/// A viewer signed in at a fresh stub, with the stub and the session.
async fn signed_in() -> (Viewer, Stub, String, crate::launch::tempfile_path::Path) {
    let stub = ferrosmart().await;
    let key = assertion_key();
    let viewer = Viewer::start_signing_in(oidc(&stub), launch_section(&stub, &key.0)).await;
    let cookie = sign_in(&viewer, &stub).await;
    (viewer, stub, cookie, key)
}

/// Whether the facade read the Task `task` with the facade token.
fn read_at_facade(stub: &Stub, task: &str) -> bool {
    let target = format!("/fhir/Task/{task}");
    let bearer = format!("Bearer {FACADE_BEARER}");
    stub.seen
        .lock()
        .expect("the stub's record")
        .facade
        .iter()
        .any(|(seen, sent)| *seen == target && *sent == bearer)
}

/// Posts the launch of the Task `task` from the fixture EHR's page.
async fn launch(viewer: &Viewer, cookie: &str, task: &str) -> Answer {
    viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{}", id("ehr")),
            Some(cookie),
            Some(&[("intent", "launch-task"), ("task", task)]),
        )
        .await
}

#[tokio::test]
async fn an_ehr_page_lists_the_patients_launch_tasks_read_with_the_facade_token() {
    let (viewer, stub, cookie, _key) = signed_in().await;
    let ehr = id("ehr");
    let page = viewer.page(&cookie, &format!("/ehrs/{ehr}")).await;
    assert_eq!(page.status, http::StatusCode::OK);
    let main = page.main();
    assert!(main.contains("Launch tasks"), "{main}");
    assert!(
        main.contains(r#"<input type="hidden" name="task" value="task-for-ehr-launch">"#),
        "{main}"
    );
    assert!(
        main.contains(r#"<input type="hidden" name="task" value="task-for-standalone-launch">"#),
        "{main}"
    );
    assert!(main.contains("Launch Growth chart"), "{main}");
    assert!(main.contains("Launch Medications"), "{main}");
    assert!(
        !main.contains(r#"value="task-unconfigured""#),
        "a Task of an app the viewer does not launch gets no button: {main}"
    );
    assert!(main.contains("App not configured"), "{main}");
    assert!(
        main.contains(&format!(
            r#"<form method="post" action="/ehrs/{ehr}" target="_blank" rel="noopener"><input type="hidden" name="intent" value="launch-task">"#
        )),
        "{main}"
    );

    let seen = stub.seen.lock().expect("the stub's record");
    let [(search, bearer)] = seen.facade.as_slice() else {
        panic!("one facade search, got {:?}", seen.facade);
    };
    assert_eq!(bearer, &format!("Bearer {FACADE_BEARER}"));
    let search = url::Url::parse(&format!("http://facade{search}")).expect("a request target");
    assert_eq!(search.path(), "/fhir/Task");
    let query: Vec<(String, String)> = search.query_pairs().into_owned().collect();
    assert_eq!(
        query,
        vec![
            ("patient".to_owned(), PATIENT.to_owned()),
            (
                "code".to_owned(),
                "http://hl7.org/fhir/smart-app-launch/CodeSystem/smart-codes|launch-app-ehr,\
                 http://hl7.org/fhir/smart-app-launch/CodeSystem/smart-codes|launch-app-standalone"
                    .to_owned()
            ),
        ]
    );
    let refreshes: Vec<&Form> = seen
        .tokens
        .iter()
        .filter(|form| field(form, "grant_type") == Some("refresh_token"))
        .collect();
    let [refresh] = refreshes.as_slice() else {
        panic!("one refresh grant, got {:?}", seen.tokens);
    };
    assert_eq!(field(refresh, "refresh_token"), Some(REFRESH));
    assert_eq!(
        field(refresh, "resource"),
        Some(stub.fhir_base().as_str()),
        "RFC 8707 §2.2: the refresh names the facade"
    );
    assert!(
        seen.launches.is_empty(),
        "drawing the page registers no launch"
    );
}

#[tokio::test]
async fn an_ehr_launch_task_registers_its_context_and_opens_the_configured_app() {
    let (viewer, stub, cookie, _key) = signed_in().await;
    let answer = launch(&viewer, &cookie, "task-for-ehr-launch").await;
    assert_eq!(
        answer.status,
        http::StatusCode::SEE_OTHER,
        "{}",
        answer.body
    );
    let location = url::Url::parse(answer.location()).expect("an absolute Location");
    assert_eq!(
        location.origin().ascii_serialization(),
        "https://apps.example.org"
    );
    assert_eq!(location.path(), "/growth/launch");
    let query: Vec<(String, String)> = location.query_pairs().into_owned().collect();
    assert_eq!(
        query,
        vec![
            ("theme".to_owned(), "light".to_owned()),
            ("iss".to_owned(), stub.fhir_base()),
            ("launch".to_owned(), LAUNCH_VALUE.to_owned()),
        ]
    );

    let launches = stub.launches();
    let [form] = launches.as_slice() else {
        panic!("one launch registration, got {launches:?}");
    };
    assert_eq!(field(form, "app_launch_uri"), Some(GROWTH));
    assert_eq!(field(form, "resource"), Some(stub.fhir_base().as_str()));
    assert_eq!(field(form, "patient"), Some(PATIENT));
    assert_eq!(field(form, "encounter"), Some("456"));
    assert_eq!(field(form, "appContext"), Some(r#"{"field1":"value"}"#));
    assert_eq!(field(form, "need_patient_banner"), Some("false"));
    assert!(
        read_at_facade(&stub, "task-for-ehr-launch"),
        "the Task is read again at the facade before it launches"
    );
}

#[tokio::test]
async fn a_standalone_launch_task_opens_the_configured_app_with_no_registration() {
    let (viewer, stub, cookie, _key) = signed_in().await;
    let answer = launch(&viewer, &cookie, "task-for-standalone-launch").await;
    assert_eq!(
        answer.status,
        http::StatusCode::SEE_OTHER,
        "{}",
        answer.body
    );
    assert_eq!(answer.location(), MEDS);
    assert_eq!(stub.launches(), Vec::<Form>::new());
}

#[tokio::test]
async fn a_forged_post_naming_a_task_of_an_unconfigured_app_launches_nothing() {
    let (viewer, stub, cookie, _key) = signed_in().await;
    let answer = launch(&viewer, &cookie, "task-unconfigured").await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert_eq!(answer.location(), "");
    assert!(answer.main().contains("The app could not be launched"));
    assert!(!answer.body.contains("www.example.org/myapplication"));
    assert!(read_at_facade(&stub, "task-unconfigured"));
    assert_eq!(stub.launches(), Vec::<Form>::new());
}

#[tokio::test]
async fn a_task_for_another_patient_is_never_launched() {
    let (viewer, stub, cookie, _key) = signed_in().await;
    let answer = launch(&viewer, &cookie, "task-other-patient").await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert_eq!(answer.location(), "");
    assert!(answer.main().contains("The app could not be launched"));
    assert!(read_at_facade(&stub, "task-other-patient"));
    assert_eq!(stub.launches(), Vec::<Form>::new());
}

#[tokio::test]
async fn a_basic_sign_in_holds_no_grant_and_is_asked_to_sign_in_with_ferrosmart() {
    let stub = ferrosmart().await;
    let key = assertion_key();
    let viewer = Viewer::start_signing_in(oidc(&stub), launch_section(&stub, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &format!("/ehrs/{}", id("ehr"))).await;
    assert_eq!(page.status, http::StatusCode::OK);
    assert!(
        page.main()
            .contains("Sign in with FerroSMART to see the launch tasks."),
        "{}",
        page.main()
    );
    let seen = stub.seen.lock().expect("the stub's record");
    assert!(seen.facade.is_empty(), "no facade read without a token");
    assert!(
        seen.tokens.is_empty(),
        "a Basic session asks the provider nothing"
    );
}
