// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed seals of the system records, those outside the access log
//! (#5573, #6242).
//!
//! The system records are sealed in windows as the access log is
//! ([`crate::access_log::seal`], under the tag
//! [`crate::access_log::seal::SYSTEM_TAG`]). Each window's seal is signed as a
//! [`ChainSeal`]: the window's last position and its seal digest, signed with
//! the keyring's `OpenPGP` key over the seal's canonical form without its
//! proof (RFC 8785, the scheme [`crate::access_log::store::canonical_form`]
//! applies), and published outside the database. A verifier that holds a
//! seal detects a later rewrite of the records it covers. The sealer runs on
//! the access log's schedule, by any replica, one at a time. No openEHR spec
//! governs audit signing: our own design.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::access_log::chain::ALGORITHM;
use crate::access_log::seal::SYSTEM;
use crate::access_log::store::{AccessLogError, AccessLogStore, canonical_form};
use crate::db::acquire::{AcquireError, acquire};
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope};
use crate::versioning::signature::signer::{AttestError, Signer};
use crate::versioning::signature::trust::TrustedCertificates;

/// A signed seal of a window of the system records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainSeal {
    /// The system whose records it seals.
    pub system_id: String,
    /// The last position of the sealed window.
    pub sequence: i64,
    /// The window's seal digest, in lower-case hexadecimal.
    pub digest: String,
    /// The digest algorithm of the seal.
    pub algorithm: String,
    /// When the seal was made, an RFC 3339 instant.
    pub time_sealed: String,
    /// The signature over the seal's canonical form without its proof, an
    /// armored `OpenPGP` signature.
    pub proof: String,
}

/// Why a seal could not be made or read.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// A statement on the store failed.
    #[error("the audit store failed")]
    Store(#[from] sqlx::Error),
    /// No connection to the store could be obtained.
    #[error("the audit store is unavailable")]
    Acquire(#[from] AcquireError),
    /// The keyring holds no `OpenPGP` key or could not sign.
    #[error("the system chain could not be sealed")]
    Sign(#[from] AttestError),
    /// The seal has no canonical form.
    #[error("the seal has no canonical form")]
    Canonical(#[source] AccessLogError),
    /// The windows could not be sealed.
    #[error("the system records could not be sealed")]
    Windows(#[source] AccessLogError),
    /// The chain's verifier failed.
    #[error("the system chain could not be verified")]
    Chain(#[source] Box<AuditError>),
}

/// What a seal fails against the stored chain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealFinding {
    /// The seal is not signed by the keyring's key or a trusted one.
    #[error("the seal at {sequence} is not signed by a trusted key")]
    Proof {
        /// The sealed position.
        sequence: i64,
    },
    /// No stored window ends at the sealed position with the sealed digest.
    #[error("the window ending at {sequence} is not the window the seal fixes")]
    Digest {
        /// The sealed position.
        sequence: i64,
    },
    /// The seal names another algorithm than the chain's.
    #[error("the seal at {sequence} names the algorithm {algorithm:?}")]
    Algorithm {
        /// The sealed position.
        sequence: i64,
        /// The algorithm it names.
        algorithm: String,
    },
}

/// The outcome of verifying the system chain against a seal.
#[derive(Debug, Clone)]
pub struct SealVerification {
    /// The seal the chain was verified against; `None` when none is stored
    /// and none was given.
    pub seal: Option<ChainSeal>,
    /// What the seal fails.
    pub findings: Vec<SealFinding>,
    /// What the chain fails, as its verifier reports it.
    pub chain: Vec<AuditChainFinding>,
}

impl SealVerification {
    /// Whether the chain verifies: a seal was there to verify against, and
    /// nothing failed.
    #[must_use]
    pub fn intact(&self) -> bool {
        self.seal.is_some() && self.findings.is_empty() && self.chain.is_empty()
    }
}

/// The seals of this system's chain, the key that signs them and the
/// certificates that verify them.
#[derive(Debug, Clone)]
pub struct Seals {
    pool: PgPool,
    system_id: String,
    signer: Arc<Signer>,
    trusted: Arc<TrustedCertificates>,
}

impl Seals {
    /// The seals of the system chain in the audit pool `pool`, made for
    /// `system_id` with `signer`'s key; `trusted` verifies a seal a key
    /// retired from the keyring made.
    #[must_use]
    pub fn new(
        pool: PgPool,
        system_id: String,
        signer: Arc<Signer>,
        trusted: Arc<TrustedCertificates>,
    ) -> Self {
        Self {
            pool,
            system_id,
            signer,
            trusted,
        }
    }

    /// Seal every committed system record held by no write, in windows,
    /// and return the newest signed seal; `None` while no window is signed.
    ///
    /// # Errors
    /// [`SealError::Sign`] when a record waits to be sealed and the keyring
    /// holds no `OpenPGP` key, [`SealError::Windows`] when sealing fails, and
    /// the store errors when a statement fails.
    pub async fn seal(&self) -> Result<Option<ChainSeal>, SealError> {
        let store = AccessLogStore::new(self.pool.clone());
        if !self.signer.holds_key() {
            if store.unsealed(&SYSTEM).await.map_err(SealError::Windows)? {
                return Err(SealError::Sign(AttestError::NoKey));
            }
            return self.stored(None).await;
        }
        store
            .with_sealer(Arc::clone(&self.signer), self.system_id.clone())
            .seal_committed(&SYSTEM)
            .await
            .map_err(SealError::Windows)?;
        self.stored(None).await
    }

    /// The signed seal of the window ending at `sequence`, or the newest
    /// signed one when `sequence` is `None`.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn stored(&self, sequence: Option<i64>) -> Result<Option<ChainSeal>, SealError> {
        let found: Option<(i64, String, String, jiff_sqlx::Timestamp, String)> = sqlx::query_as(
            "SELECT last_id, system_id, encode(digest, 'hex'), sealed_at, proof \
             FROM audit.audit_event_seal \
             WHERE proof IS NOT NULL AND ($1::bigint IS NULL OR last_id = $1) \
             ORDER BY window_no DESC LIMIT 1",
        )
        .bind(sequence)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(found.map(|(sequence, system_id, digest, sealed, proof)| {
            let mut seal = ChainSeal::unsigned(&system_id, sequence, digest, sealed.to_jiff());
            seal.proof = proof;
            seal
        }))
    }

    /// Verify the system records against `seal`, or the newest stored one
    /// when none is given: every sealed window, the seal's proof, and that
    /// the stored window ending at its position has the digest it fixes. A
    /// run that finds nothing moves the verification cursor to the last
    /// window.
    ///
    /// # Errors
    /// The store errors when a statement fails; a finding is never an error.
    pub async fn verify(&self, seal: Option<ChainSeal>) -> Result<SealVerification, SealError> {
        let seal = match seal {
            Some(seal) => Some(seal),
            None => self.stored(None).await?,
        };
        let chain = AuditStore::new(self.pool.clone())
            .verify_chain_advancing(AuditChain::System, VerifyScope::Full)
            .await
            .map_err(|error| SealError::Chain(Box::new(error)))?;
        let mut findings = Vec::new();
        if let Some(seal) = &seal {
            let sequence = seal.sequence;
            if seal.algorithm != ALGORITHM {
                findings.push(SealFinding::Algorithm {
                    sequence,
                    algorithm: seal.algorithm.clone(),
                });
            }
            let data = canonical_form(seal).map_err(SealError::Canonical)?;
            if !self.signer.attests(data.as_bytes(), &seal.proof)
                && !self.trusted.verifies(data.as_bytes(), &seal.proof)
            {
                findings.push(SealFinding::Proof { sequence });
            }
            let window: Option<String> = sqlx::query_scalar(
                "SELECT encode(digest, 'hex') FROM audit.audit_event_seal WHERE last_id = $1",
            )
            .bind(sequence)
            .fetch_optional(&mut *acquire(&self.pool).await?)
            .await?;
            if window.as_deref() != Some(seal.digest.as_str()) {
                findings.push(SealFinding::Digest { sequence });
            }
        }
        Ok(SealVerification {
            seal,
            findings,
            chain,
        })
    }
}

impl ChainSeal {
    /// The seal of `system_id`'s window ending at `sequence` over `digest`,
    /// made at `sealed`, with an empty proof.
    #[must_use]
    pub fn unsigned(system_id: &str, sequence: i64, digest: String, sealed: Timestamp) -> Self {
        Self {
            system_id: system_id.to_owned(),
            sequence,
            digest,
            algorithm: ALGORITHM.to_owned(),
            time_sealed: sealed.to_string(),
            proof: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The proof is outside what it signs, so a seal signs the same form
    /// before and after its proof is set.
    #[test]
    fn the_proof_is_outside_the_signed_form() {
        let mut seal = ChainSeal {
            system_id: "cdr.example.org".to_owned(),
            sequence: 7,
            digest: "0".repeat(64),
            algorithm: ALGORITHM.to_owned(),
            time_sealed: "2026-11-14T00:00:00Z".to_owned(),
            proof: String::new(),
        };
        let unsigned = canonical_form(&seal).expect("the form");
        seal.proof = "-----BEGIN PGP SIGNATURE-----".to_owned();
        assert_eq!(canonical_form(&seal).expect("the form"), unsigned);
        assert!(!unsigned.contains("proof"));
    }
}
