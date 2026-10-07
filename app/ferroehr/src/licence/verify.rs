// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chain check: embedded master key → its signature → licence window.
//!
//! Every step is a typed refusal. A caller that only wants "licensed or not"
//! maps the whole error enum to "not"; a caller that logs gets the exact
//! reason without string matching.

use jiff::civil::Date;
use pgp::composed::SignedPublicKey;
use pgp::packet::{PublicKey, Signature, SignatureType};
use pgp::types::{Fingerprint, KeyDetails as _};

use crate::licence::document::{Licence, Window};
use crate::licence::token::Token;

/// Why a structurally valid token was refused.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// A binding or self-signature of the trusted certificate does not
    /// verify under its own primary key.
    #[error("trusted certificate has broken bindings")]
    BrokenBindings(#[source] pgp::errors::Error),
    /// The certificate's primary key is not one the verifier embeds.
    #[error("certificate primary {0} is not a trusted issuer")]
    UntrustedPrimary(Fingerprint),
    /// The signed block carries no signature at all.
    #[error("signed licence block carries no signature")]
    NoSignature,
    /// The signed block carries more than one signature. Every check must
    /// judge the same signature the cryptography verifies, so a token holds
    /// exactly one.
    #[error("signed licence block carries {0} signatures; exactly one is allowed")]
    ExtraSignatures(usize),
    /// The signature names no issuer key.
    #[error("signature names no issuer")]
    NoIssuer,
    /// The signature was made by a key other than the certificate's primary,
    /// a subkey included.
    #[error("signature was not made by the certificate's primary key")]
    UnknownSigner,
    /// The primary's self-signature does not grant the signing capability.
    #[error("primary {0} is not certified for signing")]
    NotSigningCapable(Fingerprint),
    /// The signature carries no creation time.
    #[error("signature carries no creation time")]
    NoSigningTime,
    /// The signature was made after the primary had expired.
    #[error("key {key} expired at {expired_at} but signed at {signed_at}")]
    ExpiredAtSigning {
        /// The signing primary.
        key: Fingerprint,
        /// When its self-signature says it expired.
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
    /// The trusted primary key that signed.
    pub signer: Fingerprint,
    /// When it signed.
    pub signed_at: jiff::Timestamp,
    /// When the signer expires, if its self-signature sets an expiry.
    pub signer_expires_at: Option<jiff::Timestamp>,
}

/// Verify `token` against the trusted certificates `anchors`, as of `today`.
///
/// Trust is decided on the full primary key packet, not on a fingerprint
/// string: the bundled certificate's primary must equal an anchor's byte for
/// byte. From then on only the anchor counts. Whether the key may sign and
/// when it expires are read from the anchor's own self-signature, never from
/// the copy the token carries, so a token cannot bring an older or altered
/// state of the key with it. The licence must carry exactly one signature,
/// made by that primary itself.
///
/// # Errors
/// [`VerifyError`], naming the first check that failed.
pub fn verify(
    token: &Token,
    anchors: &[SignedPublicKey],
    today: Date,
) -> Result<Verified, VerifyError> {
    let bundled = &token.certificate().primary_key;
    let fingerprint = bundled.fingerprint();
    let anchor = anchors
        .iter()
        .find(|a| a.primary_key == *bundled)
        .ok_or_else(|| VerifyError::UntrustedPrimary(fingerprint.clone()))?;
    let primary = &anchor.primary_key;
    anchor
        .verify_bindings()
        .map_err(VerifyError::BrokenBindings)?;

    let signature = match token.message().signatures() {
        [] => return Err(VerifyError::NoSignature),
        [one] => one,
        many => return Err(VerifyError::ExtraSignatures(many.len())),
    };
    signed_by(primary, signature)?;

    let self_signature = newest_self_signature(anchor)
        .ok_or_else(|| VerifyError::NotSigningCapable(fingerprint.clone()))?;
    if !self_signature.key_flags().sign() {
        return Err(VerifyError::NotSigningCapable(fingerprint));
    }

    let signed_at = to_jiff(signature.created().ok_or(VerifyError::NoSigningTime)?)?;
    let signer_expires_at = expiry(primary.created_at(), self_signature.key_expiration_time())?;
    if let Some(expired_at) = signer_expires_at
        && signed_at >= expired_at
    {
        return Err(VerifyError::ExpiredAtSigning {
            key: fingerprint,
            expired_at,
            signed_at,
        });
    }

    let signed_text = token.signed_text();
    signature
        .verify(primary, signed_text.as_bytes())
        .map_err(VerifyError::BadSignature)?;

    let licence = Licence::from_json(&signed_text).map_err(VerifyError::Payload)?;
    match licence.window(today) {
        Window::NotYetValid => Err(VerifyError::NotYetValid {
            not_before: licence.not_before,
        }),
        Window::Expired => Err(VerifyError::Expired {
            not_after: licence.not_after,
        }),
        Window::Active => Ok(Verified {
            licence,
            signer: fingerprint,
            signed_at,
            signer_expires_at,
        }),
    }
}

/// Whether `signature` names `primary` as its issuer, by fingerprint first
/// and by 64-bit key id as the v4 fallback.
fn signed_by(primary: &PublicKey, signature: &Signature) -> Result<(), VerifyError> {
    let named = if let Some(fingerprint) = signature.issuer_fingerprint().first() {
        **fingerprint == primary.fingerprint()
    } else {
        let key_ids = signature.issuer_key_id();
        let key_id = key_ids.first().ok_or(VerifyError::NoIssuer)?;
        **key_id == primary.legacy_key_id()
    };
    if named {
        Ok(())
    } else {
        Err(VerifyError::UnknownSigner)
    }
}

/// The primary's newest self-signature that can carry its key flags and
/// expiry: a direct-key signature or a user id certification (RFC 9580
/// §5.2.3.10).
///
/// Called on a trusted anchor after its bindings verified, so every
/// candidate is the primary's own.
fn newest_self_signature(certificate: &SignedPublicKey) -> Option<&Signature> {
    let users = certificate
        .details
        .users
        .iter()
        .flat_map(|user| user.signatures.iter());
    certificate
        .details
        .direct_signatures
        .iter()
        .chain(users)
        .filter(|s| {
            matches!(
                s.typ(),
                Some(
                    SignatureType::Key
                        | SignatureType::CertGeneric
                        | SignatureType::CertPersona
                        | SignatureType::CertCasual
                        | SignatureType::CertPositive
                )
            )
        })
        .max_by_key(|s| s.created())
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

/// An in-process issuer for tests: an Ed25519 primary that certifies and
/// signs, with one signing subkey beside it, generated by `rPGP`. Real
/// issuance is the licensor's `GnuPG` on a `YubiKey`; this exists so the
/// verifier's refusals are testable without a keyring.
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

    /// Which key of the issuer makes a signature.
    #[derive(Debug, Clone, Copy)]
    pub(crate) enum By {
        /// The primary: the production path.
        Primary,
        /// The subkey: a token the verifier must refuse.
        Subkey,
    }

    pub(crate) struct Issuer {
        pub(crate) secret: SignedSecretKey,
    }

    impl Issuer {
        /// A primary certified for signing, as a `YubiKey` generates it.
        pub(crate) fn generate() -> Self {
            Self::with_primary_signing(true)
        }

        /// A certify-only primary, which must not be able to issue.
        pub(crate) fn certify_only() -> Self {
            Self::with_primary_signing(false)
        }

        fn with_primary_signing(can_sign: bool) -> Self {
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
                .can_sign(can_sign)
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

        /// The public certificate, as `keys/*.asc` holds it: the anchor.
        pub(crate) fn certificate(&self) -> pgp::composed::SignedPublicKey {
            self.secret.to_public_key()
        }

        pub(crate) fn certificate_armored(&self) -> String {
            self.secret
                .to_public_key()
                .to_armored_string(ArmorOptions::default())
                .unwrap()
        }

        /// Sign `text` with the key `by` names.
        pub(crate) fn sign(&self, text: &str, by: By) -> String {
            let msg = match by {
                By::Primary => CleartextSignedMessage::sign(
                    OsRng,
                    text,
                    &self.secret.primary_key,
                    &Password::empty(),
                ),
                By::Subkey => {
                    let sub = self.secret.secret_subkeys.first().unwrap();
                    CleartextSignedMessage::sign(OsRng, text, &sub.key, &Password::empty())
                }
            }
            .unwrap();
            msg.to_armored_string(ArmorOptions::default()).unwrap()
        }

        /// A token whose signed block carries two signatures by the primary.
        pub(crate) fn token_with_two_signatures(&self, licence: &Licence) -> Token {
            let text = licence.to_canonical_json().unwrap();
            let one = || {
                CleartextSignedMessage::sign(
                    OsRng,
                    &text,
                    &self.secret.primary_key,
                    &Password::empty(),
                )
                .unwrap()
                .signatures()[0]
                    .clone()
            };
            let msg = CleartextSignedMessage::new_many(&text, |_| Ok(vec![one(), one()])).unwrap();
            let signed = msg.to_armored_string(ArmorOptions::default()).unwrap();
            Token::parse(&assemble(&signed, &self.certificate_armored())).unwrap()
        }

        pub(crate) fn token_text(&self, licence: &Licence) -> String {
            let signed = self.sign(&licence.to_canonical_json().unwrap(), By::Primary);
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

    use super::fixtures::{By, Issuer};
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

    fn token_signed(issuer: &Issuer, text: &str, by: By) -> Token {
        let signed = issuer.sign(text, by);
        Token::parse(&assemble(&signed, &issuer.certificate_armored())).unwrap()
    }

    #[test]
    fn a_well_formed_token_verifies() {
        let issuer = Issuer::generate();
        let licence = licence();
        let token = issuer.token_for(&licence);
        let verified = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap();
        assert_eq!(verified.licence, licence);
        assert_eq!(verified.signer, issuer.primary().fingerprint());
        assert!(verified.signer_expires_at.is_none());
    }

    #[test]
    fn either_of_two_anchors_verifies_its_own_tokens() {
        let first = Issuer::generate();
        let second = Issuer::generate();
        let anchors = [first.certificate(), second.certificate()];
        for issuer in [&first, &second] {
            let verified = verify(&issuer.token_for(&licence()), &anchors, date(2026, 12, 1));
            assert_eq!(verified.unwrap().signer, issuer.primary().fingerprint());
        }
    }

    #[test]
    fn an_unknown_primary_is_refused_before_anything_else() {
        let issuer = Issuer::generate();
        let stranger = Issuer::generate();
        let token = issuer.token_for(&licence());
        let err = verify(&token, &[stranger.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UntrustedPrimary(_)), "{err}");
    }

    #[test]
    fn no_anchors_means_nothing_verifies() {
        let issuer = Issuer::generate();
        let token = issuer.token_for(&licence());
        assert!(verify(&token, &[], date(2026, 12, 1)).is_err());
    }

    #[test]
    fn a_token_with_two_signatures_is_refused() {
        let issuer = Issuer::generate();
        let token = issuer.token_with_two_signatures(&licence());
        let err = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::ExtraSignatures(2)), "{err}");
    }

    #[test]
    fn signing_rights_come_from_the_anchor_not_the_token() {
        // The token bundles a certificate whose self-signature allows
        // signing; the anchor for the same primary key packet carries no
        // self-signature at all. The verifier must judge by the anchor, so
        // the token is refused.
        let issuer = Issuer::generate();
        let token = issuer.token_for(&licence());
        let mut anchor = issuer.certificate();
        anchor.details.direct_signatures.clear();
        anchor.details.users.clear();
        let err = verify(&token, &[anchor], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::NotSigningCapable(_)), "{err}");
    }

    #[test]
    fn a_tampered_payload_is_refused() {
        let issuer = Issuer::generate();
        let signed = issuer.sign(&licence().to_canonical_json().unwrap(), By::Primary);
        let tampered = signed.replace("Example Hospital NV", "Example Hospital BV");
        let token = Token::parse(&assemble(&tampered, &issuer.certificate_armored())).unwrap();
        let err = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::BadSignature(_)), "{err}");
    }

    #[test]
    fn a_licence_signed_by_a_subkey_is_refused() {
        let issuer = Issuer::generate();
        let token = token_signed(&issuer, &licence().to_canonical_json().unwrap(), By::Subkey);
        let err = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UnknownSigner), "{err}");
    }

    #[test]
    fn a_primary_not_certified_for_signing_is_refused() {
        let issuer = Issuer::certify_only();
        let token = token_signed(
            &issuer,
            &licence().to_canonical_json().unwrap(),
            By::Primary,
        );
        let err = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::NotSigningCapable(_)), "{err}");
    }

    #[test]
    fn a_certificate_swapped_under_a_valid_signature_is_refused() {
        let issuer = Issuer::generate();
        let other = Issuer::generate();
        let signed = issuer.sign(&licence().to_canonical_json().unwrap(), By::Primary);
        let token = Token::parse(&assemble(&signed, &other.certificate_armored())).unwrap();
        let err = verify(&token, &[other.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::UnknownSigner), "{err}");
    }

    #[test]
    fn a_signed_text_that_is_not_a_licence_is_refused() {
        let issuer = Issuer::generate();
        let token = token_signed(&issuer, "{\"hello\": \"world\"}\n", By::Primary);
        let err = verify(&token, &[issuer.certificate()], date(2026, 12, 1)).unwrap_err();
        assert!(matches!(err, VerifyError::Payload(_)), "{err}");
    }

    #[test]
    fn the_window_is_enforced_on_both_sides() {
        let issuer = Issuer::generate();
        let token = issuer.token_for(&licence());
        assert!(matches!(
            verify(&token, &[issuer.certificate()], date(2026, 9, 10)).unwrap_err(),
            VerifyError::NotYetValid { .. }
        ));
        assert!(matches!(
            verify(&token, &[issuer.certificate()], date(2027, 9, 11)).unwrap_err(),
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
