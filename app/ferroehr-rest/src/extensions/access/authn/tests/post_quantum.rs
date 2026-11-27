// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access tokens signed with ML-DSA-65 (RFC 9964 §5), verified under the
//! issuer's default algorithms and under `[auth] post_quantum_only`, which
//! narrows them to the ML-DSA parameter sets.

use ferrohealth_smart::config::IssuerEntry;
use ferrohealth_smart::jws::algorithm::Algorithm;
use ferrohealth_smart::jws::compact::{Header, encode};
use ferrohealth_smart::jws::key::{PrivateKey, SigningKey};

use super::*;

/// The `kid` of the ML-DSA-65 test key.
const ML_DSA_KID: &str = "ml-dsa-65-test";

/// The ML-DSA-65 test key, expanded from a fixed seed.
fn ml_dsa_key() -> SigningKey {
    let private =
        PrivateKey::from_ml_dsa_seed(Algorithm::MlDsa65, &[7; 32]).expect("a 32-octet seed");
    SigningKey::new(private, Algorithm::MlDsa65).expect("an ML-DSA-65 key signs ML-DSA-65")
}

/// The key set of the committed RSA test key and the ML-DSA-65 key, in a
/// file of this process.
fn jwks_file() -> String {
    let mut set: Value = serde_json::from_str(include_str!(
        "../../../../../../../docs/conformance/party/smart/jwks.json"
    ))
    .expect("the committed key set");
    let ml_dsa = ml_dsa_key()
        .public_jwk()
        .clone()
        .with_kid(ML_DSA_KID)
        .with_signature_use()
        .to_value();
    set["keys"].as_array_mut().expect("a key list").push(ml_dsa);
    let path = std::env::temp_dir().join(format!(
        "ferroehr-rest-unit-ml-dsa-jwks-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, set.to_string()).expect("write the key set");
    path.display().to_string()
}

/// An RFC 9068 access token carrying `claims`, signed with the ML-DSA-65 key.
fn ml_dsa_token(claims: &Value) -> String {
    let mut header = Header::new(Algorithm::MlDsa65);
    header.typ = Some("at+jwt".to_owned());
    header.kid = Some(ML_DSA_KID.to_owned());
    encode(&header, claims, &ml_dsa_key()).expect("the token signs")
}

/// An authenticator trusting the test issuer from [`jwks_file`], with its
/// algorithms narrowed as `post_quantum_only` narrows them when `on`.
async fn authenticator(on: bool) -> Arc<Authenticator> {
    let mut auth = test_support::trusting(IssuerEntry {
        jwks_file: jwks_file(),
        ..test_support::entry(ISSUER)
    });
    if on {
        ferroehr::config::auth_section::post_quantum(&mut auth.shared)
            .expect("the default lists narrow");
        auth.post_quantum_only = true;
    }
    Authenticator::new(
        &AppConfig {
            auth,
            ..AppConfig::default()
        },
        None,
        memory(),
    )
    .await
    .expect("a bearer authenticator")
}

/// Authenticates `token` at `authenticator`.
async fn verdict(authenticator: &Authenticator, token: &str) -> Result<Authenticated, AuthError> {
    authenticator
        .authenticate(&headers(&format!("Bearer {token}")))
        .await
}

#[tokio::test]
async fn post_quantum_only_admits_an_ml_dsa_token_and_refuses_a_classical_one() {
    let claims = test_support::claims("alice", "openid user/composition-*.r");
    let on = authenticator(true).await;
    let admitted = verdict(&on, &ml_dsa_token(&claims))
        .await
        .expect("an ML-DSA-65 token verifies");
    assert_eq!(admitted.principal.subject, "alice");
    let refused = verdict(&on, &test_support::token(&claims)).await;
    assert!(
        refused.is_err(),
        "an RS256 token is refused under post_quantum_only"
    );
}

#[tokio::test]
async fn the_default_algorithms_admit_a_classical_token_and_no_ml_dsa_one() {
    let claims = test_support::claims("alice", "openid user/composition-*.r");
    let off = authenticator(false).await;
    verdict(&off, &test_support::token(&claims))
        .await
        .expect("an RS256 token verifies under the defaults");
    assert!(
        verdict(&off, &ml_dsa_token(&claims)).await.is_err(),
        "the issuer's default list names no ML-DSA parameter set"
    );
}

#[tokio::test]
async fn a_token_naming_none_or_an_hmac_algorithm_is_refused_under_post_quantum_only() {
    let claims = test_support::claims("alice", "openid user/composition-*.r");
    let on = authenticator(true).await;
    let body = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        claims.to_string(),
    );
    for alg in ["none", "HS256", "ML-DSA-128"] {
        let head = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            json!({ "alg": alg, "typ": "at+jwt", "kid": ML_DSA_KID }).to_string(),
        );
        let forged = format!("{head}.{body}.c2lnbmF0dXJl");
        assert!(verdict(&on, &forged).await.is_err(), "{alg}");
    }
}
