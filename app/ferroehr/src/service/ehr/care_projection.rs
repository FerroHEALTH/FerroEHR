// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care projector (#5699).
//!
//! It keeps the clinical care projection (`clinical/0009_baseline_access.sql`) at
//! the party facts this replica holds, so a query's access condition binds
//! only the caller's team ids.
//!
//! It reads the party domain on the party pool, in one repeatable-read
//! snapshot, and gives that connection back before it writes the clinical
//! domain on the clinical pool, under the projection's state row, so one
//! projector of the cluster writes at a time, no role reaches both domains
//! and no pool is waited for while another's connection is held. Each run
//! applies every `PERSON` stamped by a transaction the snapshot the
//! projection was read under did not see, or every `PERSON` in care when it
//! holds none or its digests were taken in another form, then digests the EHR
//! rows that carry a subject and no digest. Only then does this replica read
//! the projection, and only while its snapshot covers the one the facts it
//! holds were read under: a removed relationship is never read from a stale
//! row.
//!
//! Every subject reference is held as its SHA-256 digest
//! ([`access_care::subject_digest`]). No openEHR spec governs the
//! projection: our own design.

use std::sync::{Arc, PoisonError, RwLock};

use sqlx::{Connection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::db::snapshot::Snapshot;
use crate::party_binding::grants::caller;
use crate::party_binding::grants::caller::care_of::{CareOf, CarerKind};
use crate::party_binding::grants::care::care_of;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::FerroEhrService;
use crate::storage::access_care::{
    self, CareRow, StateRead, cared_key, subject_digest, subject_key,
};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The `PERSON`s read and written in one statement.
const CHUNK: usize = 1000;

/// How often the projector compares the snapshot of the facts this replica
/// holds with the one the projection was read under; an in-memory read, no
/// statement.
const TICK: std::time::Duration = std::time::Duration::from_millis(200);

/// How long the projector waits after a failed run before it tries again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// The party snapshot the projection is known to have been read under on
/// this replica; none before its first run.
#[derive(Debug, Default)]
pub struct Projected(RwLock<Option<Arc<Snapshot>>>);

impl Projected {
    /// Whether the projection holds every stamp the snapshot `held` sees.
    #[must_use]
    pub fn covers(&self, held: Option<&Snapshot>) -> bool {
        let projected = self.0.read().unwrap_or_else(PoisonError::into_inner);
        projected
            .as_ref()
            .zip(held)
            .is_some_and(|(projected, held)| projected.covers(held))
    }

    /// Record that the projection was read under `snapshot`, never moving
    /// back.
    fn reached(&self, snapshot: Arc<Snapshot>) {
        let mut projected = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if projected
            .as_ref()
            .is_none_or(|projected| snapshot.covers(projected))
        {
            *projected = Some(snapshot);
        }
    }
}

/// What a projector run reads and writes.
#[derive(Debug, Clone)]
struct Projector {
    clinical: PgPool,
    party: PgPool,
    grants: Arc<LiveGrants>,
    projected: Arc<Projected>,
}

/// The party side of one projector run, read before its clinical
/// transaction begins.
#[derive(Debug)]
struct PartyRead {
    /// The party snapshot the read ran under.
    snapshot: Arc<Snapshot>,
    /// Whether the projection is rebuilt whole.
    whole: bool,
    /// The `PERSON`s to replace, a chunk at a time, each with its rows.
    chunks: Vec<(Vec<Uuid>, Vec<CareRow>)>,
}

impl FerroEhrService {
    /// The digest of `subject`, the reference an EHR row is matched to the
    /// care projection by; `None` without a subject.
    pub(crate) fn care_subject_digest(&self, subject: Option<&EhrSubject>) -> Option<Vec<u8>> {
        Some(subject_digest(&subject_key(
            &self.grants.namespace(),
            subject?,
        )))
    }

    /// Whether the clinical care projection reflects at least the facts
    /// this replica holds, so a query may read it.
    #[must_use]
    pub fn care_projection_current(&self) -> bool {
        self.care_projected
            .covers(self.grants.facts_snapshot().as_deref())
    }

    /// Remove the care of each of `persons` from the projection, the erasure
    /// of a `PERSON` taking its care with it.
    ///
    /// # Errors
    /// [`StorageError`] when the delete fails; the projection is then read
    /// by no query until the projector applies the erasure's stamp.
    pub(crate) async fn forget_care(&self, persons: &[Uuid]) -> Result<(), StorageError> {
        access_care::forget(&self.pool, persons).await
    }

    /// Run the care projector once on this replica, as its task runs it.
    ///
    /// # Errors
    /// [`StorageError`] when a domain cannot be read or written.
    pub async fn project_care(&self) -> Result<(), StorageError> {
        let projector = self.projector();
        let snapshot = projector.project().await?;
        projector.projected.reached(snapshot);
        Ok(())
    }

    /// The projector of this replica.
    fn projector(&self) -> Projector {
        Projector {
            clinical: self.pool.clone(),
            party: self.demographic_pool.clone(),
            grants: Arc::clone(&self.grants),
            projected: Arc::clone(&self.care_projected),
        }
    }

    /// Keep the care projection at the facts this replica holds, in a
    /// detached task.
    pub fn spawn_care_projector(&self) {
        let projector = self.projector();
        tokio::spawn(async move { projector.run().await });
    }
}

impl Projector {
    /// Project whenever the facts this replica holds move past the projection.
    #[expect(
        clippy::infinite_loop,
        reason = "the projector lives as long as the server, like the grants refresher"
    )]
    async fn run(self) {
        let mut first = true;
        loop {
            let held = self.grants.facts_snapshot();
            if first || !self.projected.covers(held.as_deref()) {
                match self.project().await {
                    Ok(snapshot) => {
                        self.projected.reached(snapshot);
                        first = false;
                    }
                    Err(error) => {
                        {
                            let cause: &(dyn std::error::Error + 'static) = &error;
                            tracing::warn!(
                                error = cause,
                                "the care projection could not be written; queries bind the census"
                            );
                        }
                        drop(error);
                        tokio::time::sleep(RETRY).await;
                        continue;
                    }
                }
            }
            tokio::time::sleep(TICK).await;
        }
    }

    /// One run: the projection brought to the party facts it reads, then
    /// every undigested EHR row digested; the snapshot it read under.
    ///
    /// The party side is read whole first, from the state the projection
    /// held, and its connection given back before the clinical transaction
    /// begins, which writes it only while the state row still holds that
    /// state, so no pool is waited for while another's connection is held
    /// (#5879, #5986). No openEHR spec governs this: our own design (#6187).
    async fn project(&self) -> Result<Arc<Snapshot>, StorageError> {
        let key_check = subject_digest(access_care::DIGEST_CHECK);
        let namespace = self.grants.namespace();
        loop {
            let seen =
                access_care::state(&mut *acquire(&self.clinical).await?, StateRead::Unlocked)
                    .await?;
            let read = self.read_party(&seen, &key_check, &namespace).await?;
            let mut clinical = acquire(&self.clinical).await?;
            let mut tx = clinical.begin().await?;
            if access_care::state(&mut tx, StateRead::Locked).await? != seen {
                // NOTE: our own design (#6187), another projector wrote since the state was read,
                // so the party side is read again from the state it left.
                tx.rollback().await?;
                continue;
            }
            if read.whole {
                access_care::clear(&mut tx).await?;
            }
            for (persons, rows) in &read.chunks {
                access_care::replace(&mut tx, persons, rows).await?;
            }
            access_care::set_state(&mut tx, &read.snapshot, &key_check).await?;
            // NOTE: our own design (#5762), the care projector, which runs in the background.
            tx.commit().await?;
            drop(clinical);
            self.digest_ehrs(&namespace).await?;
            return Ok(read.snapshot);
        }
    }

    /// The party side of a run from the projection's state `seen`, read in
    /// one repeatable-read snapshot: every `PERSON` stamped by a transaction
    /// the snapshot it holds did not see, or every `PERSON` in care when it
    /// holds none or its digests were taken in a form other than `key_check`.
    async fn read_party(
        &self,
        seen: &access_care::State,
        key_check: &[u8],
        namespace: &str,
    ) -> Result<PartyRead, StorageError> {
        let (held, check) = seen;
        let whole = held.is_none() || check.as_deref() != Some(key_check);
        let mut party = acquire(&self.party).await?;
        let mut snapshot = party.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        let read = Arc::new(crate::db::snapshot::current(&mut snapshot).await?);
        let persons = match held {
            Some(held) if !whole => caller::store::stamped_since(&mut snapshot, held).await?,
            _ => caller::store::cared_persons(&mut snapshot).await?,
        };
        let mut chunks = Vec::new();
        for chunk in persons.chunks(CHUNK) {
            let rows = self.rows_of(&mut snapshot, chunk, namespace).await?;
            chunks.push((chunk.to_vec(), rows));
        }
        // NOTE: our own design (#5762), the care projector, which runs in the background.
        snapshot.commit().await?;
        Ok(PartyRead {
            snapshot: read,
            whole,
            chunks,
        })
    }

    /// The projected rows of `persons`: each care relationship of a live head,
    /// once for the `PERSON`'s own reference and once for each record subject
    /// it declares, each reference as its digest.
    async fn rows_of(
        &self,
        snapshot: &mut sqlx::PgConnection,
        persons: &[Uuid],
        namespace: &str,
    ) -> Result<Vec<CareRow>, StorageError> {
        let care = caller::store::care_of_subjects(snapshot, persons).await?;
        let cared: Vec<Uuid> = care.iter().map(|(person, _)| *person).collect();
        let declared = caller::subject::declared_by(snapshot, &cared).await?;
        let mut rows = Vec::new();
        for (person, facts) in &care {
            let relations = care_of(facts);
            if relations.is_empty() {
                continue;
            }
            let mut keys = vec![cared_key(&EhrSubject::new(namespace, person.to_string()))];
            keys.extend(
                declared
                    .iter()
                    .filter(|(holder, _)| holder == person)
                    .map(|(_, subject)| cared_key(subject)),
            );
            for digest in keys.iter().map(|key| subject_digest(key)) {
                for care in &relations {
                    rows.push(CareRow {
                        digest: digest.clone(),
                        person: *person,
                        carer: care.carer(),
                        carer_kind: stored_kind(care),
                        provider_issuer: care.provider().issuer.clone(),
                        provider_id: care.provider().id.clone(),
                        from: care.from(),
                        until: care.until(),
                        writer: care.writer(),
                        self_declared: care.self_declared(),
                        role: care.role().clone(),
                    });
                }
            }
        }
        Ok(rows)
    }

    /// Digest every EHR row that carries a subject and no digest: rows
    /// written before the projection existed or in another digest form.
    async fn digest_ehrs(&self, namespace: &str) -> Result<(), StorageError> {
        let mut conn = acquire(&self.clinical).await?;
        loop {
            let batch = access_care::undigested(&mut conn, 1000).await?;
            if batch.is_empty() {
                return Ok(());
            }
            let keys: Vec<String> = batch
                .iter()
                .map(|(_, subject)| subject_key(namespace, subject))
                .collect();
            let digests = keys.iter().map(|key| subject_digest(key));
            let rows: Vec<_> = batch
                .into_iter()
                .zip(digests)
                .map(|((ehr, subject), digest)| (ehr, subject, digest))
                .collect();
            // NOTE: our own design, a batch whose subjects all moved meanwhile digests nothing,
            // and the rows that moved were digested by the write that moved them.
            if access_care::set_digests(&mut conn, &rows).await? == 0 || rows.len() < 1000 {
                return Ok(());
            }
        }
    }
}

/// What the carer of `care` is, as its stored fact names it.
#[expect(
    clippy::expect_used,
    reason = "care_of builds every relationship with stored_as, which records the carer's kind"
)]
fn stored_kind(care: &CareOf) -> CarerKind {
    care.carer_kind()
        .expect("a relationship read from the facts should carry its carer's kind")
}
