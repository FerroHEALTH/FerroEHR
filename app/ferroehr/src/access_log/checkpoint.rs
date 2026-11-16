// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed checkpoints of the access log and the verification of the log
//! against them (#5129, #5704).
//!
//! A checkpoint seals a position of the system's chain: its sequence and link
//! digest, signed with the keyring's `OpenPGP` key in the form
//! `ATTESTATION._proof_` takes (RM common `master04-generic_package.adoc`
//! §Attestation). Every append ends, in its own transaction, with the
//! checkpoint of the last link it wrote ([`Sealer`]), so every committed
//! entry is covered by a signature from the moment it commits, and the chain
//! of links carries that signature down to every record below it. A
//! verifier that holds one detects any later rewrite of the records it
//! covers. The access log is a chain of its own, apart from the records
//! outside it (#5573), verified where it is stored by
//! `audit.verify_access_log_chain()` through
//! [`AuditStore::verify_chain_advancing`], which accepts a tombstone in place
//! of its entry and a gap in place of a folded run (#5619); this module adds
//! what a checkpoint adds: its proof and the link it seals.
//!
//! No openEHR spec governs log signing: our own design
//! (`docs/plans/access-log.md` §Integrity).

use std::sync::Arc;

use jiff::Timestamp;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};

use crate::access_log::chain::{ALGORITHM, ChainFinding};
use crate::access_log::imported::ImportedFinding;
use crate::access_log::store::{AccessLogError, AccessLogStore, canonical_form};
use crate::access_log::transfer::named_party;
use crate::db::acquire::acquire;
use crate::system_log::seal::Seals;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope};
use crate::telemetry::access_path;
use crate::versioning::audit::dv_date_time;
use crate::versioning::signature::signer::{AttestError, Signer};
use crate::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::invariants::check_checkpoint;

/// The columns a stored checkpoint is rebuilt from.
const CHECKPOINT_COLUMNS: &str = "sequence, system_id, digest, time_sealed, proof";

/// The keyring and the system that seal the appends of this system's log.
#[derive(Debug, Clone)]
pub struct Sealer {
    signer: Arc<Signer>,
    system_id: String,
}

impl Sealer {
    /// Seal for `system_id` with `signer`'s key.
    #[must_use]
    pub fn new(signer: Arc<Signer>, system_id: String) -> Self {
        Self { signer, system_id }
    }

    /// Begin the checkpoint of the link at `sequence` whose digest is
    /// `digest`: its canonical form is signed on the blocking pool, since an
    /// RSA signature takes milliseconds, while the caller goes on.
    ///
    /// # Errors
    /// [`AccessLogError::Clock`] when the time cannot be named to the
    /// microsecond, and [`AccessLogError::Canonical`] when the checkpoint has
    /// no canonical form.
    pub(super) fn begin(&self, sequence: i64, digest: &str) -> Result<PendingSeal, AccessLogError> {
        let sealed = Timestamp::from_microsecond(Timestamp::now().as_microsecond())
            .map_err(AccessLogError::Clock)?;
        let checkpoint = unsigned(&self.system_id, sequence, digest.to_owned(), &sealed);
        let form = canonical_form(&checkpoint)?;
        let signer = Arc::clone(&self.signer);
        let signing = tokio::task::spawn_blocking(move || signer.attest(form.as_bytes()));
        Ok(PendingSeal {
            checkpoint,
            sealed,
            signing,
            started: std::time::Instant::now(),
        })
    }
}

/// A checkpoint whose signature is being made, owed by the transaction of the
/// append it seals.
#[derive(Debug)]
pub struct PendingSeal {
    checkpoint: AccessLogCheckpoint,
    sealed: Timestamp,
    signing: tokio::task::JoinHandle<Result<String, AttestError>>,
    started: std::time::Instant,
}

impl PendingSeal {
    /// Wait for the signature and store the checkpoint on `conn`; the stored
    /// checkpoint is returned.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when the keyring cannot sign,
    /// [`AccessLogError::Signing`] when the task signing it ends without an
    /// answer, [`AccessLogError::Invalid`] for a checkpoint that breaks an
    /// invariant, and the store errors when the insert fails.
    pub(super) async fn write(
        self,
        conn: &mut PgConnection,
    ) -> Result<AccessLogCheckpoint, AccessLogError> {
        let signed = match self.signing.await {
            Ok(signed) => signed.map_err(AccessLogError::Sign),
            Err(unfinished) => Err(AccessLogError::Signing(unfinished)),
        };
        access_path::metrics().log_sign_duration.record(
            access_path::seconds_since(self.started),
            &[access_path::outcome(&signed)],
        );
        let mut checkpoint = self.checkpoint;
        checkpoint.proof = signed?;
        check_checkpoint(&checkpoint)?;
        sqlx::query(
            "INSERT INTO audit.access_log_checkpoint (sequence, system_id, digest, time_sealed, \
             proof) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (sequence) DO NOTHING",
        )
        .bind(checkpoint.sequence)
        .bind(&checkpoint.system_id)
        .bind(&checkpoint.digest)
        .bind(jiff_sqlx::Timestamp::from(self.sealed))
        .bind(&checkpoint.proof)
        .execute(&mut *conn)
        .await?;
        Ok(checkpoint)
    }
}

/// The checkpoint of `system_id` at `sequence` over `digest`, sealed at
/// `sealed`, with an empty proof.
fn unsigned(
    system_id: &str,
    sequence: i64,
    digest: String,
    sealed: &Timestamp,
) -> AccessLogCheckpoint {
    // NOTE: the signer is the organisation responsible for the system, named by its system_id (our own design).
    AccessLogCheckpoint {
        system_id: system_id.to_owned(),
        sequence,
        digest,
        algorithm: ALGORITHM.to_owned(),
        time_sealed: dv_date_time(sealed),
        signer: named_party(system_id),
        proof: String::new(),
    }
}

/// The checkpoint `row` of `audit.access_log_checkpoint` holds, rebuilt from
/// the columns [`CHECKPOINT_COLUMNS`] names.
fn stored_checkpoint(row: &PgRow) -> Result<AccessLogCheckpoint, AccessLogError> {
    let system_id: String = row.try_get("system_id")?;
    let sealed = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_sealed")?
        .to_jiff();
    let mut checkpoint = unsigned(
        &system_id,
        row.try_get("sequence")?,
        row.try_get("digest")?,
        &sealed,
    );
    checkpoint.proof = row.try_get("proof")?;
    Ok(checkpoint)
}

/// The outcome of verifying the log against a checkpoint.
#[derive(Debug, Clone)]
pub struct Verification {
    /// The checkpoint the log was verified against; `None` when none is
    /// stored and none was given.
    pub checkpoint: Option<AccessLogCheckpoint>,
    /// What the checkpoint and the signed links fail.
    pub findings: Vec<ChainFinding>,
    /// What the access log's chain fails, as its verifier reports it.
    pub chain: Vec<AuditChainFinding>,
    /// What the logs imported from other systems fail against their own
    /// checkpoints (#5664).
    pub imported: Vec<ImportedFinding>,
}

impl Verification {
    /// Whether the log verifies: a checkpoint was there to verify against,
    /// and nothing failed.
    #[must_use]
    pub fn intact(&self) -> bool {
        self.checkpoint.is_some()
            && self.findings.is_empty()
            && self.chain.is_empty()
            && self.imported.is_empty()
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
    /// systems signed. Whenever the keyring holds an `OpenPGP` key every
    /// append through [`Self::store`] ends with its checkpoint (#5704).
    #[must_use]
    pub fn new(
        pool: PgPool,
        system_id: String,
        signer: Arc<Signer>,
        trusted: Arc<TrustedCertificates>,
    ) -> Self {
        let store = AccessLogStore::new(pool.clone());
        let store = if signer.holds_key() {
            store.with_sealer(Arc::clone(&signer), system_id.clone())
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

    /// Whether the keyring holds the `OpenPGP` key checkpoints are signed
    /// with; without one nothing is sealed.
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

    /// The seals of the system chain, the records outside the access log,
    /// made and verified with the same keyring and trusted certificates.
    #[must_use]
    pub fn system_seals(&self) -> Seals {
        Seals::new(
            self.pool.clone(),
            self.system_id.clone(),
            Arc::clone(&self.signer),
            Arc::clone(&self.trusted),
        )
    }

    /// The access-log store over the same pool, sealing each append whenever
    /// the keyring holds a key.
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

    /// The checkpoint of the head of the access log, stored and returned;
    /// `None` while the log is empty.
    ///
    /// A head already sealed, as every append made while the keyring held a
    /// key seals its own, returns its stored checkpoint, so this writes only
    /// for a head appended while the keyring held none.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when the keyring holds no `OpenPGP` key or
    /// cannot sign, [`AccessLogError::Invalid`] for a checkpoint that breaks
    /// an invariant, and the store errors when a statement fails.
    pub async fn seal(&self) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        let mut conn = acquire(&self.pool).await?;
        let head = sqlx::query(
            "SELECT head_seq, head_digest FROM audit.access_log_chain_state \
             WHERE singleton AND head_seq > 0",
        )
        .fetch_optional(&mut *conn)
        .await?;
        let Some(head) = head else {
            return Ok(None);
        };
        let sequence: i64 = head.try_get("head_seq")?;
        if let Some(sealed) = self.at(sequence).await? {
            return Ok(Some(sealed));
        }
        let digest: String = head.try_get("head_digest")?;
        Sealer::new(Arc::clone(&self.signer), self.system_id.clone())
            .begin(sequence, &digest)?
            .write(&mut conn)
            .await?;
        self.at(sequence).await
    }

    /// Mark the checkpoint at `sequence` published outside the database, so
    /// the fold keeps it and the gap that folds its position records the link
    /// it seals (#5727); a checkpoint already marked stays as it is.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn mark_published(&self, sequence: i64) -> Result<(), AccessLogError> {
        sqlx::query(
            "UPDATE audit.access_log_checkpoint SET published_at = now() \
             WHERE sequence = $1 AND published_at IS NULL",
        )
        .bind(sequence)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(())
    }

    /// The stored checkpoint at `sequence`.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn at(&self, sequence: i64) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_checkpoint WHERE sequence = $1"
        )))
        .bind(sequence)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        row.as_ref().map(stored_checkpoint).transpose()
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
            None => sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_checkpoint \
                 ORDER BY sequence DESC LIMIT 1"
            ))),
            Some(sequence) => sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_checkpoint \
                 WHERE sequence >= $1 ORDER BY sequence LIMIT 1"
            )))
            .bind(sequence),
        }
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        row.as_ref().map(stored_checkpoint).transpose()
    }

    /// Verify the log against `checkpoint`, or the newest stored one when
    /// none is given: the whole chain, the checkpoint's proof and that the
    /// link at its sequence, a record's or a gap's, is the one it seals; and
    /// every log imported from another system against each of its
    /// checkpoints ([`Self::verify_imported`]).
    ///
    /// A run that finds nothing moves the access log's verification cursor
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
            .verify_chain_advancing(AuditChain::AccessLog, VerifyScope::Full)
            .await
            .map_err(|error| AccessLogError::Chain(Box::new(error)))?;
        let mut findings = Vec::new();
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &mut findings).await?;
        }
        let imported = self.verify_imported().await?;
        Ok(Verification {
            checkpoint,
            findings,
            chain,
            imported,
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
        let sealed: Option<String> =
            sqlx::query_scalar("SELECT audit.access_log_sealed_link($1, $2)")
                .bind(sequence)
                .bind(&checkpoint.proof)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
        if sealed.as_deref() != Some(checkpoint.digest.as_str()) {
            findings.push(ChainFinding::CheckpointDigest { sequence });
        }
        Ok(())
    }
}

/// The checkpoint an exporting system signed, stored at `sequence` as its
/// ITS-JSON `text`.
pub(super) fn read(sequence: i64, text: &str) -> Result<AccessLogCheckpoint, AccessLogError> {
    serde_json::from_str(text).map_err(|source| AccessLogError::Unreadable {
        chain_seq: sequence,
        source,
    })
}
