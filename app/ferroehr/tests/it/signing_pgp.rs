// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `OpenPGP`-mode integration tests: sign→verify round-trip with a
//! generated key, tamper detection, armor-parse failures, the fail-closed
//! boot validation (missing path, garbled key, wrong passphrase), and the
//! boot advisory an RSA signing key raises.
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
    ArmorOptions, KeyType, SecretKeyParamsBuilder, SignedSecretKey, SubkeyParamsBuilder,
};
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
