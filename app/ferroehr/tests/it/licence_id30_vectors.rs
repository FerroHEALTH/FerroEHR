// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The licence check over the shared ML-DSA-65+Ed25519 (RFC 9980 algorithm
//! ID 30) vectors in `corpus/openpgp-id30/`: each case with its anchors, its
//! day and the answer every verifier of these documents gives.

use std::path::{Path, PathBuf};

use ferroehr::licence::token::{Token, TokenError};
use ferroehr::licence::verify::{VerifyError, verify};
use pgp::composed::{Deserializable as _, SignedPublicKey};
use pgp::crypto::hash::HashAlgorithm;

/// The answer a case expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Ok,
    UntrustedPrimary,
    BrokenBindings,
    ExtraSignatures,
    NotTextSignature,
    NoIssuer,
    UnknownSigner,
    NotSigningCapable,
    ExpiredAtSigning,
    WeakDigest,
    BadSignature,
    Payload,
    NotYetValid,
    Expired,
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/openpgp-id30")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn anchors(names: &[&str]) -> Vec<SignedPublicKey> {
    names
        .iter()
        .map(|name| {
            SignedPublicKey::from_string(&read(
                &corpus().join("anchors").join(format!("{name}.asc")),
            ))
            .expect("an anchor")
            .0
        })
        .collect()
}

fn answer(result: &Result<ferroehr::licence::verify::Verified, VerifyError>) -> Answer {
    match result {
        Ok(_) => Answer::Ok,
        Err(VerifyError::UntrustedPrimary(_)) => Answer::UntrustedPrimary,
        Err(VerifyError::BrokenBindings(_)) => Answer::BrokenBindings,
        Err(VerifyError::ExtraSignatures(_)) => Answer::ExtraSignatures,
        Err(VerifyError::NotTextSignature) => Answer::NotTextSignature,
        Err(VerifyError::NoIssuer) => Answer::NoIssuer,
        Err(VerifyError::UnknownSigner) => Answer::UnknownSigner,
        Err(VerifyError::NotSigningCapable(_)) => Answer::NotSigningCapable,
        Err(VerifyError::ExpiredAtSigning { .. }) => Answer::ExpiredAtSigning,
        Err(VerifyError::WeakDigest { .. }) => Answer::WeakDigest,
        Err(VerifyError::BadSignature(_)) => Answer::BadSignature,
        Err(VerifyError::Payload(_)) => Answer::Payload,
        Err(VerifyError::NotYetValid { .. }) => Answer::NotYetValid,
        Err(VerifyError::Expired { .. }) => Answer::Expired,
        Err(other) => panic!("an answer no case expects: {other:?}"),
    }
}

/// Every case of the shared vectors: the signed file, the anchors, the day
/// it is checked on, and the answer.
#[test]
fn every_shared_vector_gives_its_answer() {
    let cases: [(&str, &[&str], &str, Answer); 17] = [
        ("ok-sha3-256", &["A", "B"], "2026-11-01", Answer::Ok),
        ("ok-sha512", &["A", "B"], "2027-10-31", Answer::Ok),
        (
            "untrusted",
            &["A", "B"],
            "2026-11-01",
            Answer::UntrustedPrimary,
        ),
        (
            "no-direct-key",
            &["N"],
            "2026-11-01",
            Answer::BrokenBindings,
        ),
        (
            "two-signatures",
            &["A", "B"],
            "2026-11-01",
            Answer::ExtraSignatures,
        ),
        (
            "binary-signature",
            &["A", "B"],
            "2026-11-01",
            Answer::NotTextSignature,
        ),
        ("key-id-only", &["A", "B"], "2026-11-01", Answer::NoIssuer),
        (
            "unknown-signer",
            &["A", "B"],
            "2026-11-01",
            Answer::UnknownSigner,
        ),
        (
            "not-signing",
            &["A", "B", "C"],
            "2026-11-01",
            Answer::NotSigningCapable,
        ),
        (
            "expired-at-signing",
            &["A", "B", "D"],
            "2026-11-01",
            Answer::ExpiredAtSigning,
        ),
        (
            "weak-digest-sha224",
            &["A", "B"],
            "2026-11-01",
            Answer::WeakDigest,
        ),
        ("tampered", &["A", "B"], "2026-11-01", Answer::BadSignature),
        (
            "ed25519-half-broken",
            &["A", "B"],
            "2026-11-01",
            Answer::BadSignature,
        ),
        (
            "mldsa-half-broken",
            &["A", "B"],
            "2026-11-01",
            Answer::BadSignature,
        ),
        ("unknown-field", &["A", "B"], "2026-11-01", Answer::Payload),
        (
            "ok-sha3-256",
            &["A", "B"],
            "2026-10-31",
            Answer::NotYetValid,
        ),
        ("ok-sha3-256", &["A", "B"], "2027-11-01", Answer::Expired),
    ];
    for (file, names, day, expected) in cases {
        let token = Token::parse(&read(&corpus().join("signed").join(format!("{file}.asc"))))
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        let result = verify(&token, &anchors(names), day.parse().expect("a day"));
        assert_eq!(answer(&result), expected, "{file} on {day}: {result:?}");
    }
}

/// The good vectors grant the licence they sign.
#[test]
fn the_good_vectors_grant_their_licence() {
    let token = Token::parse(&read(&corpus().join("signed/ok-sha512.asc"))).expect("a token");
    let verified = verify(
        &token,
        &anchors(&["A", "B"]),
        "2027-10-31".parse().expect("a day"),
    )
    .expect("the licence verifies");
    assert_eq!(
        verified.licence.id.to_string(),
        "019a1171-8f20-7b3e-a5c4-2e9d7f60b1a9"
    );
    assert_eq!(
        verified.licence.licensee,
        "Ünïcode \"Quoted\" / Slash & Co. — ✓"
    );
}

/// A token without its certificate is refused when it is read.
#[test]
fn a_token_without_a_certificate_is_refused_when_read() {
    assert!(matches!(
        Token::parse(&read(&corpus().join("signed/no-certificate.asc"))),
        Err(TokenError::MissingCertificate)
    ));
}

/// SHA2-224 is refused for its digest before any cryptography.
#[test]
fn the_weak_digest_names_sha2_224() {
    let token =
        Token::parse(&read(&corpus().join("signed/weak-digest-sha224.asc"))).expect("a token");
    assert!(matches!(
        verify(
            &token,
            &anchors(&["A", "B"]),
            "2026-11-01".parse().expect("a day")
        ),
        Err(VerifyError::WeakDigest {
            hash: HashAlgorithm::Sha224
        })
    ));
}
