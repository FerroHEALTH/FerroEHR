// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chain check: embedded primary → bundled signing subkey → signature →
//! licence window.
//!
//! Every step is a typed refusal. A caller that only wants "licensed or not"
//! maps the whole error enum to "not"; a caller that logs gets the exact
//! reason without string matching.

use jiff::civil::Date;
use pgp::composed::SignedPublicSubKey;
use pgp::packet::{PublicKey, SignatureType};
use pgp::types::{Fingerprint, KeyDetails as _};

use crate::licence::document::{Licence, Window};
use crate::licence::token::Token;

/// Why a structurally valid token was refused.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// A binding or self-signature inside the bundled certificate does not
    /// verify under its own primary key.
    #[error("bundled certificate has broken bindings")]
    BrokenBindings(#[source] pgp::errors::Error),
    /// The certificate's primary key is not one the verifier embeds.
    #[error("certificate primary {0} is not a trusted issuer")]
    UntrustedPrimary(Fingerprint),
    /// The signed block carries no signature at all.
    #[error("signed licence block carries no signature")]
    NoSignature,
    /// The signature names no issuer key, so no subkey can be selected.
    #[error("signature names no issuer")]
    NoIssuer,
    /// The signature's issuer is not a subkey of the bundled certificate.
    #[error("signature issuer {0} is not a subkey of the bundled certificate")]
    UnknownSigningSubkey(Fingerprint),
    /// The issuing subkey's binding does not grant the signing capability.
    #[error("subkey {0} is not bound for signing")]
    SubkeyNotSigningCapable(Fingerprint),
    /// The signature carries no creation time.
    #[error("signature carries no creation time")]
    NoSigningTime,
    /// The signature was made after the subkey's binding had expired.
    #[error("subkey {subkey} expired at {expired_at} but signed at {signed_at}")]
    SubkeyExpiredAtSigning {
        /// The issuing subkey.
        subkey: Fingerprint,
        /// When its binding expired.
        expired_at: jiff::Timestamp,
        /// When the licence was signed.
        signed_at: jiff::Timestamp,
    },
    /// The signature does not verify over the signed text.
    #[error("licence signature does not verify")]
    BadSignature(#[source] pgp::errors::Error),
    /// The signed text is not a licence document.
    #[error("signed text is not a licence document")]
    Payload(#[source] serde_json::Error),
    /// A time value in the certificate or signature is outside what `jiff`
    /// can represent.
    #[error("time value out of range")]
    Time(#[source] jiff::Error),
    /// The window has not opened yet.
    #[error("licence is not valid before {not_before}")]
    NotYetValid {
        /// First valid day.
        not_before: Date,
    },
    /// The window has closed.
    #[error("licence expired after {not_after}")]
    Expired {
        /// Last valid day.
        not_after: Date,
    },
}

/// A token that passed every check.
#[derive(Debug, Clone)]
pub struct Verified {
    /// The licence the token grants.
    pub licence: Licence,
    /// The trusted primary the certificate chains to.
    pub primary: Fingerprint,
    /// The subkey that signed.
    pub signing_subkey: Fingerprint,
    /// When it signed.
    pub signed_at: jiff::Timestamp,
    /// When the signing subkey's binding expires, if it does.
    pub subkey_expires_at: Option<jiff::Timestamp>,
}

/// Verify `token` against the trusted primary keys `anchors`, as of `today`.
///
/// Trust is decided on the full primary key packet, not on a fingerprint
/// string: the bundled certificate's primary must equal an anchor byte for
/// byte, and only then are its bindings and the licence signature checked
/// under it.
///
/// # Errors
/// [`VerifyError`], naming the first check that failed.
pub fn verify(token: &Token, anchors: &[PublicKey], today: Date) -> Result<Verified, VerifyError> {
    let certificate = token.certificate();
    let primary = certificate.primary_key.fingerprint();
    if !anchors.contains(&certificate.primary_key) {
        return Err(VerifyError::UntrustedPrimary(primary));
    }
    certificate
        .verify_bindings()
        .map_err(VerifyError::BrokenBindings)?;

    let signature = token
        .message()
        .signatures()
        .first()
        .ok_or(VerifyError::NoSignature)?;
    let subkey = issuing_subkey(certificate.public_subkeys.as_slice(), signature)?;
    let subkey_fingerprint = subkey.key.fingerprint();

    let binding = subkey
        .signatures
        .iter()
        .find(|s| s.typ() == Some(SignatureType::SubkeyBinding))
        .ok_or_else(|| VerifyError::SubkeyNotSigningCapable(subkey_fingerprint.clone()))?;
    if !binding.key_flags().sign() {
        return Err(VerifyError::SubkeyNotSigningCapable(subkey_fingerprint));
    }

    let signed_at = to_jiff(signature.created().ok_or(VerifyError::NoSigningTime)?)?;
    let subkey_expires_at = expiry(subkey.key.created_at(), binding.key_expiration_time())?;
    if let Some(expired_at) = subkey_expires_at
        && signed_at >= expired_at
    {
        return Err(VerifyError::SubkeyExpiredAtSigning {
            subkey: subkey_fingerprint,
            expired_at,
            signed_at,
        });
    }

    token
        .message()
        .verify(subkey)
        .map_err(VerifyError::BadSignature)?;

    let licence = Licence::from_json(&token.signed_text()).map_err(VerifyError::Payload)?;
    match licence.window(today) {
        Window::NotYetValid => Err(VerifyError::NotYetValid {
            not_before: licence.not_before,
        }),
        Window::Expired => Err(VerifyError::Expired {
            not_after: licence.not_after,
        }),
        Window::Active => Ok(Verified {
            licence,
            primary,
            signing_subkey: subkey_fingerprint,
            signed_at,
            subkey_expires_at,
        }),
    }
}

/// The subkey the signature names as its issuer, by fingerprint first and by
/// 64-bit key id as the v4 fallback.
fn issuing_subkey<'c>(
    subkeys: &'c [SignedPublicSubKey],
    signature: &pgp::packet::Signature,
) -> Result<&'c SignedPublicSubKey, VerifyError> {
    let fingerprints = signature.issuer_fingerprint();
    if let Some(named) = fingerprints.first() {
        return subkeys
            .iter()
            .find(|s| s.key.fingerprint() == **named)
            .ok_or_else(|| VerifyError::UnknownSigningSubkey((*named).clone()));
    }
    let key_ids = signature.issuer_key_id();
    let named = key_ids.first().ok_or(VerifyError::NoIssuer)?;
    subkeys
        .iter()
        .find(|s| s.key.legacy_key_id() == **named)
        .ok_or(VerifyError::NoIssuer)
}

/// Absolute expiry of a component created at `created` whose binding carries
/// `lifetime`; `None` when the binding sets no expiry (RFC 9580 §5.2.3.13, a
/// zero lifetime also means none).
fn expiry(
    created: pgp::types::Timestamp,
    lifetime: Option<pgp::types::Duration>,
) -> Result<Option<jiff::Timestamp>, VerifyError> {
    match lifetime {
        Some(d) if d.as_secs() > 0 => {
            let secs = i64::from(created.as_secs()) + i64::from(d.as_secs());
            jiff::Timestamp::from_second(secs)
                .map(Some)
                .map_err(VerifyError::Time)
        }
        _ => Ok(None),
    }
}

fn to_jiff(ts: pgp::types::Timestamp) -> Result<jiff::Timestamp, VerifyError> {
    jiff::Timestamp::from_second(i64::from(ts.as_secs())).map_err(VerifyError::Time)
}

/// An in-process issuer for tests: an Ed25519 certify-only primary with one
/// signing subkey, generated by `rPGP`. Real issuance is the licensor's `GnuPG`;
/// this exists so the verifier's refusals are testable without a keyring.
#[cfg(test)]
pub(crate) mod fixtures {
    use pgp::composed::{
        ArmorOptions, CleartextSignedMessage, KeyType, SecretKeyParamsBuilder, SignedSecretKey,
        SubkeyParamsBuilder,
    };
    use pgp::types::{KeyVersion, Password};
    use rand::rngs::OsRng;

    use crate::licence::document::Licence;
    use crate::licence::token::{Token, assemble};

    pub(crate) struct Issuer {
        pub(crate) secret: SignedSecretKey,
    }

    impl Issuer {
        pub(crate) fn generate() -> Self {
            let subkey = SubkeyParamsBuilder::default()
                .version(KeyVersion::V4)
                .key_type(KeyType::Ed25519Legacy)
                .can_sign(true)
                .build()
                .unwrap();
            let params = SecretKeyParamsBuilder::default()
                .version(KeyVersion::V4)
                .key_type(KeyType::Ed25519Legacy)
                .can_certify(true)
                .can_sign(false)
                .primary_user_id("FerroEHR Licensing (test) <licensing@example.invalid>".into())
                .subkey(subkey)
                .build()
                .unwrap();
            let secret = params.generate(OsRng).unwrap();
            Self { secret }
        }

        pub(crate) fn primary(&self) -> pgp::packet::PublicKey {
            self.secret.to_public_key().primary_key
        }

        pub(crate) fn certificate_armored(&self) -> String {
            self.secret
                .to_public_key()
                .to_armored_string(ArmorOptions::default())
                .unwrap()
        }

        /// Sign `text` with the signing subkey (the production path) or with
        /// the primary (a misuse the verifier must refuse).
        pub(crate) fn sign(&self, text: &str, with_primary: bool) -> String {
            let msg = if with_primary {
                CleartextSignedMessage::sign(
                    OsRng,
                    text,
                    &self.secret.primary_key,
                    &Password::empty(),
                )
            } else {
                let sub = self.secret.secret_subkeys.first().unwrap();
                CleartextSignedMessage::sign(OsRng, text, &sub.key, &Password::empty())
            }
            .unwrap();
            msg.to_armored_string(ArmorOptions::default()).unwrap()
        }

        pub(crate) fn token_text(&self, licence: &Licence) -> String {
            let signed = self.sign(&licence.to_canonical_json().unwrap(), false);
            assemble(&signed, &self.certificate_armored())
        }

        pub(crate) fn token_for(&self, licence: &Licence) -> Token {
            Token::parse(&self.token_text(licence)).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use uuid::Uuid;

    use super::fixtures::Issuer;
    use super::*;
    use crate::licence::document::Use;
    use crate::licence::token::assemble;

    fn licence() -> Licence {
        Licence {
            id: Uuid::now_v7(),
            licensee: "Example Hospital NV".to_owned(),
            issued: date(2026, 9, 11),
            not_before: date(2026, 9, 11),
            not_after: date(2027, 9, 10),
            permitted_use: Use::Commercial,
        }
    }

    #[test]
    fn a_well_formed_token_verifies() {
        let issuer = Issuer::generate();
        let licence = licence();
        let token = issuer.token_for(&licence);
        let verified = verify(&token, &[issuer.primary()], date(2026, 12, 1)).unwrap();
        assert_eq!(verified.licence, licence);
        assert_eq!(verified.primary, issuer.primary().fingerprint());
        assert_ne!(verified.signing_subkey, verified.primary);
        assert!(verified.subkey_expires_at.is_none());
    }

    #[test]
    fn an_unknown_primary_is_refused_before_anything_else() {
        let issuer = Issuer::generate();
        let stranger = Issuer::generate();
        let token = issuer.token_for(&licence());
        let err = verify(&token, &[stranger.primary()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UntrustedPrimary(_)), "{err}");
    }

    #[test]
    fn no_anchors_means_nothing_verifies() {
        let issuer = Issuer::generate();
        let token = issuer.token_for(&licence());
        assert!(verify(&token, &[], date(2026, 12, 1)).is_err());
    }

    #[test]
    fn a_tampered_payload_is_refused() {
        let issuer = Issuer::generate();
        let signed = issuer.sign(&licence().to_canonical_json().unwrap(), false);
        let tampered = signed.replace("Example Hospital NV", "Example Hospital BV");
        let token = Token::parse(&assemble(&tampered, &issuer.certificate_armored())).unwrap();
        let err = verify(&token, &[issuer.primary()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::BadSignature(_)), "{err}");
    }

    #[test]
    fn a_licence_signed_by_the_primary_itself_is_refused() {
        let issuer = Issuer::generate();
        let signed = issuer.sign(&licence().to_canonical_json().unwrap(), true);
        let token = Token::parse(&assemble(&signed, &issuer.certificate_armored())).unwrap();
        let err = verify(&token, &[issuer.primary()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UnknownSigningSubkey(_)), "{err}");
    }

    #[test]
    fn a_certificate_swapped_under_a_valid_signature_is_refused() {
        let issuer = Issuer::generate();
        let other = Issuer::generate();
        let signed = issuer.sign(&licence().to_canonical_json().unwrap(), false);
        let token = Token::parse(&assemble(&signed, &other.certificate_armored())).unwrap();
        let err = verify(&token, &[other.primary()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UnknownSigningSubkey(_)), "{err}");
    }

    #[test]
    fn a_signed_text_that_is_not_a_licence_is_refused() {
        let issuer = Issuer::generate();
        let signed = issuer.sign("{\"hello\": \"world\"}\n", false);
        let token = Token::parse(&assemble(&signed, &issuer.certificate_armored())).unwrap();
        let err = verify(&token, &[issuer.primary()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::Payload(_)), "{err}");
    }

    #[test]
    fn the_window_is_enforced_on_both_sides() {
        let issuer = Issuer::generate();
        let token = issuer.token_for(&licence());
        assert!(matches!(
            verify(&token, &[issuer.primary()], date(2026, 9, 10)).unwrap_err(),
            VerifyError::NotYetValid { .. }
        ));
        assert!(matches!(
            verify(&token, &[issuer.primary()], date(2027, 9, 11)).unwrap_err(),
            VerifyError::Expired { .. }
        ));
    }

    #[test]
    fn expiry_arithmetic_treats_zero_as_none() {
        let created = pgp::types::Timestamp::from_secs(1_000);
        assert_eq!(expiry(created, None).unwrap(), None);
        assert_eq!(
            expiry(created, Some(pgp::types::Duration::from_secs(0))).unwrap(),
            None
        );
        assert_eq!(
            expiry(created, Some(pgp::types::Duration::from_secs(500))).unwrap(),
            Some(jiff::Timestamp::from_second(1_500).unwrap())
        );
    }
}
