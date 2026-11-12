// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `DPoP` proof check reads the server's one public URL (#5465): a
//! proof's `htu` must name `[server] public_base_url`, the mount path
//! `server.base_path` and the request path (RFC 9449 §4.3, check 9), and a
//! proof addressed to the URL the server sees behind its proxy is refused.
//! The proofs are real: an ES256 key signs each, and the token is bound to
//! its thumbprint (RFC 9449 §6.1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions, diagnostics and fixtures"
)]

use axum::Router;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::section::{BasePath, ServerSection};
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::sender::SenderConstraint;
use ferrohealth_smart::sender::proof::DpopSection;
use http::{StatusCode, header};
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;

use crate::common;

/// The issuer of the bound tokens.
const ISSUER: &str = "https://as.example/realms/staff";
/// The audience of its tokens.
const AUDIENCE: &str = "ferroehr";
/// The one public URL of the deployment, behind a proxy that serves it under
/// `/cdr`.
const PUBLIC: &str = "https://gw.example/cdr";
/// The path the server mounts every route under.
const MOUNT: &str = "/ehr";
/// The route every request asks for, below the mount path.
const ROUTE: &str = "/ferroehr/rest/openehr/v1/ehr";

/// A client's proof key: its signing key and its public JWK.
struct Key {
    /// The ES256 signing key.
    signing: EncodingKey,
    /// The public key a proof's header carries.
    jwk: Jwk,
}

impl Key {
    /// A fresh ES256 key.
    fn new() -> Self {
        let pair = rcgen::KeyPair::generate().expect("an ES256 key");
        let point = pair.public_key_raw();
        let (x, y) = point
            .get(1..33)
            .zip(point.get(33..65))
            .expect("an uncompressed P-256 point");
        let jwk = serde_json::from_value(json!({
            "kty": "EC", "crv": "P-256",
            "x": URL_SAFE_NO_PAD.encode(x), "y": URL_SAFE_NO_PAD.encode(y),
        }))
        .expect("a public JWK");
        let signing = EncodingKey::from_ec_pem(pair.serialize_pem().as_bytes()).expect("a key");
        Self { signing, jwk }
    }

    /// A proof of a `GET` addressed to `htu`, bound to `token`.
    fn proof(&self, htu: &str, token: &str) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("dpop+jwt".to_owned());
        header.jwk = Some(self.jwk.clone());
        let ath = {
            use sha2::Digest as _;
            URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(token.as_bytes()))
        };
        let claims = json!({
            "jti": uuid::Uuid::now_v7().to_string(),
            "htm": "GET",
            "htu": htu,
            "iat": jiff::Timestamp::now().as_second(),
            "ath": ath,
        });
        encode(&header, &claims, &self.signing).expect("a signed proof")
    }

    /// An access token of [`ISSUER`] bound to this key (RFC 9449 §6.1).
    fn token(&self) -> String {
        let exp = jiff::Timestamp::now().as_second() + 3600;
        let jkt = ferrohealth_smart::client::assertion::thumbprint(&self.jwk);
        let bearer = common::bearer(&json!({
            "iss": ISSUER, "aud": AUDIENCE, "exp": exp, "sub": "someone",
            "cnf": { "jkt": jkt },
        }));
        bearer
            .strip_prefix("Bearer ")
            .expect("a bearer credential")
            .to_owned()
    }
}

/// The server: a sender-constrained issuer, `[auth.dpop]`, and `[server]`
/// naming the public URL once and the mount path.
async fn app() -> (testkit::TestDb, Router) {
    let server = ServerConfig {
        shared: ServerSection {
            base_path: BasePath::new(MOUNT).expect("a mount path"),
            ..common::server()
        },
        public_base_url: Some(SecretUrl::new(PUBLIC)),
    };
    let mut auth = common::auth(AuthSection {
        issuer: vec![IssuerEntry {
            jwks_file: common::jwks_file().display().to_string(),
            sender_constraint: SenderConstraint::Required,
            ..common::entry(ISSUER, AUDIENCE)
        }],
        dpop: Some(DpopSection::default()),
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let config = AppConfig {
        server: server.shared.clone(),
        public_root: server.public_root(),
        auth,
        ..common::api_config(false)
    };
    let (db, service) = common::test_service().await;
    (db, common::router_with(config, service).await)
}

/// `GET` the mounted route with `token` and `proof`; the status and the
/// challenge.
async fn send(app: &Router, token: &str, proof: &str) -> (StatusCode, Option<String>) {
    let request = common::req("GET", format!("{MOUNT}{ROUTE}"))
        .header(header::AUTHORIZATION, format!("DPoP {token}"))
        .header("dpop", proof)
        .build();
    let (status, headers, _body) = common::send(app, request).await;
    let challenge = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    (status, challenge)
}

#[tokio::test]
async fn a_proof_addressed_to_the_public_url_and_mount_path_is_admitted() {
    let (_db, app) = app().await;
    let key = Key::new();
    let token = key.token();
    let proof = key.proof(&format!("{PUBLIC}{MOUNT}{ROUTE}"), &token);
    let (status, challenge) = send(&app, &token, &proof).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "the proof names the URL the client addressed: {challenge:?}"
    );
}

#[tokio::test]
async fn a_proof_addressed_elsewhere_is_refused() {
    let (_db, app) = app().await;
    let key = Key::new();
    let token = key.token();
    for htu in [
        format!("http://10.0.0.7:8080{MOUNT}{ROUTE}"),
        format!("https://gw.example{MOUNT}{ROUTE}"),
        format!("{PUBLIC}{ROUTE}"),
    ] {
        let proof = key.proof(&htu, &token);
        let (status, challenge) = send(&app, &token, &proof).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{htu}: {challenge:?}");
        assert!(
            challenge
                .as_deref()
                .is_some_and(|text| text.contains("DPoP")),
            "{htu}: {challenge:?}"
        );
    }
}
