// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenPGP`-mode integration tests: sign→verify round-trip with a
//! generated key, tamper detection, armor-parse failures, the fail-closed
//! boot validation (missing path, garbled key, wrong passphrase), the
//! boot advisory an RSA signing key raises, the refusal of ECDSA and
//! Ed448 signing keys (RUSTSEC-2026-0387), and the choice of the signing
//! component, subkey or primary key.
//!
//! Keys are generated in-test via rPGP so there is no vendored key fixture.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
use ferroehr::versioning::signature::key::KeyError;
use ferroehr::versioning::signature::signer::{Signer, SigningError};
use ferroehr::versioning::signature::verify::Verdict;
use pgp::composed::{
    ArmorOptions, Deserializable as _, DetachedSignature, KeyType, SecretKeyParamsBuilder,
    SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{KeyFlags, SignatureConfig, SignatureType, Subpacket, SubpacketData};
use pgp::types::{Duration, KeyDetails as _, KeyVersion, Password, Tag, Timestamp};
use rand::rngs::OsRng;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// A short openEHR-canonical-form-like string to sign in the tests.
const CANONICAL: &str = r#"{"_type":"ORIGINAL_VERSION","uid":{"value":"a::b::1"}}"#;

/// Generate an armored Ed25519 `OpenPGP` secret key, optionally passphrase-locked.
fn generate_key(passphrase: Option<&str>) -> String {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_sign(true)
        .primary_user_id("ferroehr-signing test <test@ferroehr.local>".into());
    if let Some(p) = passphrase {
        builder.passphrase(Some(p.to_owned()));
    }
    let params = builder.build().expect("build key params");
    let key: SignedSecretKey = params.generate(OsRng).expect("generate key");
    key.to_armored_string(ArmorOptions::default())
        .expect("armor key")
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
        key_path: Some(temp_file(armored_key)),
        key_passphrase: passphrase.map(ferroehr::config::secret::Secret::new),
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
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
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
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
    // A passphrase-locked key loaded with the wrong passphrase cannot sign →
    // the boot-time test signature fails → Unusable (fail-closed).
    let key = generate_key(Some("the-right-one"));
    let err = pgp_signer(&key, Some("the-wrong-one")).expect_err("wrong passphrase must fail");
    assert!(
        matches!(err, SigningError::Key(KeyError::Unusable(_))),
        "got {err:?}"
    );
}

#[test]
fn boot_fails_on_missing_key_file() {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_path: Some(PathBuf::from("/nonexistent/ferroehr-signing/key.asc")),
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
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
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
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

/// Generate a key and return `(armored secret, armored PUBLIC certificate)` —
/// the public half is what a retired key is configured as.
fn generate_key_pair() -> (String, String) {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_sign(true)
        .primary_user_id("ferroehr-signing rotation <rotation@ferroehr.local>".into());
    let params = builder.build().expect("build key params");
    let key: SignedSecretKey = params.generate(OsRng).expect("generate key");
    let secret = key
        .to_armored_string(ArmorOptions::default())
        .expect("armor secret key");
    let public = key
        .to_public_key()
        .to_armored_string(ArmorOptions::default())
        .expect("armor public certificate");
    (secret, public)
}

/// A `pgp`-mode signer whose `active` key signs and whose `retired` public
/// certificates are kept for verification only.
fn signer_with_retired(active: &str, retired: &[&str]) -> Signer {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_path: Some(temp_file(active)),
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: retired.iter().map(|cert| temp_file(cert)).collect(),
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

/// Generate a certificate whose SIGNING happens on a subkey, returning
/// `(armored secret, armored PUBLIC certificate)`.
///
/// This is the OpenPGP-native rotation shape (RFC 9580 §5.2.1.8): the primary
/// key certifies, a subkey signs, and rotating the subkey leaves the
/// certificate — and therefore every past signature — intact.
fn generate_key_pair_with_signing_subkey() -> (String, String) {
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
    let params = builder.build().expect("build key params");
    let key: SignedSecretKey = params.generate(OsRng).expect("generate key");
    let secret = key
        .to_armored_string(ArmorOptions::default())
        .expect("armor secret key");
    let public = key
        .to_public_key()
        .to_armored_string(ArmorOptions::default())
        .expect("armor public certificate");
    (secret, public)
}

/// Signing goes through a signing-capable SUBKEY when the certificate has one,
/// and the result verifies — which requires the verifier to walk subkeys, since
/// `rpgp` checks only the primary key by default.
#[test]
fn a_certificate_with_a_signing_subkey_signs_and_verifies_through_it() {
    let (secret, _) = generate_key_pair_with_signing_subkey();
    let signer = signer_with_retired(&secret, &[]);

    let signature = signer.sign(CANONICAL).expect("sign via the signing subkey");
    assert_eq!(
        signer.verify(CANONICAL, &signature),
        Verdict::PgpValid,
        "a subkey signature must verify against its own certificate"
    );
    assert_eq!(
        signer.verify(&format!("{CANONICAL} "), &signature),
        Verdict::PgpInvalid,
        "tampered content must still fail when the signature came from a subkey"
    );
}

/// The point of subkey signing: a signature made before a rotation verifies
/// afterwards with **no** retired-keyring entry, because the certificate itself
/// carries both subkeys.
///
/// The retired keyring stays for the case a whole certificate is replaced; this
/// is the ordinary path, and it needs no operator ceremony at all.
#[test]
fn a_subkey_signature_verifies_against_the_certificate_that_retains_it() {
    let (secret, public) = generate_key_pair_with_signing_subkey();
    let signature = signer_with_retired(&secret, &[])
        .sign(CANONICAL)
        .expect("sign via the signing subkey");

    // A verifier configured with a DIFFERENT active key still accepts it once
    // the signing certificate is present — here as a retired entry, which is
    // how a replaced certificate is carried.
    let (other_secret, _) = generate_key_pair();
    let rotated = signer_with_retired(&other_secret, &[&public]);
    assert_eq!(
        rotated.verify(CANONICAL, &signature),
        Verdict::PgpValid,
        "a subkey signature must verify against a retained certificate, not just the active one"
    );
}

// ── The RSA signing-key boot advisory ───────────────────────────────────────

/// One captured `tracing` event: its level and its rendered field values.
#[derive(Debug, Clone)]
struct CapturedEvent {
    level: tracing::Level,
    fields: BTreeMap<String, String>,
}

/// A layer collecting every event emitted while it is the default subscriber.
#[derive(Clone, Default)]
struct EventCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S: tracing::Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().expect("lock").push(CapturedEvent {
            level: *event.metadata().level(),
            fields: visitor.0,
        });
    }
}

/// Collects field name → rendered value.
#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

/// Build a `pgp`-mode signer with a capturing subscriber installed, returning
/// every event the key load emitted.
fn boot_events(armored_key: &str) -> Vec<CapturedEvent> {
    let capture = EventCapture::default();
    let events = Arc::clone(&capture.events);
    let subscriber = tracing_subscriber::registry().with(capture);
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        pgp_signer(armored_key, None).expect("build signer");
    }
    events.lock().expect("lock").clone()
}

/// Generate an armored RSA-2048 `OpenPGP` secret key (rPGP refuses anything
/// smaller as insecure).
fn generate_rsa_key() -> String {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Rsa(2048))
        .can_sign(true)
        .primary_user_id("ferroehr-signing rsa <rsa@ferroehr.local>".into());
    let params = builder.build().expect("build RSA key params");
    let key: SignedSecretKey = params.generate(OsRng).expect("generate RSA key");
    key.to_armored_string(ArmorOptions::default())
        .expect("armor RSA key")
}

/// An RSA signing key BOOTS — the operator with an RSA-signed corpus keeps a
/// read path — and raises a prominent warning naming the advisory and the
/// remedy.
#[test]
fn an_rsa_signing_key_boots_with_a_prominent_advisory() {
    let warnings: Vec<CapturedEvent> = boot_events(&generate_rsa_key())
        .into_iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();
    let advisory = warnings
        .iter()
        .find(|e| {
            e.fields
                .get("advisory")
                .is_some_and(|a| a == "RUSTSEC-2023-0071")
        })
        .unwrap_or_else(|| panic!("no RSA advisory among the boot events: {warnings:?}"));
    let message = advisory.fields.get("message").expect("a message field");
    assert!(
        message.contains("RSA"),
        "the warning names the key algorithm: {message}"
    );
    assert!(
        message.contains("Ed25519"),
        "the warning names the recommended replacement: {message}"
    );
    assert!(
        message.contains("retired_key_paths"),
        "the warning points at the rotation mechanism that keeps history verifying: {message}"
    );
}

/// An Ed25519 key keeps `rsa` off the signing path entirely, so it raises no
/// advisory at all.
#[test]
fn an_ed25519_signing_key_raises_no_advisory() {
    let advisories: Vec<CapturedEvent> = boot_events(&generate_key(None))
        .into_iter()
        .filter(|e| e.fields.contains_key("advisory"))
        .collect();
    assert!(
        advisories.is_empty(),
        "an ECC signing key must boot silently, got {advisories:?}"
    );
}

// ── ECDSA and Ed448 signing keys (RUSTSEC-2026-0387) ────────────────────────

/// Generate a `key_type` signing key, returning the key and its armored secret.
fn generate_typed_key(key_type: KeyType) -> (SignedSecretKey, String) {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(key_type)
        .can_sign(true)
        .primary_user_id("ferroehr-signing typed <typed@ferroehr.local>".into());
    let params = builder.build().expect("build key params");
    let key: SignedSecretKey = params.generate(OsRng).expect("generate key");
    let armored = key
        .to_armored_string(ArmorOptions::default())
        .expect("armor secret key");
    (key, armored)
}

/// The signing keys the server refuses, with the algorithm each reports.
fn refused_key_types() -> Vec<(KeyType, PublicKeyAlgorithm)> {
    vec![
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

/// An ECDSA or Ed448 signing key stops the boot with an error naming the
/// advisory and the replacement, before any signature is made with it.
#[test]
fn an_ecdsa_or_ed448_signing_key_is_refused_at_boot() {
    for (key_type, expected) in refused_key_types() {
        let (_, armored) = generate_typed_key(key_type.clone());
        let err = pgp_signer(&armored, None).expect_err("the key must be refused");
        let SigningError::Key(KeyError::RefusedAlgorithm { algorithm }) = &err else {
            panic!("{key_type:?}: expected RefusedAlgorithm, got {err:?}");
        };
        assert_eq!(*algorithm, expected, "{key_type:?}");
        let message = KeyError::RefusedAlgorithm {
            algorithm: *algorithm,
        }
        .to_string();
        for needle in ["RUSTSEC-2026-0387", "Ed25519", "retired_key_paths"] {
            assert!(message.contains(needle), "{key_type:?}: {message}");
        }
    }
}

/// A refused key's certificate, listed as retired, still verifies a version it
/// signed before the upgrade, and still fails tampered content.
#[test]
fn a_refused_key_still_verifies_its_signatures_as_a_retired_key() {
    for (key_type, _) in refused_key_types() {
        let (old_key, _) = generate_typed_key(key_type.clone());
        // NOTE: RFC 9580 §5.2.3.2; a digest shorter than the curve fails, and rPGP holds Ed448 to 512 bits.
        let old_signature = DetachedSignature::sign_binary_data(
            OsRng,
            &old_key.primary_key,
            &Password::empty(),
            HashAlgorithm::Sha512,
            CANONICAL.as_bytes(),
        )
        .expect("sign with the pre-upgrade key")
        .to_armored_string(ArmorOptions::default())
        .expect("armor the signature");
        let old_public = old_key
            .to_public_key()
            .to_armored_string(ArmorOptions::default())
            .expect("armor public certificate");

        let (new_secret, _) = generate_key_pair();
        let rotated = signer_with_retired(&new_secret, &[&old_public]);
        assert_eq!(
            rotated.verify(CANONICAL, &old_signature),
            Verdict::PgpValid,
            "{key_type:?}: a retired certificate must verify its own signature"
        );
        assert_eq!(
            rotated.verify(&format!("{CANONICAL} "), &old_signature),
            Verdict::PgpInvalid,
            "{key_type:?}: tampered content must still fail"
        );
    }
}

/// Ed25519 signs and verifies in both encodings: the legacy `EdDSA` one and
/// the RFC 9580 one.
#[test]
fn an_ed25519_signing_key_still_signs() {
    for key_type in [KeyType::Ed25519Legacy, KeyType::Ed25519] {
        let (_, armored) = generate_typed_key(key_type.clone());
        let signer = pgp_signer(&armored, None)
            .unwrap_or_else(|err| panic!("{key_type:?} must load: {err:?}"));
        let signature = signer.sign(CANONICAL).expect("sign");
        assert_eq!(
            signer.verify(CANONICAL, &signature),
            Verdict::PgpValid,
            "{key_type:?}"
        );
    }
}

// ── Choosing the signing subkey (RFC 9580 §5.2.1.12, §5.2.3.10, §5.2.3.13) ──

/// A certificate whose primary only certifies and which carries two Ed25519
/// signing subkeys: index 0 created long ago, index 1 created recently.
fn certificate_with_two_signing_subkeys() -> SignedSecretKey {
    let now = Timestamp::now().as_secs();
    let subkey = |created: u32| {
        SubkeyParamsBuilder::default()
            .key_type(KeyType::Ed25519Legacy)
            .can_sign(true)
            .created_at(Timestamp::from_secs(created))
            .passphrase(None)
            .build()
            .expect("build subkey params")
    };
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(false)
        .primary_user_id("ferroehr-signing rotation <rotation@ferroehr.local>".into())
        .subkey(subkey(now - 1_000_000))
        .subkey(subkey(now - 100));
    builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key")
}

/// Adds a signature of `typ` by the primary over subkey `index`, created one
/// second from now so it is the newest, with `extra` hashed subpackets.
fn add_subkey_signature(
    key: &mut SignedSecretKey,
    index: usize,
    typ: SignatureType,
    extra: Vec<SubpacketData>,
) {
    let created = Timestamp::from_secs(Timestamp::now().as_secs() + 1);
    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, typ).expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(created),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
    ]
    .into_iter()
    .chain(extra)
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let primary = &key.primary_key;
    let subkey = key
        .secret_subkeys
        .get_mut(index)
        .expect("the subkey exists");
    let signature = config
        .sign_subkey_binding(
            primary,
            primary.public_key(),
            &Password::empty(),
            subkey.key.public_key(),
        )
        .expect("sign over the subkey");
    subkey.signatures.push(signature);
}

/// Loads `key` as the signing key, signs, and returns the index of the
/// subkey whose public half verifies the signature.
fn subkey_that_signs(key: &SignedSecretKey) -> usize {
    let armored = key
        .to_armored_string(ArmorOptions::default())
        .expect("armor secret key");
    let signature = pgp_signer(&armored, None)
        .expect("build signer")
        .sign(CANONICAL)
        .expect("sign");
    let (signature, _) = DetachedSignature::from_string(&signature).expect("parse signature");
    let public = key.to_public_key();
    let signers: Vec<usize> = public
        .public_subkeys
        .iter()
        .enumerate()
        .filter(|(_, subkey)| signature.verify(*subkey, CANONICAL.as_bytes()).is_ok())
        .map(|(index, _)| index)
        .collect();
    assert_eq!(signers.len(), 1, "exactly one subkey signs: {signers:?}");
    *signers.first().expect("one signing subkey")
}

/// Of two valid signing subkeys the one created last signs, even though it
/// comes second in the file: adding a signing subkey rotates to it.
#[test]
fn the_newest_signing_subkey_signs() {
    assert_eq!(
        subkey_that_signs(&certificate_with_two_signing_subkeys()),
        1
    );
}

/// A revoked subkey is not to be used (RFC 9580 §5.2.1.12), however new.
#[test]
fn a_revoked_signing_subkey_is_not_used() {
    let mut key = certificate_with_two_signing_subkeys();
    add_subkey_signature(&mut key, 1, SignatureType::SubkeyRevocation, Vec::new());
    assert_eq!(subkey_that_signs(&key), 0);
}

/// A subkey whose newest binding sets a key lifetime that has run out is not
/// used (RFC 9580 §5.2.3.13, counted from the subkey's creation).
#[test]
fn an_expired_signing_subkey_is_not_used() {
    let mut key = certificate_with_two_signing_subkeys();
    let mut flags = KeyFlags::default();
    flags.set_sign(true);
    add_subkey_signature(
        &mut key,
        1,
        SignatureType::SubkeyBinding,
        vec![
            SubpacketData::KeyFlags(flags),
            SubpacketData::KeyExpirationTime(Duration::from_secs(10)),
        ],
    );
    assert_eq!(subkey_that_signs(&key), 0);
}

/// Only the newest binding counts (RFC 9580 §5.2.3.10): one that drops the
/// signing flag takes the subkey out of signing, though an older binding
/// grants it.
#[test]
fn a_subkey_whose_newest_binding_drops_signing_is_not_used() {
    let mut key = certificate_with_two_signing_subkeys();
    add_subkey_signature(
        &mut key,
        1,
        SignatureType::SubkeyBinding,
        vec![SubpacketData::KeyFlags(KeyFlags::default())],
    );
    assert_eq!(subkey_that_signs(&key), 0);
}

// ── Falling back to the primary key (RFC 9580 §5.2.1.11, §5.2.3.10, §5.2.3.29) ──

/// A single-key certificate, no subkeys, whose primary key may sign when
/// `can_sign`, created `age` seconds ago.
fn primary_only_certificate(can_sign: bool, age: u32) -> SignedSecretKey {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(can_sign)
        .created_at(Timestamp::from_secs(Timestamp::now().as_secs() - age))
        .primary_user_id("ferroehr-signing primary <primary@ferroehr.local>".into());
    builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key")
}

/// The armored secret key of `key`.
fn armored(key: &SignedSecretKey) -> String {
    key.to_armored_string(ArmorOptions::default())
        .expect("armor secret key")
}

/// Asserts that loading `key` stops the boot because no key may sign.
fn assert_no_signing_component(key: &SignedSecretKey) {
    let err = pgp_signer(&armored(key), None).expect_err("the certificate must be refused");
    assert!(
        matches!(err, SigningError::Key(KeyError::NoSigningComponent)),
        "got {err:?}"
    );
}

/// A primary key whose self-signature lacks the signing flag, with no subkey
/// to sign instead, is refused at boot: it is not authorised to sign data.
#[test]
fn a_primary_key_without_the_signing_flag_is_refused_at_boot() {
    assert_no_signing_component(&primary_only_certificate(false, 0));
    let message = KeyError::NoSigningComponent.to_string();
    for needle in ["0x02", "RFC 9580", "signing subkey"] {
        assert!(message.contains(needle), "{message}");
    }
}

/// The case the fallback hid: a certify-only primary whose one signing
/// subkey is revoked has nothing left that may sign.
#[test]
fn a_certify_only_primary_is_not_used_when_its_signing_subkey_is_revoked() {
    let (secret, _) = generate_key_pair_with_signing_subkey();
    let (mut key, _) = SignedSecretKey::from_string(&secret).expect("parse key");
    add_subkey_signature(&mut key, 0, SignatureType::SubkeyRevocation, Vec::new());
    assert_no_signing_component(&key);
}

/// A primary key whose User ID self-signature carries the signing flag signs,
/// and the signature verifies against the primary key itself.
#[test]
fn a_primary_key_with_the_signing_flag_signs() {
    let key = primary_only_certificate(true, 0);
    let signature = pgp_signer(&armored(&key), None)
        .expect("build signer")
        .sign(CANONICAL)
        .expect("sign");
    let (signature, _) = DetachedSignature::from_string(&signature).expect("parse signature");
    signature
        .verify(&key.to_public_key(), CANONICAL.as_bytes())
        .expect("the primary key made the signature");
}

/// A version 6 key keeps the primary key's flags in its Direct Key signature
/// (RFC 9580 §5.2.3.10), and the primary key signs from there.
#[test]
fn a_version_6_primary_key_signs_from_its_direct_key_signature() {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .version(KeyVersion::V6)
        .key_type(KeyType::Ed25519)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id("ferroehr-signing v6 <v6@ferroehr.local>".into());
    let key = builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key");
    assert!(
        !key.details.direct_signatures.is_empty(),
        "the v6 fixture has a Direct Key signature"
    );
    let signer = pgp_signer(&armored(&key), None).expect("build signer");
    let signature = signer.sign(CANONICAL).expect("sign");
    assert_eq!(signer.verify(CANONICAL, &signature), Verdict::PgpValid);
}

/// A primary key whose newest User ID self-signature sets a key lifetime that
/// has run out (RFC 9580 §5.2.3.13, counted from the primary key's creation)
/// is refused, though an older self-signature granted signing.
#[test]
fn an_expired_primary_key_is_refused() {
    let mut key = primary_only_certificate(true, 1_000);
    let mut flags = KeyFlags::default();
    flags.set_certify(true);
    flags.set_sign(true);
    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, SignatureType::CertPositive)
            .expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(Timestamp::from_secs(Timestamp::now().as_secs() + 1)),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
        SubpacketData::KeyFlags(flags),
        SubpacketData::IsPrimary(true),
        SubpacketData::KeyExpirationTime(Duration::from_secs(10)),
    ]
    .into_iter()
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let user = key.details.users.first_mut().expect("the primary User ID");
    let signature = config
        .sign_certification(
            &key.primary_key,
            key.primary_key.public_key(),
            &Password::empty(),
            Tag::UserId,
            &user.id,
        )
        .expect("sign the User ID");
    user.signatures.push(signature);
    assert_no_signing_component(&key);
}

/// A revoked key is not to be used (RFC 9580 §5.2.1.11), signing flag or not.
#[test]
fn a_revoked_primary_key_is_refused() {
    let mut key = primary_only_certificate(true, 0);
    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, SignatureType::KeyRevocation)
            .expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(Timestamp::now()),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
    ]
    .into_iter()
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let revocation = config
        .sign_key(
            &key.primary_key,
            &Password::empty(),
            key.primary_key.public_key(),
        )
        .expect("sign the revocation");
    key.details.revocation_signatures.push(revocation);
    assert_no_signing_component(&key);
}

// ── A subkey under a revoked or expired primary key (RFC 9580 §5.2.1.11, §5.2.3.13) ──

/// A certify-only primary key, created long ago, with one valid Ed25519
/// signing subkey.
fn certificate_with_a_signing_subkey() -> SignedSecretKey {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(false)
        .created_at(Timestamp::from_secs(Timestamp::now().as_secs() - 1_000))
        .primary_user_id("ferroehr-signing standing <standing@ferroehr.local>".into())
        .subkey(
            SubkeyParamsBuilder::default()
                .key_type(KeyType::Ed25519Legacy)
                .can_sign(true)
                .passphrase(None)
                .build()
                .expect("build subkey params"),
        );
    builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key")
}

/// The fixture's signing subkey signs while its primary key stands, so the
/// two refusals below are caused by the primary key alone.
#[test]
fn a_signing_subkey_under_a_valid_primary_key_signs() {
    assert_eq!(subkey_that_signs(&certificate_with_a_signing_subkey()), 0);
}

/// A revoked key is not to be used (RFC 9580 §5.2.1.11), and its signing
/// subkey with it: the certificate is refused at boot.
#[test]
fn a_signing_subkey_under_a_revoked_primary_key_is_refused_at_boot() {
    let mut key = certificate_with_a_signing_subkey();
    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, SignatureType::KeyRevocation)
            .expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(Timestamp::now()),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
    ]
    .into_iter()
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let revocation = config
        .sign_key(
            &key.primary_key,
            &Password::empty(),
            key.primary_key.public_key(),
        )
        .expect("sign the revocation");
    key.details.revocation_signatures.push(revocation);
    assert_no_signing_component(&key);
}

/// A primary key past its Key Expiration Time (RFC 9580 §5.2.3.13, counted
/// from its creation) takes its signing subkey out of signing too.
#[test]
fn a_signing_subkey_under_an_expired_primary_key_is_refused_at_boot() {
    let mut key = certificate_with_a_signing_subkey();
    let mut flags = KeyFlags::default();
    flags.set_certify(true);
    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, SignatureType::CertPositive)
            .expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(Timestamp::from_secs(Timestamp::now().as_secs() + 1)),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
        SubpacketData::KeyFlags(flags),
        SubpacketData::IsPrimary(true),
        SubpacketData::KeyExpirationTime(Duration::from_secs(10)),
    ]
    .into_iter()
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let user = key.details.users.first_mut().expect("the primary User ID");
    let signature = config
        .sign_certification(
            &key.primary_key,
            key.primary_key.public_key(),
            &Password::empty(),
            Tag::UserId,
            &user.id,
        )
        .expect("sign the User ID");
    user.signatures.push(signature);
    assert_no_signing_component(&key);
}

// ── A Certification Revocation of a Direct Key signature (RFC 9580 §5.2.1.13) ──

/// A version 6 key whose primary key may sign only through its Direct Key
/// signature (RFC 9580 §5.2.3.10).
fn version_6_signing_key() -> SignedSecretKey {
    let mut builder = SecretKeyParamsBuilder::default();
    builder
        .version(KeyVersion::V6)
        .key_type(KeyType::Ed25519)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id("ferroehr-signing v6 <v6@ferroehr.local>".into());
    builder
        .build()
        .expect("build key params")
        .generate(OsRng)
        .expect("generate key")
}

/// A Certification Revocation (Type ID 0x30) by the primary key over the
/// primary key alone, created at `created`, the data a Direct Key signature
/// covers (RFC 9580 §5.2.1.13, §5.2.4). rPGP's `sign_key` refuses the type,
/// so the hash is built here. `tamper` flips a hashed octet after signing,
/// which leaves a signature that does not verify.
fn direct_key_revocation(
    key: &SignedSecretKey,
    created: Timestamp,
    tamper: bool,
) -> pgp::packet::Signature {
    use pgp::packet::SignatureVersionSpecific;
    use pgp::ser::Serialize as _;
    use pgp::types::SigningKey as _;

    let mut config =
        SignatureConfig::from_key(OsRng, &key.primary_key, SignatureType::CertRevocation)
            .expect("signature config");
    config.hashed_subpackets = [
        SubpacketData::SignatureCreationTime(created),
        SubpacketData::IssuerFingerprint(key.primary_key.fingerprint()),
    ]
    .into_iter()
    .map(|data| Subpacket::regular(data).expect("subpacket"))
    .collect();
    let mut hasher = config.hash_alg.new_hasher().expect("hasher");
    if let SignatureVersionSpecific::V6 { salt } = &config.version_specific {
        hasher.update(salt.as_ref());
    }
    let body = key.primary_key.public_key().to_bytes().expect("key body");
    hasher.update(&[0x9B]);
    hasher.update(&u32::try_from(body.len()).expect("key length").to_be_bytes());
    hasher.update(&body);
    let len = config
        .hash_signature_data(&mut hasher)
        .expect("hashed data");
    hasher.update(&config.trailer(len).expect("trailer"));
    let hash = hasher.finalize().into_vec();
    let signature = key
        .primary_key
        .sign(&Password::empty(), config.hash_alg, &hash)
        .expect("sign the revocation");
    let [first, second]: [u8; 2] = hash
        .get(..2)
        .and_then(|leading| leading.try_into().ok())
        .expect("a digest of at least two octets");
    let signed_hash = [first, second ^ u8::from(tamper)];
    pgp::packet::Signature::from_config(config, signed_hash, signature).expect("revocation")
}

/// A valid Certification Revocation of the Direct Key signature, created
/// after it, cancels it (RFC 9580 §5.2.3.10, "Revoking a Direct Key
/// signature cancels that signature"), and a version 6 primary key without a
/// Direct Key signature in force may not sign.
#[test]
fn a_revoked_direct_key_signature_no_longer_lets_the_primary_key_sign() {
    let mut key = version_6_signing_key();
    let created = Timestamp::from_secs(Timestamp::now().as_secs() + 1);
    let revocation = direct_key_revocation(&key, created, false);
    key.details.direct_signatures.push(revocation);
    assert_no_signing_component(&key);
}

/// A revocation created before the Direct Key signature, or one that does not
/// verify, cancels nothing: the primary key still signs.
#[test]
fn an_older_or_invalid_revocation_leaves_the_direct_key_signature_in_force() {
    for (created, tamper) in [
        (
            Timestamp::from_secs(Timestamp::now().as_secs() - 3_600),
            false,
        ),
        (Timestamp::from_secs(Timestamp::now().as_secs() + 1), true),
    ] {
        let mut key = version_6_signing_key();
        let revocation = direct_key_revocation(&key, created, tamper);
        key.details.direct_signatures.push(revocation);
        let signer = pgp_signer(&armored(&key), None).expect("build signer");
        let signature = signer.sign(CANONICAL).expect("sign");
        assert_eq!(signer.verify(CANONICAL, &signature), Verdict::PgpValid);
    }
}
