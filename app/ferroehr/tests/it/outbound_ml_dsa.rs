// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The client assertion of an outbound integration's client credentials
//! grant (`private_key_jwt`, RFC 7523 §2.2) under an ML-DSA-65 key: the
//! assertion is signed with ML-DSA-65 (RFC 9964 §5) and verifies against the
//! key's public half. The integration client is the one the ITI-20 feed, the
//! PIXm query, the PMIR subscription and the terminology server build with
//! `OutboundClient::build`.

use aws_lc_rs::signature::{ML_DSA_65_SIGNING, PqdsaKeyPair};
use ferrohealth_smart::client::ClientAuth;
use ferrohealth_smart::client::outbound::OutboundClient;
use ferrohealth_smart::config::ServerTls;
use ferrohealth_smart::config::credentials::{CredentialsSection, OauthSection};
use ferrohealth_smart::config::endpoint::EndpointSection;
use ferrohealth_smart::jws::algorithm::Algorithm;
use ferrohealth_smart::jws::compact::decode;
use ferrohealth_smart::jws::key::PrivateKey;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn an_ml_dsa_key_signs_the_client_assertion_with_ml_dsa_65() {
    let pkcs8 = PqdsaKeyPair::generate(&ML_DSA_65_SIGNING)
        .expect("an ML-DSA-65 key pair")
        .to_pkcs8v1()
        .expect("its PKCS#8 form");
    let public = PrivateKey::from_pkcs8(pkcs8.as_ref())
        .expect("the key reads")
        .public_jwk()
        .clone();
    let key = assert_fs::NamedTempFile::new("assertion.pem").expect("a temporary file");
    std::fs::write(
        key.path(),
        pem::encode(&pem::Pem::new("PRIVATE KEY", pkcs8.as_ref())),
    )
    .expect("the key file writes");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "synthetic-access",
            "token_type": "Bearer",
            "expires_in": 300
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fhir/metadata"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let token_endpoint = format!("{}/token", server.uri());
    let endpoint = EndpointSection {
        url: ferrohealth_secret::secret_url::SecretUrl::new(format!("{}/fhir", server.uri())),
        credentials: CredentialsSection {
            oauth: Some(OauthSection {
                token_endpoint: ferrohealth_secret::secret_url::SecretUrl::new(&token_endpoint),
                client_id: "ferroehr".to_owned(),
                client_auth: ClientAuth::PrivateKeyJwt,
                private_key_file: Some(key.path().to_path_buf()),
                ..OauthSection::default()
            }),
            ..CredentialsSection::default()
        },
        ..EndpointSection::default()
    };
    let client = OutboundClient::build(&endpoint, "terminology.endpoint", &ServerTls)
        .expect("the client builds");
    let url = url::Url::parse(&format!("{}/fhir/metadata", server.uri())).expect("a URL");
    let answer = client
        .send(&http::Method::GET, &url, |request| request)
        .await
        .expect("the call is authorized and answered");
    assert_eq!(answer.status(), http::StatusCode::OK);

    let requests = server.received_requests().await.expect("recorded requests");
    let grant = requests
        .iter()
        .find(|request| request.url.path() == "/token")
        .expect("one token request");
    let form: Vec<(String, String)> = url::form_urlencoded::parse(&grant.body)
        .into_owned()
        .collect();
    let assertion = form
        .iter()
        .find(|(name, _)| name == "client_assertion")
        .map(|(_, value)| value.as_str())
        .expect("a client assertion");
    let token = decode(assertion).expect("a compact JWS");
    assert_eq!(token.header().alg, Algorithm::MlDsa65, "RFC 9964 §5");
    let claims = token
        .verify_claims(&public)
        .expect("the assertion verifies against the key's public half");
    assert_eq!(
        claims.get("iss").and_then(serde_json::Value::as_str),
        Some("ferroehr")
    );
}
