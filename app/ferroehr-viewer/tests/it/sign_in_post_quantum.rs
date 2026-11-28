// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OpenID Connect sign-in with an ID Token signed with ML-DSA-65
//! (RFC 9964 §5) or ES256, at an in-process provider, under the default
//! `[auth]` and under `auth.post_quantum_only`, which accepts ML-DSA alone.

#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction: a loopback listener, a test key, a \
              static request; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]

use std::sync::{Arc, Mutex};

use ferrohealth_smart::jws::algorithm::Algorithm;
use ferrohealth_smart::jws::compact::{Header, encode};
use ferrohealth_smart::jws::key::{PrivateKey, SigningKey};

use crate::ssr_app::{Answer, Viewer};

/// The viewer's client id at the provider.
const CLIENT: &str = "ferroehr-viewer-sign-in";

/// The provider's `kid`.
const KID: &str = "op-1";

/// A provider signing its ID Tokens with `key`, and the `nonce` of the
/// sign-in in flight.
struct Provider {
    /// Its issuer, `http://127.0.0.1:<port>`.
    issuer: String,
    /// The `nonce` the viewer sent.
    nonce: Arc<Mutex<String>>,
}

/// The claims of the provider's ID Token (OpenID Connect Core 1.0 §2).
#[derive(serde::Serialize)]
struct IdToken {
    /// The issuer.
    iss: String,
    /// The viewer's client id.
    aud: &'static str,
    /// The signed-in person.
    sub: &'static str,
    /// The `nonce` of the authentication request.
    nonce: String,
    /// The issue time.
    iat: i64,
    /// The expiry.
    exp: i64,
}

/// The token endpoint's answer to the code exchange (RFC 6749 §5.1).
#[derive(serde::Serialize)]
struct TokenAnswer {
    /// The access token.
    access_token: &'static str,
    /// Its type.
    token_type: &'static str,
    /// Its lifetime in seconds.
    expires_in: u32,
    /// The ID Token.
    id_token: String,
}

/// A P-256 key that signs ES256.
fn es256_key() -> SigningKey {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("a P-256 key");
    let private = PrivateKey::from_pem(pair.serialize_pem().as_bytes()).expect("the key reads");
    SigningKey::new(private, Algorithm::Es256).expect("a P-256 key signs ES256")
}

/// Starts a provider whose key set publishes `key` and whose token
/// endpoint answers the code exchange with an ID Token `key` signs.
async fn provider(key: SigningKey) -> Provider {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let issuer = format!(
        "http://{}",
        listener.local_addr().expect("the bound address")
    );
    let nonce: Arc<Mutex<String>> = Arc::default();
    let jwks = format!(
        r#"{{"keys":[{}]}}"#,
        key.public_jwk()
            .clone()
            .with_kid(KID)
            .with_signature_use()
            .to_value()
    );
    let on_token = {
        let issuer = issuer.clone();
        let nonce = Arc::clone(&nonce);
        let key = Arc::new(key);
        move || {
            let now = jiff::Timestamp::now().as_second();
            let mut header = Header::new(key.algorithm());
            header.kid = Some(KID.to_owned());
            let claims = IdToken {
                iss: issuer.clone(),
                aud: CLIENT,
                sub: "synthetic-clinician",
                nonce: nonce.lock().expect("the nonce").clone(),
                iat: now,
                exp: now + 300,
            };
            let id_token = encode(&header, &claims, &key).expect("the ID Token signs");
            async move {
                axum::Json(TokenAnswer {
                    access_token: "synthetic-access",
                    token_type: "Bearer",
                    expires_in: 300,
                    id_token,
                })
            }
        }
    };
    let app = axum::Router::new()
        .route(
            "/jwks",
            axum::routing::get(move || {
                let jwks = jwks.clone();
                async move { ([(http::header::CONTENT_TYPE, "application/json")], jwks) }
            }),
        )
        .route("/token", axum::routing::post(on_token));
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the stub serves until the test ends");
    });
    Provider { issuer, nonce }
}

/// `[oidc]` at `provider`, every endpoint given.
fn oidc(provider: &Provider) -> ferrohealth_session::config::OidcSection {
    let url = |path: &str| {
        Some(ferrohealth_secret::secret_url::SecretUrl::new(format!(
            "{}{path}",
            provider.issuer
        )))
    };
    ferrohealth_session::config::OidcSection {
        issuer: ferrohealth_secret::secret_url::SecretUrl::new(provider.issuer.clone()),
        authorization_endpoint: url("/authorize"),
        token_endpoint: url("/token"),
        jwks_uri: url("/jwks"),
        client_id: CLIENT.to_owned(),
        client_secret: Some(ferrohealth_secret::secret::Secret::new(
            "synthetic-client-secret",
        )),
        redirect_uri: ferrohealth_secret::secret_url::SecretUrl::new(
            "https://viewer.test/auth/callback",
        ),
        ..ferroehr_viewer::config::ViewerConfig::default().oidc
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

/// Signs in at a provider signing with `key` under `[auth]`
/// `post_quantum_only`, and returns whether the viewer opened a session.
async fn signs_in(key: SigningKey, post_quantum_only: bool) -> bool {
    let provider = provider(key).await;
    let auth = ferroehr_viewer::config::AuthConfig {
        post_quantum_only,
        ..ferroehr_viewer::config::AuthConfig::default()
    };
    let viewer = Viewer::start_signing_in_with(
        oidc(&provider),
        ferroehr_viewer::launch::LaunchSection::default(),
        auth,
    )
    .await;
    let begun = viewer
        .send(http::Method::GET, "/login?return=%2F", None, None)
        .await;
    assert_eq!(begun.status, http::StatusCode::SEE_OTHER, "{}", begun.body);
    let location = begun.location().to_owned();
    *provider.nonce.lock().expect("the nonce") = query_value(&location, "nonce");
    let state = query_value(&location, "state");
    let back = viewer
        .send(
            http::Method::GET,
            &format!("/auth/callback?code=synthetic-code&state={state}"),
            Some(&kept_cookies(&begun)),
            None,
        )
        .await;
    back.status == http::StatusCode::SEE_OTHER && back.location() == "/"
}

#[tokio::test]
async fn an_ml_dsa_id_token_signs_in_with_or_without_post_quantum_only() {
    for post_quantum_only in [false, true] {
        let key = crate::ml_dsa::key(Algorithm::MlDsa65).signing;
        assert!(
            signs_in(key, post_quantum_only).await,
            "post_quantum_only = {post_quantum_only}"
        );
    }
}

#[tokio::test]
async fn an_es256_id_token_signs_in_by_default_and_is_refused_under_post_quantum_only() {
    assert!(
        signs_in(es256_key(), false).await,
        "the default accepts ES256"
    );
    assert!(
        !signs_in(es256_key(), true).await,
        "post_quantum_only refuses a classical ID Token"
    );
}

#[test]
fn post_quantum_only_names_the_three_ml_dsa_parameter_sets_alone() {
    let on = ferroehr_viewer::config::AuthConfig {
        post_quantum_only: true,
        ..ferroehr_viewer::config::AuthConfig::default()
    };
    assert_eq!(
        on.id_token_algorithms(),
        [Algorithm::MlDsa44, Algorithm::MlDsa65, Algorithm::MlDsa87]
    );
    let off = ferroehr_viewer::config::AuthConfig::default().id_token_algorithms();
    assert!(off.contains(&Algorithm::Es256) && off.contains(&Algorithm::MlDsa65));
}
