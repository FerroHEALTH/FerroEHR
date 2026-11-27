// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART EHR launch from an EHR's page (#5880), through the viewer's
//! real service over the replayed CDR, against a stub of FerroSMART's EHR
//! launch API that answers as its documentation has it: the
//! `launch_registration_endpoint` its discovery document names, a form carrying the EHR client's `private_key_jwt`
//! assertion, answered `201` with `{"launch", "expires_in": 300}` or an RFC
//! 6749 §5.2 refusal (FerroSMART `docs/architecture.md` §13.6 item 3).

#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction: a loopback listener, a test key, a \
              static request; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]

use std::sync::{Arc, Mutex};

use crate::ssr_app::{Viewer, id};

/// The launch value the stub hands out.
const LAUNCH_VALUE: &str = "Zm9vYmFyLWxhdW5jaC12YWx1ZS1mb3ItdGhlLXRlc3Qtb25seQ";

/// The base the app gets as `iss`.
const RESOURCE: &str = "https://ehr.example.org/ferroehr/rest/openehr/v1";

/// The forms the stub received, in order.
type Forms = Arc<Mutex<Vec<Vec<(String, String)>>>>;

/// The claims of the client assertion the test reads.
#[derive(serde::Deserialize)]
struct Claims {
    /// The subject, the viewer's client id.
    sub: String,
    /// The assertion's id.
    jti: String,
}

/// What the stub FerroSMART received, and how it answers.
struct Stub {
    /// Its issuer, `http://127.0.0.1:<port>`.
    issuer: String,
    /// The form of every launch request, in order.
    received: Forms,
}

/// Starts a FerroSMART stub whose discovery document names its `/launch`
/// as `launch_registration_endpoint`, and whose `/launch` answers `status`
/// with `body`.
async fn ferrosmart(status: http::StatusCode, body: &'static str) -> Stub {
    let received: Forms = Arc::default();
    let seen = Arc::clone(&received);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    let discovery = format!(
        r#"{{"issuer":"http://{addr}","launch_registration_endpoint":"http://{addr}/launch"}}"#
    );
    let app = axum::Router::new()
        .route(
            "/.well-known/smart-configuration",
            axum::routing::get(move || {
                let discovery = discovery.clone();
                async move {
                    (
                        [(http::header::CONTENT_TYPE, "application/json")],
                        discovery,
                    )
                }
            }),
        )
        .route(
            "/launch",
            axum::routing::post(move |form: axum::extract::Form<Vec<(String, String)>>| {
                let seen = Arc::clone(&seen);
                async move {
                    seen.lock().expect("the stub's record").push(form.0);
                    (
                        status,
                        [(http::header::CONTENT_TYPE, "application/json")],
                        body,
                    )
                }
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the stub serves until the test ends");
    });
    Stub {
        issuer: format!("http://{addr}"),
        received,
    }
}

/// A P-384 key for ES384 in a temporary file, and its public half as PEM.
fn key() -> (tempfile_path::Path, String) {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("a P-384 key");
    let path = tempfile_path::Path::new(&pair.serialize_pem());
    (path, pair.public_key_pem())
}

/// A file under the system's temporary directory, removed when dropped.
pub(crate) mod tempfile_path {
    /// The file.
    pub(crate) struct Path(pub(crate) std::path::PathBuf);

    impl Path {
        /// Writes `contents` to a fresh file.
        pub(crate) fn new(contents: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let name = format!("{}-{n}", std::process::id());
            let path = std::env::temp_dir().join(format!("ferroehr-viewer-launch-{name}.pem"));
            std::fs::write(&path, contents).expect("the key file writes");
            Self(path)
        }
    }

    impl Drop for Path {
        fn drop(&mut self) {
            let _removed = std::fs::remove_file(&self.0);
        }
    }
}

/// `[launch]` against `issuer`, signing with the key at `key`, launching
/// one app, its EHR subjects of `ferroehr-sandbox` FHIR Patient ids.
fn section(issuer: &str, key: &std::path::Path) -> ferroehr_viewer::launch::LaunchSection {
    ferroehr_viewer::launch::LaunchSection {
        issuer: issuer.to_owned(),
        client_id: "ferroehr-viewer".to_owned(),
        private_key_file: Some(key.to_path_buf()),
        kid: "viewer-1".to_owned(),
        resource: RESOURCE.to_owned(),
        patient_namespaces: vec!["ferroehr-sandbox".to_owned()],
        app: vec![ferroehr_viewer::launch::LaunchApp {
            name: "Growth chart".to_owned(),
            client_id: "growth-chart".to_owned(),
            launch_url: "https://apps.example.org/growth/launch?theme=light".to_owned(),
        }],
        ..ferroehr_viewer::launch::LaunchSection::default()
    }
}

/// The value of `name` in `form`.
fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn an_ehr_page_offers_each_configured_app_in_a_new_window() {
    let stub = ferrosmart(http::StatusCode::CREATED, "{}").await;
    let (key, _public) = key();
    let viewer = Viewer::start_launching(section(&stub.issuer, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let page = viewer.page(&cookie, &format!("/ehrs/{ehr}")).await;
    assert_eq!(page.status, http::StatusCode::OK);
    assert!(page.body.contains("Launch Growth chart"), "{}", page.body);
    assert!(
        page.body.contains(&format!(
            r#"<form method="post" action="/ehrs/{ehr}" target="_blank" rel="noopener""#
        )),
        "{}",
        page.body
    );
    assert!(
        page.body
            .contains(r#"<input type="hidden" name="app" value="growth-chart">"#),
        "{}",
        page.body
    );
    let policy = page
        .headers
        .get(http::header::CONTENT_SECURITY_POLICY)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        policy.contains("form-action 'self' https://apps.example.org;"),
        "{policy}"
    );
    assert!(
        stub.received.lock().expect("the stub's record").is_empty(),
        "drawing the page registers no launch"
    );
}

#[tokio::test]
async fn a_launch_registers_the_ehr_and_sends_the_new_window_to_the_app() {
    let stub = ferrosmart(
        http::StatusCode::CREATED,
        r#"{"launch":"Zm9vYmFyLWxhdW5jaC12YWx1ZS1mb3ItdGhlLXRlc3Qtb25seQ","expires_in":300}"#,
    )
    .await;
    let (key, public) = key();
    let viewer = Viewer::start_launching(section(&stub.issuer, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{ehr}"),
            Some(&cookie),
            Some(&[("intent", "launch"), ("app", "growth-chart")]),
        )
        .await;
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
            ("iss".to_owned(), RESOURCE.to_owned()),
            ("launch".to_owned(), LAUNCH_VALUE.to_owned()),
        ]
    );
    assert_eq!(
        answer
            .headers
            .get(http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );

    let received = stub.received.lock().expect("the stub's record").clone();
    let [form] = received.as_slice() else {
        panic!("one launch request, got {received:?}");
    };
    assert_eq!(
        field(form, "client_assertion_type"),
        Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
    );
    assert_eq!(field(form, "app_client_id"), Some("growth-chart"));
    assert_eq!(field(form, "resource"), Some(RESOURCE));
    assert_eq!(field(form, "ehrId"), Some(ehr.as_str()));
    assert_eq!(field(form, "patient"), Some("sandbox-patient-01"));
    assert_eq!(field(form, "need_patient_banner"), Some("false"));
    let names: Vec<&str> = form.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "client_assertion_type",
            "client_assertion",
            "app_client_id",
            "resource",
            "ehrId",
            "patient",
            "need_patient_banner"
        ],
        "FerroSMART refuses any other parameter"
    );

    let assertion = field(form, "client_assertion").expect("the assertion");
    let header = jsonwebtoken::decode_header(assertion).expect("a JWS header");
    assert_eq!(header.alg, jsonwebtoken::Algorithm::ES384);
    assert_eq!(header.typ.as_deref(), Some("client-authentication+jwt"));
    assert_eq!(header.kid.as_deref(), Some("viewer-1"));
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES384);
    validation.set_audience(&[stub.issuer.as_str()]);
    validation.set_issuer(&["ferroehr-viewer"]);
    validation.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);
    let claims = jsonwebtoken::decode::<Claims>(
        assertion,
        &jsonwebtoken::DecodingKey::from_ec_pem(public.as_bytes()).expect("the public key"),
        &validation,
    )
    .expect("the assertion verifies against the viewer's key")
    .claims;
    assert_eq!(claims.sub, "ferroehr-viewer");
    assert_ne!(claims.jti, "", "every assertion carries an id");
}

#[tokio::test]
async fn a_launch_ferrosmart_refuses_draws_the_refusal_and_leaves_the_viewer_on_the_ehr() {
    let stub = ferrosmart(
        http::StatusCode::BAD_REQUEST,
        r#"{"error":"invalid_target","error_description":"the base is not served"}"#,
    )
    .await;
    let (key, _public) = key();
    let viewer = Viewer::start_launching(section(&stub.issuer, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{ehr}"),
            Some(&cookie),
            Some(&[("intent", "launch"), ("app", "growth-chart")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert_eq!(answer.location(), "");
    let main = answer.main();
    assert!(main.contains("The app could not be launched"), "{main}");
    assert!(main.contains("invalid_target"), "{main}");
    assert!(!answer.body.contains(LAUNCH_VALUE));
}

#[tokio::test]
async fn an_app_the_configuration_does_not_list_is_never_launched() {
    let stub = ferrosmart(
        http::StatusCode::CREATED,
        r#"{"launch":"Zm9vYmFyLWxhdW5jaC12YWx1ZS1mb3ItdGhlLXRlc3Qtb25seQ","expires_in":300}"#,
    )
    .await;
    let (key, _public) = key();
    let viewer = Viewer::start_launching(section(&stub.issuer, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{ehr}"),
            Some(&cookie),
            Some(&[("intent", "launch"), ("app", "someone-else")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK);
    assert!(answer.main().contains("The app could not be launched"));
    assert!(stub.received.lock().expect("the stub's record").is_empty());
}

#[tokio::test]
async fn an_ml_dsa_key_signs_the_launch_assertion_with_its_parameter_set() {
    let stub = ferrosmart(
        http::StatusCode::CREATED,
        r#"{"launch":"Zm9vYmFyLWxhdW5jaC12YWx1ZS1mb3ItdGhlLXRlc3Qtb25seQ","expires_in":300}"#,
    )
    .await;
    let generated = crate::ml_dsa::key(ferrohealth_smart::jws::algorithm::Algorithm::MlDsa65);
    let key = tempfile_path::Path::new(&generated.pem);
    let viewer = Viewer::start_launching(section(&stub.issuer, &key.0)).await;
    let cookie = viewer.sign_in().await;
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{}", id("ehr")),
            Some(&cookie),
            Some(&[("intent", "launch"), ("app", "growth-chart")]),
        )
        .await;
    assert_eq!(
        answer.status,
        http::StatusCode::SEE_OTHER,
        "{}",
        answer.body
    );

    let received = stub.received.lock().expect("the stub's record").clone();
    let [form] = received.as_slice() else {
        panic!("one launch request, got {received:?}");
    };
    let assertion = field(form, "client_assertion").expect("the assertion");
    let token = ferrohealth_smart::jws::compact::decode(assertion).expect("a compact JWS");
    assert_eq!(
        token.header().alg,
        ferrohealth_smart::jws::algorithm::Algorithm::MlDsa65,
        "RFC 9964 §5"
    );
    assert_eq!(token.header().kid.as_deref(), Some("viewer-1"));
    let payload = token
        .verify(generated.signing.public_jwk())
        .expect("the assertion verifies against the viewer's ML-DSA-65 key");
    let claims: Assertion = serde_json::from_slice(payload).expect("the assertion's claims");
    assert_eq!(claims.sub, "ferroehr-viewer");
    assert!(
        matches!(&claims.aud, Audience::One(one) if *one == stub.issuer)
            || matches!(claims.aud, Audience::Many(ref many) if *many == [stub.issuer.clone()]),
        "the issuer is the sole audience"
    );
}

/// The claims of the client assertion the ML-DSA test reads.
#[derive(serde::Deserialize)]
struct Assertion {
    /// The subject, the viewer's client id.
    sub: String,
    /// The audience, FerroSMART's issuer.
    aud: Audience,
}

/// An `aud` claim, one string or a list (RFC 7519 §4.1.3).
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Audience {
    /// One audience.
    One(String),
    /// A list of audiences.
    Many(Vec<String>),
}
