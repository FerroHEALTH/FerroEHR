// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The signed checkpoints of the access log and the verification of the log
//! against them (#5129, #5704, #6242).
//!
//! A checkpoint is the signed seal of a window: the `ACCESS_LOG_CHECKPOINT`
//! of the log up to the window's last entry id, whose digest is the window's
//! seal digest, signed with the keyring's `OpenPGP` key in the form
//! `ATTESTATION._proof_` takes (RM common `master04-generic_package.adoc`
//! §Attestation). Each seal follows the one before it, so a verifier that
//! holds a checkpoint detects any later rewrite of the entries it covers
//! ([`crate::access_log::seal`]). The verification recomputes every window,
//! checks the checkpoint's proof and seal, and verifies every imported bundle
//! ([`crate::access_log::imported`]).
//!
//! No openEHR spec governs log signing: our own design.

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
use crate::system_log::store::chains::AuditChainFinding;
use crate::telemetry::access_path;
use crate::versioning::audit::dv_date_time;
use crate::versioning::signature::signer::{AttestError, Signer};
use crate::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use openehr_rm::draft::access_log::invariants::check_checkpoint;

/// The columns a stored checkpoint is rebuilt from.
const CHECKPOINT_COLUMNS: &str = "last_id AS sequence, system_id, encode(digest, 'hex') AS digest, \
     sealed_at AS time_sealed, proof";

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

/// A window's seal as signed: the `ACCESS_LOG_CHECKPOINT` of the log up to
/// its last entry id over its seal digest, its proof, the system it names and
/// the time it names.
#[derive(Debug, Clone)]
pub struct SignedSeal {
    /// The armored signature.
    pub proof: String,
    /// The system the checkpoint names.
    pub system_id: String,
    /// The time the checkpoint names, to the microsecond.
    pub sealed: Timestamp,
}

impl Sealer {
    /// Sign the seal of the log up to entry `last_id` whose digest is
    /// `digest`, as the `ACCESS_LOG_CHECKPOINT` at that position over that
    /// digest (#6242), on the blocking pool.
    ///
    /// # Errors
    /// [`AccessLogError::Clock`], [`AccessLogError::Canonical`],
    /// [`AccessLogError::Sign`] and [`AccessLogError::Signing`] when the
    /// checkpoint cannot be built or signed, and [`AccessLogError::Sign`] or
    /// [`AccessLogError::Signing`] when the signature fails.
    pub async fn sign_seal(
        &self,
        last_id: i64,
        digest: &str,
    ) -> Result<SignedSeal, AccessLogError> {
        let pending = self.begin(last_id, digest)?;
        let signed = match pending.signing.await {
            Ok(signed) => signed.map_err(AccessLogError::Sign),
            Err(unfinished) => Err(AccessLogError::Signing(unfinished)),
        };
        access_path::metrics().log_sign_duration.record(
            access_path::seconds_since(pending.started),
            &[access_path::outcome(&signed)],
        );
        let mut checkpoint = pending.checkpoint;
        checkpoint.proof = signed?;
        check_checkpoint(&checkpoint)?;
        Ok(SignedSeal {
            proof: checkpoint.proof,
            system_id: self.system_id.clone(),
            sealed: pending.sealed,
        })
    }
}

/// A checkpoint whose signature is being made.
#[derive(Debug)]
pub struct PendingSeal {
    checkpoint: AccessLogCheckpoint,
    sealed: Timestamp,
    signing: tokio::task::JoinHandle<Result<String, AttestError>>,
    started: std::time::Instant,
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

/// The checkpoint `row` of `audit.access_log_seal` holds, rebuilt from
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

    /// The access-log store over the same pool, sealing the chain whenever
    /// the keyring holds a key.
    #[must_use]
    pub fn store(&self) -> &AccessLogStore {
        &self.store
    }

    /// The keyring's signature over `data`.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when the keyring holds no `OpenPGP` key or
    /// cannot sign.
    pub fn sign(&self, data: &[u8]) -> Result<String, AccessLogError> {
        self.signer.attest(data).map_err(AccessLogError::Sign)
    }

    /// Whether `signature` is a signature over `data` by the keyring's key, a
    /// certificate retired from it, or a trusted certificate.
    #[must_use]
    pub fn signed(&self, data: &[u8], signature: &str) -> bool {
        self.signer.attests(data, signature) || self.trusted.verifies(data, signature)
    }

    /// The checkpoint of the access log up to its last committed entry:
    /// every committed entry sealed in its window, and the newest signed
    /// seal returned; `None` while nothing is sealed.
    ///
    /// # Errors
    /// [`AccessLogError::Sign`] when an entry waits to be sealed and the
    /// keyring holds no `OpenPGP` key or cannot sign, and the store errors
    /// when a statement fails.
    pub async fn seal(&self) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
        if !self.signer.holds_key() {
            let unsealed: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM audit.access_log_entry e \
                 WHERE e.id > (SELECT s.last_id FROM audit.access_log_seal_state s \
                                WHERE s.singleton))",
            )
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
            if unsealed {
                return Err(AccessLogError::Sign(AttestError::NoKey));
            }
            return self.latest(None).await;
        }
        self.store.seal_committed().await?;
        self.latest(None).await
    }

    /// Mark the checkpoint at `sequence` published outside the database; a
    /// checkpoint already marked stays as it is.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn mark_published(&self, sequence: i64) -> Result<(), AccessLogError> {
        sqlx::query(
            "UPDATE audit.access_log_seal SET published_at = now() \
             WHERE last_id = $1 AND published_at IS NULL",
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
        let mut conn = acquire(&self.pool).await?;
        stored_at(&mut conn, sequence).await
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
                "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_seal \
                 WHERE proof IS NOT NULL ORDER BY window_no DESC LIMIT 1"
            ))),
            Some(sequence) => sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_seal \
                 WHERE proof IS NOT NULL AND last_id >= $1 ORDER BY window_no LIMIT 1"
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
        let chain = self.store.verify_windows(false).await?;
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
        let sealed: Option<String> = sqlx::query_scalar(
            "SELECT encode(digest, 'hex') FROM audit.access_log_seal WHERE last_id = $1",
        )
        .bind(sequence)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?;
        if sealed.as_deref() != Some(checkpoint.digest.as_str()) {
            findings.push(ChainFinding::CheckpointDigest { sequence });
        }
        Ok(())
    }
}

/// The stored checkpoint at `sequence`, read on `conn`.
///
/// # Errors
/// The store errors when the statement fails.
async fn stored_at(
    conn: &mut PgConnection,
    sequence: i64,
) -> Result<Option<AccessLogCheckpoint>, AccessLogError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {CHECKPOINT_COLUMNS} FROM audit.access_log_seal \
         WHERE last_id = $1 AND proof IS NOT NULL"
    )))
    .bind(sequence)
    .fetch_optional(conn)
    .await?;
    row.as_ref().map(stored_checkpoint).transpose()
}
