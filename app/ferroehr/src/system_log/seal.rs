// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed seals of the system chain, the records outside the access log
//! (#5573).
//!
//! The access log seals its head in an `ACCESS_LOG_CHECKPOINT`
//! ([`crate::access_log::checkpoint`]); the system chain (`audit.audit_event`)
//! has its own head and seals it the same way: its position and link digest,
//! signed with the keyring's `OpenPGP` key over the seal's canonical form
//! without its proof (RFC 8785, the scheme
//! [`crate::access_log::store::canonical_form`] applies). A verifier that
//! holds a seal detects a later rewrite of the records it covers. Seals are
//! written on the access log's schedule, by any replica: a position sealed
//! twice keeps its first seal. No openEHR spec governs audit signing: our own
//! design.

use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::access_log::chain::ALGORITHM;
use crate::access_log::store::{AccessLogError, canonical_form};
use crate::db::acquire::{AcquireError, acquire};
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope};
use crate::versioning::signature::signer::{AttestError, Signer};
use crate::versioning::signature::trust::TrustedCertificates;

/// A signed seal of the head of the system chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainSeal {
    /// The system whose chain it seals.
    pub system_id: String,
    /// The position of the sealed record.
    pub sequence: i64,
    /// The sealed record's link digest, in lower-case hexadecimal.
    pub digest: String,
    /// The digest algorithm of the chain.
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
    /// The seal stored at `sequence` does not read.
    #[error("the seal at {sequence} does not read")]
    Unreadable {
        /// The sealed position.
        sequence: i64,
        /// Why it does not read.
        #[source]
        source: serde_json::Error,
    },
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
    /// The chain's link at the sealed position is not the sealed digest.
    #[error("the link at {sequence} is not the link the seal fixes")]
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

    /// Seal the head of the system chain, stored and returned; `None` while
    /// the chain holds no record. A head sealed already returns its stored
    /// seal, so a run on an idle chain writes nothing.
    ///
    /// # Errors
    /// [`SealError::Sign`] when the keyring holds no `OpenPGP` key or cannot
    /// sign, and the store errors when a statement fails.
    pub async fn seal(&self) -> Result<Option<ChainSeal>, SealError> {
        let head = sqlx::query(
            "SELECT chain_seq, encode(row_hash, 'hex') AS digest FROM audit.audit_event \
             ORDER BY chain_seq DESC LIMIT 1",
        )
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        let Some(head) = head else {
            return Ok(None);
        };
        let sequence: i64 = head.try_get("chain_seq")?;
        if let Some(sealed) = self.stored(Some(sequence)).await? {
            return Ok(Some(sealed));
        }
        let mut seal = ChainSeal {
            system_id: self.system_id.clone(),
            sequence,
            digest: head.try_get("digest")?,
            algorithm: ALGORITHM.to_owned(),
            time_sealed: Timestamp::now().to_string(),
            proof: String::new(),
        };
        let unsigned = canonical_form(&seal).map_err(SealError::Canonical)?;
        seal.proof = self.signer.attest(unsigned.as_bytes())?;
        let content = serde_json::to_string(&seal)
            .map_err(|source| SealError::Unreadable { sequence, source })?;
        sqlx::query(
            "INSERT INTO audit.audit_chain_seal (sequence, digest, time_sealed, content) \
             VALUES ($1, $2, now(), $3) ON CONFLICT (sequence) DO NOTHING",
        )
        .bind(sequence)
        .bind(&seal.digest)
        .bind(&content)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        let stored = self.stored(Some(sequence)).await?;
        Ok(stored.or(Some(seal)))
    }

    /// The stored seal at `sequence`, or the newest stored one when
    /// `sequence` is `None`.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`SealError::Unreadable`] when the stored seal does not read.
    pub async fn stored(&self, sequence: Option<i64>) -> Result<Option<ChainSeal>, SealError> {
        let found: Option<(i64, String)> = sqlx::query_as(
            "SELECT sequence, content FROM audit.audit_chain_seal \
             WHERE $1::bigint IS NULL OR sequence = $1 ORDER BY sequence DESC LIMIT 1",
        )
        .bind(sequence)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        found.map(|(at, content)| read(at, &content)).transpose()
    }

    /// Verify the system chain against `seal`, or the newest stored one when
    /// none is given: the whole chain, the seal's proof, and that the link
    /// at its position is the one it fixes. A run that finds nothing in the
    /// chain moves its verification cursor to the newest record.
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
            let link: Option<String> = sqlx::query_scalar("SELECT audit.audit_chain_link_at($1)")
                .bind(sequence)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            if link.as_deref() != Some(seal.digest.as_str()) {
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

/// The seal stored at `sequence` as `text`.
fn read(sequence: i64, text: &str) -> Result<ChainSeal, SealError> {
    serde_json::from_str(text).map_err(|source| SealError::Unreadable { sequence, source })
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
