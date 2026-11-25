// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one `OpenPGP` verification path every signature FerroEHR checks takes.
//!
//! A certificate whose primary key is ML-DSA-65+Ed25519 (RFC 9980 algorithm ID
//! 30) verifies through `ferrohealth-openpgp-pq`: both halves must verify and a
//! digest under 256 bits is refused before any cryptography (RFC 9980 §3.2,
//! §5.3.1). Any other certificate holds a classical key retained to verify the
//! signatures already stored, checked on its primary key and its subkeys. Every
//! signature over MD5, SHA-1 or RIPEMD-160 is refused before any cryptography
//! (RFC 9580 §9.5).

use ferrohealth_openpgp_pq::certificate::Certificate as CompositeCertificate;
use ferrohealth_openpgp_pq::composite::ALGORITHM_ID;
use ferrohealth_openpgp_pq::error::Error as CompositeError;
use pgp::composed::SignedPublicKey;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::Signature;
use pgp::types::{Fingerprint, KeyDetails as _};

/// A certificate FerroEHR verifies signatures against.
#[derive(Debug, Clone)]
pub enum Certificate {
    /// An ML-DSA-65+Ed25519 certificate (RFC 9980 algorithm ID 30), checked
    /// on construction.
    Composite(Box<CompositeCertificate>),
    /// A classical certificate, kept to verify the signatures it already made.
    Classical(Box<SignedPublicKey>),
}

/// Why a signature did not verify against a [`Certificate`].
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    /// The signature's hash is one RFC 9580 §9.5 forbids validating (MD5,
    /// SHA-1, RIPEMD-160) or, for ML-DSA-65+Ed25519, has a digest under 256
    /// bits (RFC 9980 §5.3.1). Refused before any cryptography.
    #[error("the signature's hash {hash:?} is too weak to be accepted")]
    WeakDigest {
        /// The hash algorithm the signature names.
        hash: HashAlgorithm,
    },
    /// The ML-DSA-65+Ed25519 check refused the signature or one of its
    /// halves did not verify.
    #[error("the ML-DSA-65+Ed25519 signature does not verify")]
    Composite(#[source] CompositeError),
    /// The signature verifies under neither the classical certificate's
    /// primary key nor any of its subkeys.
    #[error("the signature does not verify under the classical certificate")]
    Classical(#[source] pgp::errors::Error),
}

impl Certificate {
    /// Takes `signed` as an ML-DSA-65+Ed25519 certificate when its primary
    /// key is algorithm 30, and as a classical one otherwise.
    ///
    /// # Errors
    /// The refusal of `ferrohealth-openpgp-pq` for an algorithm 30
    /// certificate that fails its checks: not version 6, no valid direct-key
    /// self-signature, revoked, or without the signing flag.
    pub fn from_signed(signed: SignedPublicKey) -> Result<Self, CompositeError> {
        if signed.primary_key.algorithm() == PublicKeyAlgorithm::from(ALGORITHM_ID) {
            CompositeCertificate::from_signed(signed)
                .map(|certificate| Self::Composite(Box::new(certificate)))
        } else {
            Ok(Self::Classical(Box::new(signed)))
        }
    }

    /// The fingerprint of the primary key.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        match self {
            Self::Composite(certificate) => certificate.fingerprint(),
            Self::Classical(signed) => signed.primary_key.fingerprint(),
        }
    }

    /// Verifies the detached `signature` over `data`.
    ///
    /// # Errors
    /// [`SignatureError::WeakDigest`] before any cryptography, then
    /// [`SignatureError::Composite`] or [`SignatureError::Classical`] for a
    /// signature that does not verify.
    pub fn verify(&self, signature: &Signature, data: &[u8]) -> Result<(), SignatureError> {
        if let Some(hash) = signature.hash_alg()
            && matches!(
                hash,
                HashAlgorithm::Md5 | HashAlgorithm::Sha1 | HashAlgorithm::Ripemd160
            )
        {
            return Err(SignatureError::WeakDigest { hash });
        }
        match self {
            Self::Composite(certificate) => certificate
                .verify(signature, data)
                .map(|_verification| ())
                .map_err(|error| match error {
                    CompositeError::DigestTooShort { hash } => SignatureError::WeakDigest { hash },
                    other => SignatureError::Composite(other),
                }),
            Self::Classical(signed) => {
                let primary = signature.verify(signed.as_ref(), data);
                match primary {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        if signed
                            .public_subkeys
                            .iter()
                            .any(|subkey| signature.verify(subkey, data).is_ok())
                        {
                            Ok(())
                        } else {
                            Err(SignatureError::Classical(error))
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pgp::composed::{Deserializable as _, DetachedSignature, KeyType, SecretKeyParamsBuilder};
    use pgp::packet::{SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::{SignatureBytes, Timestamp};
    use rand::rngs::OsRng;

    use super::*;
    use crate::licence::token::Token;

    const A3_CERTIFICATE: &str =
        include_str!("../../../corpus/openpgp-id30/rfc9980/a3.2-certificate.asc");
    const A3_SIGNATURE: &str =
        include_str!("../../../corpus/openpgp-id30/rfc9980/a3.4-signature.asc");

    /// The certificate and the one signature of a signed vector, with the text
    /// it signs.
    fn signed(vector: &str) -> (Certificate, Signature, String) {
        let token = Token::parse(vector).expect("a signed vector");
        let certificate =
            Certificate::from_signed(token.certificate().clone()).expect("the certificate");
        let [signature] = token.message().signatures() else {
            panic!("one signature");
        };
        (certificate, signature.clone(), token.signed_text())
    }

    /// A signature packet of `config` with zeroed signature bytes, refused on
    /// its form before any cryptography.
    fn unsigned(config: SignatureConfig, length: usize) -> Signature {
        Signature::from_config(
            config,
            [0, 0],
            SignatureBytes::Native(vec![0; length].into()),
        )
        .expect("a signature packet")
    }

    #[test]
    fn the_rfc_9980_a3_text_signature_verifies_over_its_crlf_hashed_text() {
        let (certificate, _) = SignedPublicKey::from_string(A3_CERTIFICATE).expect("A.3.2");
        let certificate = Certificate::from_signed(certificate).expect("an ID 30 certificate");
        assert!(matches!(certificate, Certificate::Composite(_)));
        assert_eq!(
            format!("{:x}", certificate.fingerprint()),
            "a3e2e14b6a493ff930fb27321f125e9a6880338be9fb7da3ae065ea65793242f"
        );
        let (signature, _) = DetachedSignature::from_string(A3_SIGNATURE).expect("A.3.4");
        assert_eq!(signature.signature.typ(), Some(SignatureType::Text));
        certificate
            .verify(&signature.signature, b"Testing\n")
            .expect("over Testing\\n");
        certificate
            .verify(&signature.signature, b"Testing\r\n")
            .expect("hashed as Testing\\r\\n");
        assert!(matches!(
            certificate.verify(&signature.signature, b"Testing!\n"),
            Err(SignatureError::Composite(_))
        ));
    }

    #[test]
    fn good_vectors_verify() {
        for vector in [
            include_str!("../../../corpus/openpgp-id30/signed/ok-sha3-256.asc"),
            include_str!("../../../corpus/openpgp-id30/signed/ok-sha512.asc"),
        ] {
            let (certificate, signature, text) = signed(vector);
            certificate
                .verify(&signature, text.as_bytes())
                .expect("a good vector verifies");
        }
    }

    #[test]
    fn a_sha2_224_signature_is_refused_for_its_digest() {
        let (certificate, signature, text) = signed(include_str!(
            "../../../corpus/openpgp-id30/signed/weak-digest-sha224.asc"
        ));
        assert!(matches!(
            certificate.verify(&signature, text.as_bytes()),
            Err(SignatureError::WeakDigest {
                hash: HashAlgorithm::Sha224
            })
        ));
    }

    #[test]
    fn a_tampered_text_or_a_broken_half_does_not_verify() {
        for vector in [
            include_str!("../../../corpus/openpgp-id30/signed/tampered.asc"),
            include_str!("../../../corpus/openpgp-id30/signed/ed25519-half-broken.asc"),
            include_str!("../../../corpus/openpgp-id30/signed/mldsa-half-broken.asc"),
        ] {
            let (certificate, signature, text) = signed(vector);
            assert!(matches!(
                certificate.verify(&signature, text.as_bytes()),
                Err(SignatureError::Composite(_))
            ));
        }
    }

    /// RFC 9580 §9.5: a SHA-1 signature is refused before any cryptography,
    /// whether the certificate is ML-DSA-65+Ed25519 or classical.
    #[test]
    fn a_sha1_signature_is_refused_before_any_cryptography() {
        let created = || {
            Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::now()))
                .expect("subpacket")
        };
        let (a3, _) = SignedPublicKey::from_string(A3_CERTIFICATE).expect("A.3.2");
        let composite = Certificate::from_signed(a3).expect("an ID 30 certificate");
        let mut v6 = SignatureConfig::v6_with_salt(
            SignatureType::Binary,
            PublicKeyAlgorithm::from(ALGORITHM_ID),
            HashAlgorithm::Sha1,
            vec![0; 16],
        );
        v6.hashed_subpackets = vec![created()];
        assert!(matches!(
            composite.verify(&unsigned(v6, 3373), b"data"),
            Err(SignatureError::WeakDigest {
                hash: HashAlgorithm::Sha1
            })
        ));

        let mut builder = SecretKeyParamsBuilder::default();
        builder
            .key_type(KeyType::Ed25519Legacy)
            .can_sign(true)
            .primary_user_id("sha-1 test <test@ferroehr.local>".into());
        let classical_key = builder
            .build()
            .expect("params")
            .generate(OsRng)
            .expect("a key");
        let classical = Certificate::from_signed(classical_key.to_public_key()).expect("classical");
        assert!(matches!(classical, Certificate::Classical(_)));
        let mut v4 = SignatureConfig::v4(
            SignatureType::Binary,
            PublicKeyAlgorithm::EdDSALegacy,
            HashAlgorithm::Sha1,
        );
        v4.hashed_subpackets = vec![created()];
        assert!(matches!(
            classical.verify(&unsigned(v4, 64), b"data"),
            Err(SignatureError::WeakDigest {
                hash: HashAlgorithm::Sha1
            })
        ));
    }
}
