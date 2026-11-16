// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care projector (#5699).
//!
//! It keeps the clinical care projection (`clinical/0047_access_care.sql`) at
//! the party facts revision this replica holds, so a query's access
//! condition binds only the caller's team ids.
//!
//! It reads the party domain on the party pool, in one repeatable-read
//! snapshot, and writes the clinical domain on the clinical pool, under the
//! projection's state row, so one projector of the cluster writes at a time
//! and no role reaches both domains. Each run applies every `PERSON` stamped
//! since the revision the projection holds, or every `PERSON` in care when
//! it holds none or its digests were taken under another key, then digests
//! the EHR rows that carry a subject and no digest. Only then does this
//! replica read the projection, and only while it is at least as recent as
//! the facts it holds: a removed relationship is never read from a stale row.
//!
//! The projection needs the deployment's identifier-protection key, whose
//! clinical lookup subkey digests every subject reference; without it every
//! query binds the census. No openEHR spec governs the projection: our own
//! design.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use sqlx::{Connection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::identifier_protection::engine::IdentifierProtection;
use crate::party_binding::grants::caller;
use crate::party_binding::grants::care::care_teams;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::FerroEhrService;
use crate::storage::access_care::{self, CareRow, cared_key, subject_key};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The `PERSON`s read and written in one statement.
const CHUNK: usize = 1000;

/// How often the projector compares the facts revision this replica holds
/// with the one the projection reflects; an in-memory read, no statement.
const TICK: std::time::Duration = std::time::Duration::from_millis(200);

/// How long the projector waits after a failed run before it tries again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// The party facts revision the projection is known to reflect on this
/// replica; none before its first run.
#[derive(Debug)]
pub struct Projected(AtomicI64);

impl Default for Projected {
    fn default() -> Self {
        Self(AtomicI64::new(i64::MIN))
    }
}

impl Projected {
    /// Whether the projection reflects at least the facts revision `held`.
    #[must_use]
    pub fn covers(&self, held: Option<i64>) -> bool {
        held.is_some_and(|held| self.0.load(Ordering::Acquire) >= held)
    }

    /// Record that the projection reflects `revision`, never moving back.
    fn reached(&self, revision: i64) {
        self.0.fetch_max(revision, Ordering::AcqRel);
    }
}

/// What a projector run reads and writes.
#[derive(Debug, Clone)]
struct Projector {
    clinical: PgPool,
    party: PgPool,
    engine: Arc<IdentifierProtection>,
    grants: Arc<LiveGrants>,
    projected: Arc<Projected>,
}

impl FerroEhrService {
    /// The keyed digest of `subject`, the reference an EHR row is matched to
    /// the care projection by; `None` without a subject or while identifier
    /// protection is off.
    pub(crate) fn care_subject_digest(&self, subject: Option<&EhrSubject>) -> Option<Vec<u8>> {
        let engine = self.identifier_protection()?;
        Some(engine.care_subject_digest(&subject_key(&self.grants.namespace(), subject?)))
    }

    /// Whether the clinical care projection reflects at least the facts
    /// this replica holds, so a query may read it.
    #[must_use]
    pub fn care_projection_current(&self) -> bool {
        self.identifiers.is_some() && self.care_projected.covers(self.grants.facts_revision())
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

    /// Run the care projector once on this replica, as its task runs it, and
    /// answer the facts revision the projection reached; `None` while
    /// identifier protection is off.
    ///
    /// # Errors
    /// [`StorageError`] when a domain cannot be read or written.
    pub async fn project_care(&self) -> Result<Option<i64>, StorageError> {
        let Some(projector) = self.projector() else {
            return Ok(None);
        };
        let revision = projector.project().await?;
        projector.projected.reached(revision);
        Ok(Some(revision))
    }

    /// The projector of this replica; `None` while identifier protection is
    /// off.
    fn projector(&self) -> Option<Projector> {
        Some(Projector {
            clinical: self.pool.clone(),
            party: self.demographic_pool.clone(),
            engine: self.identifiers.clone()?,
            grants: Arc::clone(&self.grants),
            projected: Arc::clone(&self.care_projected),
        })
    }

    /// Keep the care projection at the facts this replica holds, in a
    /// detached task; nothing when identifier protection is off.
    pub fn spawn_care_projector(&self) {
        if let Some(projector) = self.projector() {
            tokio::spawn(async move { projector.run().await });
        }
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
            let held = self.grants.facts_revision();
            if first || !self.projected.covers(held) {
                match self.project().await {
                    Ok(revision) => {
                        self.projected.reached(revision);
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

    /// One run: the projection brought to the party facts revision it reads,
    /// then every undigested EHR row digested; the revision reached.
    async fn project(&self) -> Result<i64, StorageError> {
        let key_check = self.engine.care_key_check();
        let mut clinical = acquire(&self.clinical).await?;
        let mut tx = clinical.begin().await?;
        let (held, check) = access_care::lock_state(&mut tx).await?;
        let whole = held.is_none() || check.as_deref() != Some(key_check.as_slice());
        let mut party = self.party.acquire().await?;
        let mut snapshot = party.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        let (_, revision) = crate::party_binding::grants::store::revisions(&mut snapshot).await?;
        let persons = match held {
            Some(held) if !whole && held >= revision => Vec::new(),
            Some(held) if !whole => caller::store::stamped_since(&mut snapshot, held).await?,
            _ => {
                access_care::clear(&mut tx).await?;
                caller::store::cared_persons(&mut snapshot).await?
            }
        };
        let namespace = self.grants.namespace();
        for chunk in persons.chunks(CHUNK) {
            let rows = self.rows_of(&mut snapshot, chunk, &namespace).await?;
            access_care::replace(&mut tx, chunk, &rows).await?;
        }
        // NOTE: our own design (#5762), the care projector, which runs in the background.
        snapshot.commit().await?;
        access_care::set_state(&mut tx, revision, &key_check).await?;
        // NOTE: our own design (#5762), the care projector, which runs in the background.
        tx.commit().await?;
        self.digest_ehrs(&namespace).await?;
        Ok(revision)
    }

    /// The projected rows of `persons`: each care relationship of a live head,
    /// once for the `PERSON`'s own reference and once for each record subject
    /// it declares, each reference as its keyed digest.
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
            let teams = care_teams(facts);
            if teams.is_empty() {
                continue;
            }
            let mut keys = vec![cared_key(&EhrSubject::new(namespace, person.to_string()))];
            keys.extend(
                declared
                    .iter()
                    .filter(|(holder, _)| holder == person)
                    .map(|(_, subject)| cared_key(subject)),
            );
            let digests = self
                .engine
                .care_subject_digests(keys.iter().map(String::as_str));
            for digest in digests {
                for team in &teams {
                    rows.push(CareRow {
                        digest: digest.clone(),
                        person: *person,
                        team: team.team(),
                        from: team.from(),
                        until: team.until(),
                    });
                }
            }
        }
        Ok(rows)
    }

    /// Digest every EHR row that carries a subject and no digest: rows
    /// written before the projection existed or under another key.
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
            let digests = self
                .engine
                .care_subject_digests(keys.iter().map(String::as_str));
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
