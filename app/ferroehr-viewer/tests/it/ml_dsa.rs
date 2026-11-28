// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ML-DSA test keys (RFC 9964 §5) made at run time: the PKCS#8 PEM a
//! section names (RFC 9881 §6) and the signing key a stub provider signs
//! with.

#![expect(
    clippy::expect_used,
    reason = "a key generated for a test that cannot be made is a broken harness, not a test \
              outcome"
)]

use aws_lc_rs::signature::{ML_DSA_44_SIGNING, ML_DSA_65_SIGNING, ML_DSA_87_SIGNING, PqdsaKeyPair};
use base64::Engine as _;
use ferrohealth_smart::jws::algorithm::Algorithm;
use ferrohealth_smart::jws::key::{PrivateKey, SigningKey};

/// One generated ML-DSA key.
pub(crate) struct Key {
    /// The key, signing with its parameter set.
    pub(crate) signing: SigningKey,
    /// Its PKCS#8 form as PEM.
    pub(crate) pem: String,
}

/// A fresh key of the ML-DSA parameter set `algorithm`.
pub(crate) fn key(algorithm: Algorithm) -> Key {
    let parameters = match algorithm {
        Algorithm::MlDsa44 => &ML_DSA_44_SIGNING,
        Algorithm::MlDsa87 => &ML_DSA_87_SIGNING,
        _ => &ML_DSA_65_SIGNING,
    };
    let pkcs8 = PqdsaKeyPair::generate(parameters)
        .expect("an ML-DSA key pair")
        .to_pkcs8v1()
        .expect("its PKCS#8 form");
    let private = PrivateKey::from_pkcs8(pkcs8.as_ref()).expect("the key reads");
    let signing = SigningKey::new(private, algorithm).expect("the key signs its parameter set");
    let body = base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref());
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).expect("base64 is ASCII"))
        .collect();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    );
    Key { signing, pem }
}
