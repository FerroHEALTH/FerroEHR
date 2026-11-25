// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenPGP`-mode integration tests: sign→verify round-trip with a
//! generated ML-DSA-65+Ed25519 key, tamper detection, armor-parse failures, the
//! fail-closed boot validation (missing path, garbled key, wrong passphrase),
//! the refusal of every classical signing key, and the stored classical
//! signatures that retired certificates keep verifying.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
use ferroehr::versioning::signature::key::KeyError;
use ferroehr::versioning::signature::signer::{Signer, SigningError};
use ferroehr::versioning::signature::verify::Verdict;
use ferrohealth_openpgp_pq::encoding::{self, Encoding};
use ferrohealth_openpgp_pq::key::Key;
use pgp::composed::{
    ArmorOptions, Deserializable as _, DetachedSignature, KeyType, SecretKeyParamsBuilder,
    SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::types::{KeyDetails as _, Password};
use rand::rngs::OsRng;

/// A short openEHR-canonical-form-like string to sign in the tests.
const CANONICAL: &str = r#"{"_type":"ORIGINAL_VERSION","uid":{"value":"a::b::1"}}"#;

/// Generate an armored ML-DSA-65+Ed25519 `OpenPGP` secret key (RFC 9980
/// algorithm ID 30), optionally passphrase-locked.
fn generate_key(passphrase: Option<&str>) -> String {
    let mut key =
        Key::generate(&["ferroehr-signing test <test@ferroehr.local>"]).expect("generate key");
    if let Some(p) = passphrase {
        key.lock(&Password::from(p)).expect("lock key");
    }
    armored(std::slice::from_ref(key.signed()))
}

/// `items` as one ASCII-armored block.
fn armored<T: encoding::Block>(items: &[T]) -> String {
    String::from_utf8(encoding::encode(items, Encoding::Armored).expect("armor")).expect("text")
}

/// Generate a classical `key_type` signing key, as releases before the move to
/// ML-DSA-65+Ed25519 signed with.
fn generate_classical_key(key_type: KeyType) -> SignedSecretKey {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(key_type)
        .can_sign(true)
        .primary_user_id("ferroehr-signing classical <classical@ferroehr.local>".into());
    builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key")
}

/// A detached signature over [`CANONICAL`] by `key`'s primary key, as a
/// release before the move stored it.
fn classical_signature(key: &SignedSecretKey) -> String {
    // NOTE: RFC 9580 §5.2.3.2; a digest shorter than the curve fails, and rPGP holds Ed448 to 512 bits.
    DetachedSignature::sign_binary_data(
        OsRng,
        &key.primary_key,
        &Password::empty(),
        HashAlgorithm::Sha512,
        CANONICAL.as_bytes(),
    )
    .expect("a classical signature")
    .to_armored_string(ArmorOptions::default())
    .expect("armor the signature")
}

/// The armored public certificate of `key`.
fn classical_certificate(key: &SignedSecretKey) -> String {
    key.to_public_key()
        .to_armored_string(ArmorOptions::default())
        .expect("armor public certificate")
}

/// Write `content` to a unique temp file and return its path.
fn temp_file(content: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("ferroehr-signing-{}-{n}.asc", std::process::id()));
    std::fs::write(&path, content).expect("write temp key");
    path
}

/// Build a `pgp`-mode [`Signer`] over an armored key written to a temp file.
fn pgp_signer(armored_key: &str, passphrase: Option<&str>) -> Result<Signer, SigningError> {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_file: Some(temp_file(armored_key)),
        key_passphrase: passphrase.map(ferrohealth_secret::secret::Secret::new),
        key_passphrase_file: None,
        retired_key_files: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    Signer::from_config(&config)
}

#[test]
fn pgp_sign_verify_round_trip() {
    let signer = pgp_signer(&generate_key(None), None).expect("build signer");
    let sig = signer.sign(CANONICAL).expect("sign");
    assert!(
        sig.contains("BEGIN PGP SIGNATURE"),
        "expected ASCII-armored signature, got: {sig}"
    );
    assert_eq!(signer.verify(CANONICAL, &sig), Verdict::PgpValid);
}

#[test]
fn pgp_tamper_detected() {
    let signer = pgp_signer(&generate_key(None), None).expect("build signer");
    let sig = signer.sign(CANONICAL).expect("sign");
    // The served canonical form differs from the signed one → invalid.
    let tampered = format!("{CANONICAL} ");
    assert_eq!(signer.verify(&tampered, &sig), Verdict::PgpInvalid);
}

#[test]
fn pgp_passphrase_locked_round_trip() {
    let key = generate_key(Some("correct horse"));
    let signer = pgp_signer(&key, Some("correct horse")).expect("build signer with passphrase");
    let sig = signer.sign(CANONICAL).expect("sign");
    assert_eq!(signer.verify(CANONICAL, &sig), Verdict::PgpValid);
}

#[test]
fn garbled_pgp_armor_is_invalid() {
    let signer = pgp_signer(&generate_key(None), None).expect("build signer");
    let garbage =
        "-----BEGIN PGP SIGNATURE-----\nnot base64 at all!!!\n-----END PGP SIGNATURE-----";
    assert_eq!(signer.verify(CANONICAL, garbage), Verdict::PgpInvalid);
}

#[test]
fn foreign_opaque_signature_is_client_foreign() {
    let signer = pgp_signer(&generate_key(None), None).expect("build signer");
    // Neither our `sha256:` digest nor PGP armor → a foreign client signature.
    assert_eq!(
        signer.verify(CANONICAL, "some-other-scheme:opaque-blob"),
        Verdict::ClientForeign
    );
}

#[test]
fn boot_fails_when_key_path_missing() {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_file: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_files: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Off),
    };
    assert!(matches!(
        Signer::from_config(&config),
        Err(SigningError::MissingKeyPath)
    ));
}

#[test]
fn boot_fails_on_garbled_key_file() {
    let err =
        pgp_signer("this is not an OpenPGP key", None).expect_err("garbled key must fail boot");
    assert!(
        matches!(err, SigningError::Key(KeyError::Parse(_))),
        "got {err:?}"
    );
}

#[test]
fn boot_fails_on_wrong_passphrase() {
    // A passphrase-locked key loaded with the wrong passphrase does not
    // unlock, so the server does not start (fail-closed).
    let key = generate_key(Some("the-right-one"));
    let err = pgp_signer(&key, Some("the-wrong-one")).expect_err("wrong passphrase must fail");
    assert!(
        matches!(
            err,
            SigningError::Key(KeyError::Composite(
                ferrohealth_openpgp_pq::error::Error::Locked(_)
            ))
        ),
        "got {err:?}"
    );
}

/// `pgp` mode signs version 6 ML-DSA-65+Ed25519 signatures (RFC 9980
/// algorithm ID 30, §5.3.1).
#[test]
fn pgp_mode_signs_with_ml_dsa_65_ed25519() {
    let signer = pgp_signer(&generate_key(None), None).expect("build signer");
    let armored = signer.sign(CANONICAL).expect("sign");
    let (signature, _) = DetachedSignature::from_string(&armored).expect("a detached signature");
    assert_eq!(
        signature.signature.version(),
        pgp::packet::SignatureVersion::V6
    );
    assert_eq!(
        signature.signature.config().map(|config| config.pub_alg),
        Some(PublicKeyAlgorithm::from(30_u8)),
        "algorithm 30"
    );
    assert!(
        !armored
            .lines()
            .any(|line| line.starts_with('=') && line.len() == 5),
        "no CRC24 (RFC 9580 §6.1)"
    );
}

/// The classical signing keys: every one is refused at boot, with the
/// algorithm each reports.
fn classical_key_types() -> Vec<(KeyType, PublicKeyAlgorithm)> {
    vec![
        (KeyType::Ed25519Legacy, PublicKeyAlgorithm::EdDSALegacy),
        (KeyType::Ed25519, PublicKeyAlgorithm::Ed25519),
        (KeyType::Rsa(2048), PublicKeyAlgorithm::RSA),
        (KeyType::ECDSA(ECCCurve::P256), PublicKeyAlgorithm::ECDSA),
        (KeyType::ECDSA(ECCCurve::P384), PublicKeyAlgorithm::ECDSA),
        (KeyType::ECDSA(ECCCurve::P521), PublicKeyAlgorithm::ECDSA),
        (
            KeyType::ECDSA(ECCCurve::Secp256k1),
            PublicKeyAlgorithm::ECDSA,
        ),
        (KeyType::Ed448, PublicKeyAlgorithm::Ed448),
    ]
}

/// A classical signing key stops the boot with an error that names the key
/// and points at `ferrohealth-sop generate-key`.
#[test]
fn every_classical_signing_key_is_refused_at_boot() {
    for (key_type, expected) in classical_key_types() {
        let key = generate_classical_key(key_type.clone());
        let armored = key
            .to_armored_string(ArmorOptions::default())
            .expect("armor key");
        let err = pgp_signer(&armored, None).expect_err("a classical key must be refused");
        let SigningError::Key(KeyError::ClassicalKey {
            fingerprint,
            algorithm,
        }) = &err
        else {
            panic!("{key_type:?}: expected ClassicalKey, got {err:?}");
        };
        assert_eq!(*algorithm, expected, "{key_type:?}");
        assert_eq!(*fingerprint, format!("{:X}", key.primary_key.fingerprint()));
        let message = err.source().map(ToString::to_string).unwrap_or_default();
        for needle in [
            fingerprint.as_str(),
            "ferrohealth-sop generate-key",
            "retired_key_files",
        ] {
            assert!(message.contains(needle), "{key_type:?}: {message}");
        }
    }
}

#[test]
fn boot_fails_on_missing_key_file() {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_file: Some(PathBuf::from("/nonexistent/ferroehr-signing/key.asc")),
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_files: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Off),
    };
    assert!(matches!(
        Signer::from_config(&config),
        Err(SigningError::Key(KeyError::Read { .. }))
    ));
}

#[test]
fn digest_mode_verify_matches_and_mismatches() {
    // A digest-mode signer round-trips and detects tampering.
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Digest,
        key_file: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_files: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    let signer = Signer::from_config(&config).expect("digest signer");
    let sig = signer.sign(CANONICAL).expect("sign");
    assert!(sig.starts_with("sha256:"));
    assert_eq!(signer.verify(CANONICAL, &sig), Verdict::DigestMatch);
    assert_eq!(
        signer.verify(&format!("{CANONICAL} "), &sig),
        Verdict::DigestMismatch
    );
    // A client-supplied PGP signature in digest mode is foreign (no key).
    let pgp_key = generate_key(None);
    let pgp_signer = pgp_signer(&pgp_key, None).expect("pgp signer");
    let pgp_sig = pgp_signer.sign(CANONICAL).expect("pgp sign");
    assert_eq!(signer.verify(CANONICAL, &pgp_sig), Verdict::ClientForeign);
}

/// Generate an ML-DSA-65+Ed25519 key and return `(armored secret, armored
/// PUBLIC certificate)`; the public half is what a retired key is configured as.
fn generate_key_pair() -> (String, String) {
    let key = Key::generate(&["ferroehr-signing rotation <rotation@ferroehr.local>"])
        .expect("generate key");
    (
        armored(std::slice::from_ref(key.signed())),
        armored(std::slice::from_ref(key.certificate().signed())),
    )
}

/// A `pgp`-mode signer whose `active` key signs and whose `retired` public
/// certificates are kept for verification only.
fn signer_with_retired(active: &str, retired: &[&str]) -> Signer {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_file: Some(temp_file(active)),
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_files: retired.iter().map(|cert| temp_file(cert)).collect(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    Signer::from_config(&config).expect("build signer")
}

/// The rotation property: a version signed before a key rotation still
/// verifies afterwards, provided the retired certificate is retained.
///
/// A `VERSION.signature` is an immutable committed fact (RM common master06
/// §Digital Signature) and carries no key identifier, so re-signing history is
/// not available — retaining the old public key is the only mechanism.
#[test]
fn a_version_signed_before_a_rotation_still_verifies() {
    let (old_secret, old_public) = generate_key_pair();
    let (new_secret, _) = generate_key_pair();

    let signature = signer_with_retired(&old_secret, &[])
        .sign(CANONICAL)
        .expect("sign with the pre-rotation key");

    let rotated = signer_with_retired(&new_secret, &[&old_public]);
    assert_eq!(
        rotated.verify(CANONICAL, &signature),
        Verdict::PgpValid,
        "a version signed before the rotation must still verify against the retained certificate"
    );

    // And this is the defect the retained certificate exists to prevent: with
    // the old key discarded, reading that same intact record is an integrity
    // failure under the default strict policy.
    let discarded = signer_with_retired(&new_secret, &[]);
    assert_eq!(
        discarded.verify(CANONICAL, &signature),
        Verdict::PgpInvalid,
        "without the retired certificate the pre-rotation signature cannot verify"
    );
}

/// A version signed by a classical key before the move to ML-DSA-65+Ed25519
/// still verifies afterwards against the retired classical certificate.
#[test]
fn a_classical_signature_verifies_after_the_move_to_ml_dsa_65_ed25519() {
    for (key_type, _) in classical_key_types() {
        let old = generate_classical_key(key_type.clone());
        let signature = classical_signature(&old);
        let (new_secret, _) = generate_key_pair();
        let rotated = signer_with_retired(&new_secret, &[&classical_certificate(&old)]);
        assert_eq!(
            rotated.verify(CANONICAL, &signature),
            Verdict::PgpValid,
            "{key_type:?}: a retired certificate verifies the signature it stored"
        );
        assert_eq!(
            rotated.verify(&format!("{CANONICAL} "), &signature),
            Verdict::PgpInvalid,
            "{key_type:?}: tampered content still fails"
        );
    }
}

/// Retaining retired keys must not make verification permissive: a signature
/// matching neither the active key nor any retired one still fails, and so does
/// tampered content.
#[test]
fn a_retired_certificate_does_not_make_verification_permissive() {
    let (_, old_public) = generate_key_pair();
    let (new_secret, _) = generate_key_pair();
    let (stranger_secret, _) = generate_key_pair();

    let rotated = signer_with_retired(&new_secret, &[&old_public]);

    let foreign = signer_with_retired(&stranger_secret, &[])
        .sign(CANONICAL)
        .expect("sign with an unrelated key");
    assert_eq!(
        rotated.verify(CANONICAL, &foreign),
        Verdict::PgpInvalid,
        "a signature by a key in neither the active nor the retired set must fail"
    );

    let own = rotated.sign(CANONICAL).expect("sign with the active key");
    assert_eq!(
        rotated.verify(&format!("{CANONICAL} "), &own),
        Verdict::PgpInvalid,
        "tampered content must fail even though the signing key is trusted"
    );
}

/// A stored signature a classical certificate's signing subkey made verifies
/// against that certificate kept as retired: verification walks the subkeys.
#[test]
fn a_stored_subkey_signature_verifies_against_its_retired_certificate() {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(false)
        .primary_user_id("ferroehr-signing subkey <subkey@ferroehr.local>".into())
        .subkey(
            SubkeyParamsBuilder::default()
                .key_type(KeyType::Ed25519Legacy)
                .can_sign(true)
                .passphrase(None)
                .build()
                .expect("build subkey params"),
        );
    let key: SignedSecretKey = builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key");
    let signature = DetachedSignature::sign_binary_data(
        OsRng,
        &key.secret_subkeys[0].key,
        &Password::empty(),
        HashAlgorithm::Sha256,
        CANONICAL.as_bytes(),
    )
    .expect("a subkey signature")
    .to_armored_string(ArmorOptions::default())
    .expect("armor the signature");
    let (new_secret, _) = generate_key_pair();
    let rotated = signer_with_retired(&new_secret, &[&classical_certificate(&key)]);
    assert_eq!(rotated.verify(CANONICAL, &signature), Verdict::PgpValid);
    assert_eq!(
        rotated.verify(&format!("{CANONICAL} "), &signature),
        Verdict::PgpInvalid
    );
}
