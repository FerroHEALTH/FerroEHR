// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The classical signing key a deployment may still sign with behind
//! `record.signing.allow_classical_key`: which component signs, and which
//! algorithms are refused or warned about.

use pgp::composed::{SignedSecretKey, SignedSecretSubKey};
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{
    PublicKey, Signature, SignatureConfig, SignatureType, SignatureVersion,
    SignatureVersionSpecific, SubpacketData,
};
use pgp::types::KeyDetails as _;
use pgp::types::{Duration, KeyVersion, SignedUser, Tag, Timestamp};

/// Whether `algorithm` signs with RSA, in any of the three spellings RFC 9580
/// §9.1 registers (`RSA`, plus the deprecated sign-only and encrypt-only
/// codepoints an older certificate can still carry).
pub(super) fn rsa_family(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign | PublicKeyAlgorithm::RSAEncrypt
    )
}

/// Returns the index of the secret subkey that signs, or `None` when none
/// may.
///
/// No subkey qualifies while the primary key is revoked or expired
/// ([`primary_in_force_at`]). Otherwise a subkey qualifies when its newest valid Subkey Binding (RFC 9580
/// §5.2.3.10) carries key flag 0x02 (§5.2.3.29), neither that binding nor the
/// subkey has expired at `now` (§5.2.3.13), and no valid Subkey Revocation
/// covers it (§5.2.1.12: "A revoked subkey is not to be used"). Among those,
/// the subkey created last signs, the later one in the file on a tie. RFC 9580
/// gives no rule for choosing among several signing subkeys; preferring the
/// newest is our own design, so that adding a signing subkey rotates to it.
pub(super) fn signing_subkey_index(secret: &SignedSecretKey, now: Timestamp) -> Option<usize> {
    if !primary_in_force_at(secret, now) {
        return None;
    }
    let primary = secret.primary_key.public_key();
    secret
        .secret_subkeys
        .iter()
        .enumerate()
        .filter(|(_, subkey)| may_sign_at(primary, subkey, now))
        .max_by_key(|(_, subkey)| subkey.key.created_at())
        .map(|(index, _)| index)
}

/// Whether `subkey` qualifies to sign at `now`, by the rule of
/// [`signing_subkey_index`].
fn may_sign_at(primary: &PublicKey, subkey: &SignedSecretSubKey, now: Timestamp) -> bool {
    let subkey_public = subkey.key.public_key();
    // NOTE: RFC 9580 §5.2.3.10; a binding that does not verify is not a valid
    // self-signature, so it is skipped rather than refused.
    let valid = || {
        subkey
            .signatures
            .iter()
            .filter(|sig| sig.verify_subkey_binding(primary, subkey_public).is_ok())
    };
    if valid().any(|sig| sig.typ() == Some(SignatureType::SubkeyRevocation)) {
        return false;
    }
    let Some(binding) = valid()
        .filter(|sig| sig.typ() == Some(SignatureType::SubkeyBinding))
        .max_by_key(|sig| sig.created())
    else {
        return false;
    };
    let binding_live = binding
        .created()
        .is_some_and(|created| !expired(created, binding.signature_expiration_time(), now));
    binding.key_flags().sign()
        && binding_live
        && !expired(subkey.key.created_at(), binding.key_expiration_time(), now)
}

/// The primary key's self-signatures that state facts about it.
#[derive(Debug, Clone, Copy)]
struct PrimarySelfSignatures<'a> {
    /// The newest valid Direct Key signature.
    direct: Option<&'a Signature>,
    /// The self-signature the primary key's key flags are read from.
    governing: Option<&'a Signature>,
}

/// The primary key's self-signatures, or `None` when the primary key is not
/// to be used at all: a valid Key Revocation signature covers it (RFC 9580
/// §5.2.1.11: "A revoked key is not to be used"), or it is a version 6 key
/// without a valid Direct Key signature (§5.2.3.10).
///
/// The governing self-signature follows §5.2.3.10, which keeps the primary
/// key's key flags in a Direct Key signature (the version 6 place) or, by
/// version 4 convention, in a User ID self-signature: the newest valid Direct
/// Key signature when it carries a Key Flags subpacket, else the newest valid
/// self-signature on the primary User ID. RFC 9580 gives no precedence
/// between the two places; preferring the Direct Key signature, whose
/// subpackets "apply to the entire key", is our own design.
fn primary_self_signatures(secret: &SignedSecretKey) -> Option<PrimarySelfSignatures<'_>> {
    let primary = secret.primary_key.public_key();
    let details = &secret.details;
    // NOTE: RFC 9580 §5.2.1.11; only a revocation by the key itself is valid.
    let revoked = details.revocation_signatures.iter().any(|sig| {
        sig.typ() == Some(SignatureType::KeyRevocation) && sig.verify_key(primary).is_ok()
    });
    if revoked {
        return None;
    }
    // NOTE: RFC 9580 §5.2.3.10, "Revoking a Direct Key signature cancels that signature".
    let cancellations: Vec<&Signature> = details
        .direct_signatures
        .iter()
        .chain(&details.revocation_signatures)
        .filter(|sig| {
            sig.typ() == Some(SignatureType::CertRevocation) && verifies_over_primary(sig, primary)
        })
        .collect();
    let direct = details
        .direct_signatures
        .iter()
        .filter(|sig| sig.typ() == Some(SignatureType::Key) && sig.verify_key(primary).is_ok())
        .filter(|sig| {
            !cancellations
                .iter()
                .any(|revocation| revocation.created() >= sig.created())
        })
        .max_by_key(|sig| sig.created());
    if primary.version() == KeyVersion::V6 && direct.is_none() {
        return None;
    }
    let governing = direct
        .filter(|sig| states_key_flags(sig))
        .or_else(|| primary_user_self_signature(primary, &details.users));
    Some(PrimarySelfSignatures { direct, governing })
}

/// Whether `sig`, a Certification Revocation (Type ID 0x30) found beside the
/// primary key's Direct Key signatures, verifies against the primary key alone.
///
/// RFC 9580 §5.2.1.13 computes it "over the same data as the certification
/// that it revokes", for a Direct Key signature the primary key alone hashed
/// as §5.2.4 describes. rPGP 0.21's `Signature::verify_key` hashes exactly
/// that but refuses the type, so the same steps are taken here: the issuer
/// matches, the key and signature versions align, the hash's leading octets
/// match and the signature verifies.
fn verifies_over_primary(sig: &Signature, primary: &PublicKey) -> bool {
    use pgp::types::VerifyingKey as _;

    let (Some(config), Some(signed_hash), Some(bytes)) =
        (sig.config(), sig.signed_hash_value(), sig.signature())
    else {
        return false;
    };
    let issuers = config.issuer_key_id();
    let fingerprints = config.issuer_fingerprint();
    let issued_by_primary = (issuers.is_empty() && fingerprints.is_empty())
        || issuers.iter().any(|id| **id == primary.legacy_key_id())
        || fingerprints.iter().any(|fp| **fp == primary.fingerprint());
    let aligned =
        (primary.version() == KeyVersion::V6) == (config.version() == SignatureVersion::V6);
    issued_by_primary
        && aligned
        && key_signature_hash(config, primary).is_ok_and(|hash| {
            hash.get(..2) == Some(signed_hash.as_slice())
                && primary.verify(config.hash_alg, &hash, bytes).is_ok()
        })
}

/// The hash a signature made with `config` over `primary` alone signs
/// (RFC 9580 §5.2.4): the version 6 salt, the key packet framed by 0x99 and a
/// 2-octet length (version 4) or 0x9B and a 4-octet length (version 6), the
/// signature's hashed data and its trailer.
fn key_signature_hash(
    config: &SignatureConfig,
    primary: &PublicKey,
) -> pgp::errors::Result<Vec<u8>> {
    use pgp::ser::Serialize as _;

    let mut hasher = config.hash_alg.new_hasher()?;
    if let SignatureVersionSpecific::V6 { salt } = &config.version_specific {
        hasher.update(salt.as_ref());
    }
    let body = primary.to_bytes()?;
    if primary.version() == KeyVersion::V6 {
        hasher.update(&[0x9B]);
        hasher.update(&u32::try_from(body.len())?.to_be_bytes());
    } else {
        hasher.update(&[0x99]);
        hasher.update(&u16::try_from(body.len())?.to_be_bytes());
    }
    hasher.update(&body);
    let len = config.hash_signature_data(&mut hasher)?;
    hasher.update(&config.trailer(len)?);
    Ok(hasher.finalize().into_vec())
}

/// Whether the primary key is in force at `now`: usable at all
/// ([`primary_self_signatures`]) and not past the Key Expiration Time its
/// governing or Direct Key self-signature states (RFC 9580 §5.2.3.13, counted
/// from the primary key's creation).
///
/// A subkey signs only while its primary key is in force. RFC 9580 does not
/// say in so many words that a subkey inherits its primary key's revocation
/// or expiry; reading the Subkey Binding (§5.2.1.8) as a statement a revoked
/// or expired primary key can no longer make is our own design.
fn primary_in_force_at(secret: &SignedSecretKey, now: Timestamp) -> bool {
    let created = secret.primary_key.created_at();
    primary_self_signatures(secret).is_some_and(|sigs| {
        // NOTE: our own design; an expiry stated on the Direct Key signature binds
        // even when the primary User ID governs.
        [sigs.direct, sigs.governing]
            .into_iter()
            .flatten()
            .all(|sig| !expired(created, sig.key_expiration_time(), now))
    })
}

/// Whether the primary key may sign data at `now`.
///
/// It may when it is in force ([`primary_in_force_at`]) and its governing
/// self-signature ([`primary_self_signatures`]) carries key flag 0x02
/// (RFC 9580 §5.2.3.29), with neither that self-signature nor the Direct Key
/// signature past its Signature Expiration Time (§5.2.3.18). A self-signature
/// with no Key Flags subpacket states no flags, and §5.2.3.29 counts an
/// unstated flag as zero, so it grants no signing.
pub(super) fn primary_may_sign_at(secret: &SignedSecretKey, now: Timestamp) -> bool {
    let Some(sigs) = primary_self_signatures(secret) else {
        return false;
    };
    let Some(governing) = sigs.governing else {
        return false;
    };
    let signature_live = |sig: &Signature| {
        sig.created()
            .is_some_and(|created| !expired(created, sig.signature_expiration_time(), now))
    };
    primary_in_force_at(secret, now)
        && governing.key_flags().sign()
        && signature_live(governing)
        && sigs.direct.is_none_or(signature_live)
}

/// Whether `sig` carries a Key Flags subpacket in its hashed area, the only
/// area a self-signature's statements about the key are read from.
fn states_key_flags(sig: &Signature) -> bool {
    sig.config().is_some_and(|config| {
        config
            .hashed_subpackets()
            .any(|subpacket| matches!(subpacket.data, SubpacketData::KeyFlags(_)))
    })
}

/// The newest valid certification self-signature on the primary User ID, or
/// `None` when no User ID carries one.
///
/// A User ID whose newest valid self-signature is followed by a valid
/// Certification Revocation is retired (RFC 9580 §5.2.1.13, §5.2.3.10) and
/// skipped. The primary User ID is the one whose newest self-signature sets
/// Primary User ID, the one with the most recent self-signature when several
/// do (§5.2.3.27). When none does, the User ID with the most recent
/// self-signature stands in: §5.2.3.27 leaves that case open, so this is our
/// own design.
fn primary_user_self_signature<'a>(
    primary: &PublicKey,
    users: &'a [SignedUser],
) -> Option<&'a Signature> {
    let newest = users.iter().filter_map(|user| {
        let valid = || {
            user.signatures.iter().filter(|sig| {
                sig.verify_certification(primary, Tag::UserId, &user.id)
                    .is_ok()
            })
        };
        let certification = valid()
            .filter(|sig| sig.typ() != Some(SignatureType::CertRevocation))
            .max_by_key(|sig| sig.created())?;
        let retired = valid().any(|sig| {
            sig.typ() == Some(SignatureType::CertRevocation)
                && sig.created() >= certification.created()
        });
        (!retired).then_some(certification)
    });
    let newest: Vec<&Signature> = newest.collect();
    newest
        .iter()
        .copied()
        .filter(|sig| sig.is_primary())
        .max_by_key(|sig| sig.created())
        .or_else(|| newest.iter().copied().max_by_key(|sig| sig.created()))
}

/// Whether a lifetime of `lifetime` from `start` has run out at `now`; an
/// absent or zero lifetime never runs out (RFC 9580 §5.2.3.13).
fn expired(start: Timestamp, lifetime: Option<Duration>, now: Timestamp) -> bool {
    match lifetime {
        Some(lifetime) if lifetime.as_secs() > 0 => {
            u64::from(start.as_secs()) + u64::from(lifetime.as_secs()) <= u64::from(now.as_secs())
        }
        _ => false,
    }
}

/// Whether `algorithm` signs with private-key arithmetic over `crypto-bigint`
/// 0.5: ECDSA on any curve (`elliptic-curve` 0.13 scalars) and Ed448 (`cx448`).
///
/// Ed25519 runs on `curve25519-dalek` and RSA on `num-bigint-dig`, so neither
/// reaches the crate.
pub(super) fn refused_signing_algorithm(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::ECDSA | PublicKeyAlgorithm::Ed448
    )
}

/// Warns that the configured signing key performs RSA private-key operations.
///
/// A warning rather than a boot refusal: an operator whose corpus is already
/// RSA-signed must keep the key configured to verify that history (a
/// `VERSION.signature` is an immutable committed fact — RM common
/// `master06-change_control_package.adoc` §Digital Signature), so refusing the
/// key would strand them with no read path at all.
pub(super) fn warn_rsa_signing_key(algorithm: PublicKeyAlgorithm) {
    tracing::warn!(
        key_algorithm = ?algorithm,
        advisory = "RUSTSEC-2023-0071",
        "the configured OpenPGP signing key is RSA, so every signature is an RSA private-key \
         operation — the operation the Marvin timing sidechannel (RUSTSEC-2023-0071 / \
         CVE-2023-49092) concerns, and the `rsa` crate has no fixed release. Rotate to an Ed25519 \
         signing key and keep this certificate in `record.signing.retired_key_files` so versions \
         already signed keep verifying."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every RFC 9580 §9.1 RSA codepoint classifies as RSA — including the two
    /// deprecated single-purpose ones, which an older certificate still uses
    /// and which sign with exactly the same private-key operation.
    #[test]
    fn every_rsa_codepoint_is_classified_as_rsa() {
        for algorithm in [
            PublicKeyAlgorithm::RSA,
            PublicKeyAlgorithm::RSASign,
            PublicKeyAlgorithm::RSAEncrypt,
        ] {
            assert!(rsa_family(algorithm), "{algorithm:?} must classify as RSA");
        }
    }

    /// A lifetime runs out at its end, and an absent or zero one never does.
    #[test]
    fn a_lifetime_runs_out_at_its_end() {
        let start = Timestamp::from_secs(1_000);
        let lifetime = Some(Duration::from_secs(100));
        assert!(!expired(start, lifetime, Timestamp::from_secs(1_099)));
        assert!(expired(start, lifetime, Timestamp::from_secs(1_100)));
        assert!(!expired(start, None, Timestamp::from_secs(u32::MAX)));
        assert!(!expired(
            start,
            Some(Duration::from_secs(0)),
            Timestamp::from_secs(u32::MAX)
        ));
    }

    /// ECDSA and Ed448 are refused as signing algorithms; Ed25519, in both
    /// encodings, and RSA are not.
    #[test]
    fn ecdsa_and_ed448_are_refused_signing_algorithms() {
        for algorithm in [PublicKeyAlgorithm::ECDSA, PublicKeyAlgorithm::Ed448] {
            assert!(refused_signing_algorithm(algorithm), "{algorithm:?}");
        }
        for algorithm in [
            PublicKeyAlgorithm::Ed25519,
            PublicKeyAlgorithm::EdDSALegacy,
            PublicKeyAlgorithm::RSA,
        ] {
            assert!(!refused_signing_algorithm(algorithm), "{algorithm:?}");
        }
    }

    /// The algorithms that keep `rsa` off the signing path raise no advisory.
    #[test]
    fn elliptic_curve_algorithms_raise_no_advisory() {
        for algorithm in [
            PublicKeyAlgorithm::Ed25519,
            PublicKeyAlgorithm::Ed448,
            PublicKeyAlgorithm::EdDSALegacy,
            PublicKeyAlgorithm::ECDSA,
            PublicKeyAlgorithm::ECDH,
            PublicKeyAlgorithm::X25519,
        ] {
            assert!(
                !rsa_family(algorithm),
                "{algorithm:?} performs no RSA private-key operation"
            );
        }
    }
}
