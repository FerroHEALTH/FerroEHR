// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed checkpoints of the access log and the verification of the log
//! against them (#5129).
//!
//! A checkpoint seals the head of the system's chain: its sequence and link
//! digest, signed with the keyring's `OpenPGP` key in the form
//! `ATTESTATION._proof_` takes (RM common `master04-generic_package.adoc`
//! §Attestation). A verifier that holds one detects any later rewrite of the
//! records it covers. The chain itself is verified where it is stored, by
//! `audit.verify_audit_chain()` through
//! [`AuditStore::verify_chain_advancing`], which accepts a tombstone in place
//! of its entry; this module adds what a checkpoint adds: its proof, the link
//! it seals, and the signature of each signed link.
//!
//! No openEHR spec governs log signing: our own design
//! (`docs/plans/access-log.md` §Integrity).

use std::sync::Arc;

use jiff::Timestamp;
use sqlx::{PgPool, Row};

use crate::access_log::chain::{ALGORITHM, ChainFinding};
use crate::access_log::store::{AccessLogError, AccessLogStore, canonical_form};
use crate::access_log::transfer::named_party;
use crate::db::acquire::acquire;
use crate::system_log::store::{AuditChainFinding, AuditStore, VerifyScope};
use crate::versioning::audit::dv_date_time;
use crate::versioning::signature::signer::Signer;
use crate::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::invariants::check_checkpoint;

/// How many signed links one page of the signature check reads.
const SIGNATURE_PAGE: i64 = 1000;

/// The outcome of verifying the log against a checkpoint.
#[derive(Debug, Clone)]
pub struct Verification {
    /// The checkpoint the log was verified against; `None` when none is
    /// stored and none was given.
    pub checkpoint: Option<AccessLogCheckpoint>,
    /// What the checkpoint and the signed links fail.
    pub findings: Vec<ChainFinding>,
    /// What the chain fails, as the audit chain's verifier reports it.
    pub chain: Vec<AuditChainFinding>,
}

impl Verification {
    /// Whether the log verifies: a checkpoint was there to verify against,
    /// and nothing failed.
    #[must_use]
    pub fn intact(&self) -> bool {
        self.checkpoint.is_some() && self.findings.is_empty() && self.chain.is_empty()
    }
}

/// The checkpoints of this system's access log, the key that signs them and
/// the certificates that verify what other systems signed.
#[derive(Debug, Clone)]
pub struct Checkpoints {
    pool: PgPool,
    store: AccessLogStore,
    system_id: String,
    signer: Arc<Signer>,
    trusted: Arc<TrustedCertificates>,
}

impl Checkpoints {
    /// The checkpoints of the log in the audit pool `pool`, sealed for
    /// `system_id` with `signer`'s key; `trusted` verifies what other
    /// systems signed. Whenever the keyring holds an `OpenPGP` key the
    /// entries this writes carry a signed link (#5562).
    #[must_use]
    pub fn new(
        pool: PgPool,
        system_id: String,
        signer: Arc<Signer>,
        trusted: Arc<TrustedCertificates>,
    ) -> Self {
        let store = AccessLogStore::new(pool.clone());
        let store = if signer.holds_key() {
            store.with_link_signer(Arc::clone(&signer))
        } else {
            store
        };
        Self {
            pool,
            store,
            system_id,
            signer,
            trusted,
        }
    }

    /// Whether the keyring holds the `OpenPGP` key checkpoints and links are
    /// signed with; without one nothing is sealed or signed.
    #[must_use]
    pub fn seals(&self) -> bool {
        self.signer.holds_key()
    }

    /// The system whose log these checkpoints seal.
    #[must_use]
    pub fn system_id(&self) -> &str {
        &self.system_id
    }

    /// The audit pool the log is stored in.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The access-log store over the same pool, signing links whenever the
    /// keyring holds a key.
    #[must_use]
    pub fn store(&self) -> &AccessLogStore {
        &self.store
    }

    /// Whether `signature` is a signature over `data` by the keyring's key, a
    /// certificate retired from it, or a trusted certificate.
    #[must_use]
    pub fn signed(&self, data: &[u8], signature: &str) -> bool {
        self.signer.attests(data, signature) || self.trusted.verifies(data, signature)
    }

    /// Seal the head of the chain in a signed checkpoint, stored and
    /// returned; `None` while the chain is empty.
    ///
    /// A head already sealed returns its stored checkpoint, so a run on an
    /// idle log writes nothing.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when the keyring holds no `OpenPGP` key or
    /// cannot sign, [`AccessLogError::Invalid`] for a checkpoint that breaks
    /// an invariant, and the store errors when a statement fails.
    pub async fn seal(&self) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        let head = sqlx::query(
            "SELECT newest.chain_seq, encode(newest.link, 'hex') AS digest \
             FROM audit.audit_chain_newest() AS newest",
        )
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        let Some(head) = head else {
            return Ok(None);
        };
        let sequence: i64 = head.try_get("chain_seq")?;
        if let Some(sealed) = self.at(sequence).await? {
            return Ok(Some(sealed));
        }
        let mut checkpoint = self.unsigned(sequence, head.try_get("digest")?);
        checkpoint.proof = self
            .signer
            .attest(canonical_form(&checkpoint)?.as_bytes())?;
        check_checkpoint(&checkpoint)?;
        let content = serde_json::to_string(&checkpoint).map_err(AccessLogError::Canonical)?;
        sqlx::query(
            "INSERT INTO audit.access_log_checkpoint (sequence, system_id, digest, time_sealed, \
             content) VALUES ($1, $2, $3, now(), $4) ON CONFLICT (sequence) DO NOTHING",
        )
        .bind(sequence)
        .bind(&checkpoint.system_id)
        .bind(&checkpoint.digest)
        .bind(&content)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(self.at(sequence).await?.or(Some(checkpoint)))
    }

    /// The checkpoint at `sequence` with an empty proof.
    #[must_use]
    fn unsigned(&self, sequence: i64, digest: String) -> AccessLogCheckpoint {
        // NOTE: the signer is the organisation responsible for the system, named by its system_id (our own design).
        AccessLogCheckpoint {
            system_id: self.system_id.clone(),
            sequence,
            digest,
            algorithm: ALGORITHM.to_owned(),
            time_sealed: dv_date_time(&Timestamp::now()),
            signer: named_party(&self.system_id),
            proof: String::new(),
        }
    }

    /// The stored checkpoint at `sequence`.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when the stored checkpoint does not
    /// read.
    pub async fn at(&self, sequence: i64) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        let content: Option<String> = sqlx::query_scalar(
            "SELECT content FROM audit.access_log_checkpoint WHERE sequence = $1",
        )
        .bind(sequence)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        content.map(|text| read(sequence, &text)).transpose()
    }

    /// The newest stored checkpoint, or with `covering` the oldest at or
    /// after that sequence.
    ///
    /// # Errors
    /// As [`Self::at`].
    pub async fn latest(
        &self,
        covering: Option<i64>,
    ) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        let row = match covering {
            None => sqlx::query(
                "SELECT sequence, content FROM audit.access_log_checkpoint \
                 ORDER BY sequence DESC LIMIT 1",
            ),
            Some(sequence) => sqlx::query(
                "SELECT sequence, content FROM audit.access_log_checkpoint \
                 WHERE sequence >= $1 ORDER BY sequence LIMIT 1",
            )
            .bind(sequence),
        }
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        row.map(|row| {
            read(
                row.try_get("sequence")?,
                &row.try_get::<String, _>("content")?,
            )
        })
        .transpose()
    }

    /// Verify the log against `checkpoint`, or the newest stored one when
    /// none is given: the whole chain, the checkpoint's proof, that the link
    /// at its sequence is the one it seals, and the signature of every signed
    /// link.
    ///
    /// A run that finds nothing moves the audit chain's verification cursor
    /// to the newest record, as a scheduled full pass does.
    ///
    /// # Errors
    /// The store errors when a statement fails; a finding is never an error.
    pub async fn verify(
        &self,
        checkpoint: Option<AccessLogCheckpoint>,
    ) -> Result<Verification, AccessLogError> {
        let checkpoint = match checkpoint {
            Some(checkpoint) => Some(checkpoint),
            None => self.latest(None).await?,
        };
        let chain = AuditStore::new(self.pool.clone())
            .verify_chain_advancing(VerifyScope::Full)
            .await
            .map_err(|error| AccessLogError::Chain(Box::new(error)))?;
        let mut findings = Vec::new();
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &mut findings).await?;
        }
        self.verify_link_signatures(&mut findings).await?;
        Ok(Verification {
            checkpoint,
            findings,
            chain,
        })
    }

    /// The findings of `checkpoint` against the stored chain.
    async fn verify_checkpoint(
        &self,
        checkpoint: &AccessLogCheckpoint,
        findings: &mut Vec<ChainFinding>,
    ) -> Result<(), AccessLogError> {
        let sequence = checkpoint.sequence;
        if !self.signed(canonical_form(checkpoint)?.as_bytes(), &checkpoint.proof) {
            findings.push(ChainFinding::CheckpointProof { sequence });
        }
        let sealed: Option<String> = sqlx::query_scalar("SELECT audit.audit_chain_link_at($1)")
            .bind(sequence)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        if sealed.as_deref() != Some(checkpoint.digest.as_str()) {
            findings.push(ChainFinding::CheckpointDigest { sequence });
        }
        Ok(())
    }

    /// The findings of every signed link whose signature does not verify.
    async fn verify_link_signatures(
        &self,
        findings: &mut Vec<ChainFinding>,
    ) -> Result<(), AccessLogError> {
        let mut after = 0_i64;
        loop {
            let rows = sqlx::query(
                "SELECT chain_seq, digest, signature FROM audit.access_log_record \
                 WHERE signature IS NOT NULL AND chain_seq > $1 ORDER BY chain_seq LIMIT $2",
            )
            .bind(after)
            .bind(SIGNATURE_PAGE)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
            let Some(last) = rows.last() else {
                return Ok(());
            };
            after = last.try_get("chain_seq")?;
            for row in &rows {
                let digest: String = row.try_get("digest")?;
                let signature: String = row.try_get("signature")?;
                if !self.signed(digest.as_bytes(), &signature) {
                    findings.push(ChainFinding::LinkSignature {
                        sequence: row.try_get("chain_seq")?,
                    });
                }
            }
        }
    }
}

/// The checkpoint stored at `sequence` as `text`.
fn read(sequence: i64, text: &str) -> Result<AccessLogCheckpoint, AccessLogError> {
    serde_json::from_str(text).map_err(|source| AccessLogError::Unreadable {
        chain_seq: sequence,
        source,
    })
}
