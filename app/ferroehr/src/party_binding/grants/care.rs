// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of the care relationships the access evaluator reads.
//!
//! It holds each subject's `under_care_of` relationships, each care team's
//! patients with the record subjects they declare, and the `PERSON` each
//! record subject names (#5634).
//!
//! The copy of the caller facts ([`super::live::LiveGrants`]) holds the facts
//! of callers alone, since the patients in care grow with the record (#5615).
//! A subject's relationships and a team's patients are read on a miss, one
//! indexed statement each, and held in two caches of a fixed bound: at most
//! [`SUBJECTS`] subjects, and teams whose censuses take at most
//! [`CENSUS_BYTES`] together, each weighed by its patients and the record
//! subjects they declare (`Census::bytes`). A team whose census alone is
//! larger is read on every use and never held. The facts read drops each
//! subject it finds stamped, and brings every held team that holds one of them
//! or that one of them is now under the care of to the stamped subjects'
//! relationships and record subjects, read with the facts of the stamped
//! parties, so a relationship commit never sends a team's census back to the
//! store (#6074); a grant reload, and a listener that connects or loses its
//! connection, drop both; and a read that a drop or a delta overtook keeps
//! nothing. No openEHR spec governs the copy: our own design.

use std::collections::BTreeMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::memory::{self, CACHE_ENTRY, Held, MAP_ENTRY, Weigh};
use crate::party_binding::grants::caller::care_of::CareOf;
use crate::party_binding::grants::caller::store::{self, Changed, FactRow};
use crate::party_binding::grants::caller::{FactKind, FactValue};
use crate::policy_register::listen::{Dropping, Subscriber};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The most subjects whose care relationships one replica holds, and the
/// most record subjects whose `PERSON` it holds.
pub const SUBJECTS: u64 = 4096;

/// The most bytes the held care teams' censuses take together, by
/// `Census::bytes`.
///
/// About 250 000 patients that each declare one record subject (#5760), each
/// relationship and each patient's record subjects shared behind an `Arc` so
/// a delta copies no relationship (#6074).
pub const CENSUS_BYTES: u64 = 64 * 1024 * 1024;

/// One patient under a carer's care, with the relationship, shared so a
/// delta copies the census without copying its relationships.
pub type Cared = (Uuid, Arc<CareOf>);

/// The patients under one carer's care, sorted by subject, and the record
/// subjects each declares.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Census {
    /// Each patient with its relationship to the team.
    pub(crate) cared: Vec<Cared>,
    /// The live record subjects of each patient that declares any.
    pub(crate) declared: BTreeMap<Uuid, Arc<[EhrSubject]>>,
}

impl Census {
    /// The bytes the census holds on the heap and inline: its patients, and
    /// each declared entry with its subjects and their text. An estimate of
    /// our own design that leaves out the allocator's rounding (#5760).
    pub(crate) fn bytes(&self) -> u64 {
        let parts = self.parts();
        memory::gauge(
            parts
                .header
                .saturating_add(parts.cared)
                .saturating_add(parts.declared),
        )
    }

    /// The census's bytes by part.
    fn parts(&self) -> Parts {
        let cared = self.cared.iter().map(|(_, care)| care.heap()).fold(
            self.cared.capacity().saturating_mul(size_of::<Cared>()),
            usize::saturating_add,
        );
        let declared = self
            .declared
            .iter()
            .map(|(person, subjects)| {
                MAP_ENTRY
                    .saturating_add(memory::bytes(person))
                    .saturating_add(memory::bytes(subjects))
            })
            .fold(0_usize, usize::saturating_add);
        Parts {
            header: size_of::<Self>(),
            cared,
            declared,
        }
    }

    /// How many record subjects the census's patients declare.
    fn declared_subjects(&self) -> usize {
        self.declared.values().map(|subjects| subjects.len()).sum()
    }

    /// The census of `team` once `delta` is applied: each stamped subject
    /// holds the relationships to `team` it holds now and the live record
    /// subjects it declares now, the census read at the delta's revision.
    fn applied(&self, team: Uuid, delta: &Delta) -> Self {
        let mut cared: Vec<Cared> = self
            .cared
            .iter()
            .filter(|(subject, _)| !delta.cared.contains_key(subject))
            .cloned()
            .collect();
        cared.extend(delta.cared.iter().flat_map(|(subject, relationships)| {
            relationships
                .iter()
                .filter(|care| care.carer() == team)
                .map(|care| (*subject, Arc::clone(care)))
        }));
        cared.sort();
        let mut declared = self.declared.clone();
        for subject in delta.cared.keys() {
            declared.remove(subject);
            let held = cared
                .binary_search_by_key(subject, |(cared, _)| *cared)
                .is_ok();
            if let Some(subjects) = delta.declared.get(subject).filter(|_| held) {
                declared.insert(*subject, Arc::clone(subjects));
            }
        }
        Self { cared, declared }
    }

    /// Whether `delta` changes the census of `team`: it holds a stamped
    /// subject, or a stamped subject is now under the care of `team`.
    fn touched_by(&self, team: Uuid, delta: &Delta) -> bool {
        delta.cared.iter().any(|(subject, relationships)| {
            relationships.iter().any(|care| care.carer() == team)
                || self
                    .cared
                    .binary_search_by_key(subject, |(cared, _)| *cared)
                    .is_ok()
        })
    }
}

/// What the parties stamped since a snapshot hold now, as a census
/// reads it: each stamped party's care relationships, none for a party with
/// none, and the live record subjects of each stamped `PERSON` that
/// declares any, in their order.
#[derive(Debug)]
struct Delta {
    cared: BTreeMap<Uuid, Vec<Arc<CareOf>>>,
    declared: BTreeMap<Uuid, Arc<[EhrSubject]>>,
}

impl Delta {
    /// The delta `changed` holds.
    fn of(changed: &Changed) -> Self {
        let mut cared: BTreeMap<Uuid, Vec<Arc<CareOf>>> = changed
            .parties
            .iter()
            .map(|(party, _)| (*party, Vec::new()))
            .collect();
        for (subject, rows) in &changed.cared {
            cared.insert(*subject, care_of(rows).into_iter().map(Arc::new).collect());
        }
        let mut declared: BTreeMap<Uuid, Vec<EhrSubject>> = BTreeMap::new();
        for row in changed.declared.iter().filter(|row| row.live) {
            declared
                .entry(row.person)
                .or_default()
                .push(row.subject.clone());
        }
        Self {
            cared,
            declared: declared
                .into_iter()
                .map(|(person, subjects)| (person, subjects.into()))
                .collect(),
        }
    }
}

/// The bytes of one census by part: the census itself, its patients, and
/// the record subjects they declare.
#[derive(Debug, Clone, Copy)]
struct Parts {
    header: usize,
    cared: usize,
    declared: usize,
}

/// What the care copy holds, for the copies' gauges (#5760, #5926).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CareHeld {
    /// The subjects whose care relationships are held, and their bytes.
    pub subjects: Held,
    /// The record subjects whose `PERSON` is held, and their bytes.
    pub persons: Held,
    /// The care teams whose censuses are held; their bytes are the
    /// censuses' own beside their patients and declared subjects, with
    /// the cache's bookkeeping.
    pub teams: Held,
    /// The patients those censuses hold together, and their bytes.
    pub patients: Held,
    /// The record subjects those patients declare, and their bytes.
    pub declared: Held,
    /// The bytes the censuses take by `Census::bytes`, which
    /// [`CENSUS_BYTES`] bounds.
    pub census_bytes: u64,
}

/// The care relationships one replica holds.
#[derive(Debug)]
pub(crate) struct CareCopy {
    subjects: Cache<Uuid, Arc<[CareOf]>>,
    teams: Cache<Uuid, Arc<Census>>,
    /// The `PERSON` each record subject names; `None` for one no `PERSON`
    /// declares.
    persons: Cache<EhrSubject, Option<Uuid>>,
    /// The most bytes the held censuses take together.
    census_bytes: u64,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
}

impl CareCopy {
    /// An empty copy.
    pub(crate) fn new() -> Self {
        Self::bounded(SUBJECTS, CENSUS_BYTES)
    }

    /// An empty copy of at most `subjects` subjects and as many record
    /// subjects, whose censuses take at most `census_bytes` together.
    fn bounded(subjects: u64, census_bytes: u64) -> Self {
        Self {
            subjects: Cache::builder().max_capacity(subjects).build(),
            teams: Cache::builder()
                .weigher(|_, census: &Arc<Census>| {
                    u32::try_from(census.bytes()).unwrap_or(u32::MAX).max(1)
                })
                .max_capacity(census_bytes)
                .build(),
            persons: Cache::builder().max_capacity(subjects).build(),
            census_bytes,
            drops: AtomicU64::new(0),
        }
    }

    /// What the copy holds: its subjects, persons and teams, the patients
    /// and declared subjects of those teams, and their bytes.
    pub(crate) fn held(&self) -> CareHeld {
        let mut teams = Parts {
            header: 0,
            cared: 0,
            declared: 0,
        };
        let (mut patients, mut declared) = (0_usize, 0_usize);
        for (team, census) in &self.teams {
            let parts = census.parts();
            teams.header = teams
                .header
                .saturating_add(memory::bytes(team.as_ref()))
                .saturating_add(size_of::<Arc<Census>>())
                .saturating_add(CACHE_ENTRY)
                .saturating_add(parts.header);
            teams.cared = teams.cared.saturating_add(parts.cared);
            teams.declared = teams.declared.saturating_add(parts.declared);
            patients = patients.saturating_add(census.cared.len());
            declared = declared.saturating_add(census.declared_subjects());
        }
        CareHeld {
            subjects: memory::held(&self.subjects),
            persons: memory::held(&self.persons),
            teams: Held {
                entries: self.teams.entry_count(),
                bytes: memory::gauge(teams.header),
            },
            patients: Held {
                entries: memory::gauge(patients),
                bytes: memory::gauge(teams.cared),
            },
            declared: Held {
                entries: memory::gauge(declared),
                bytes: memory::gauge(teams.declared),
            },
            census_bytes: self.teams.weighted_size(),
        }
    }

    /// The value `cache` holds for `key`, or `read` it and hold it when it
    /// `fits`, unless a drop overtook the read. An evicted entry is read
    /// again, so the cap never changes what a caller is answered (#5926).
    /// Concurrent misses of one key wait for one read, and each reads on its
    /// own only when that read held nothing: it failed or did not fit (#6342).
    async fn hold<K, V, E>(
        &self,
        cache: &Cache<K, V>,
        key: K,
        read: impl Future<Output = Result<V, E>>,
        fits: impl FnOnce(&V) -> bool,
    ) -> Result<V, E>
    where
        K: Hash + Eq + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let mut read = std::pin::pin!(read);
        let mut fits = Some(fits);
        loop {
            let seen = self.drops.load(Ordering::Acquire);
            let mut led = None;
            // NOTE: moka 0.12 `Entry::or_optionally_insert_with`, concurrent calls on one key
            // run one `init` and the others wait for it, answered `None` when it holds nothing.
            // Boxed: moka's future is large, and a request's future holds it on its stack.
            let held = Box::pin(cache.entry(key.clone()).or_optionally_insert_with(async {
                let value = read.as_mut().await;
                let keep = match (&value, fits.take()) {
                    (Ok(value), Some(fits)) => fits(value).then(|| value.clone()),
                    _ => None,
                };
                led = Some(value);
                keep
            }))
            .await;
            match (led, held) {
                (Some(value), held) => {
                    if held.is_some() && self.drops.load(Ordering::Acquire) != seen {
                        cache.invalidate(&key).await;
                    }
                    return value;
                }
                (None, Some(held)) => return Ok(held.into_value()),
                (None, None) => {}
            }
        }
    }

    /// The care relationships of `subject`, from the copy or read on `pool`.
    ///
    /// # Errors
    /// [`StorageError`] when the read fails.
    pub(crate) async fn of_subject(
        &self,
        pool: &PgPool,
        subject: Uuid,
    ) -> Result<Arc<[CareOf]>, StorageError> {
        let read = async {
            let mut conn = acquire(pool).await?;
            let rows = store::care_of_subject(&mut conn, subject).await?;
            Ok(care_of(&rows).into())
        };
        self.hold(&self.subjects, subject, read, |_| true).await
    }

    /// The `PERSON` that declares the record subject `subject`, from the copy
    /// or read on `pool`.
    ///
    /// # Errors
    /// [`StorageError`] when the read fails.
    pub(crate) async fn person_of(
        &self,
        pool: &PgPool,
        subject: &EhrSubject,
    ) -> Result<Option<Uuid>, StorageError> {
        let read = async {
            let mut conn = acquire(pool).await?;
            super::caller::subject::declaring(&mut conn, subject).await
        };
        self.hold(&self.persons, subject.clone(), read, |_| true)
            .await
    }

    /// The patients under the care of `team`, sorted by subject, with the
    /// record subjects they declare, from the copy or read on `pool`. A
    /// census larger than the bound is read on every use and never held.
    ///
    /// # Errors
    /// [`StorageError`] when the read fails.
    pub(crate) async fn of_team(
        &self,
        pool: &PgPool,
        team: Uuid,
    ) -> Result<Arc<Census>, StorageError> {
        let bound = self.census_bytes;
        self.hold(&self.teams, team, read_census(pool, team), |census| {
            census.bytes() <= bound
        })
        .await
    }

    /// Bring the copy to `changed`, the facts of the parties stamped by
    /// transactions the snapshot it holds did not see: drop each stamped subject's
    /// relationships, apply the stamped subjects' relationships and record
    /// subjects to every held census they change ([`Census::applied`]), and
    /// drop every record subject that names a stamped party or that one of
    /// them declares.
    pub(crate) async fn apply(&self, changed: &Changed) {
        if changed.parties.is_empty() {
            return;
        }
        self.drops.fetch_add(1, Ordering::AcqRel);
        let subjects: Vec<Uuid> = changed.parties.iter().map(|(party, _)| *party).collect();
        for subject in &subjects {
            self.subjects.invalidate(subject).await;
        }
        let delta = Delta::of(changed);
        let touched: Vec<(Uuid, Census)> = self
            .teams
            .iter()
            .filter(|(team, census)| census.touched_by(**team, &delta))
            .map(|(team, census)| (*team, census.applied(*team, &delta)))
            .collect();
        for (team, census) in touched {
            if census.bytes() <= self.census_bytes {
                self.teams.insert(team, Arc::new(census)).await;
            } else {
                self.teams.invalidate(&team).await;
            }
        }
        let declared = changed.declared.iter().map(|row| &row.subject);
        let resolved: Vec<EhrSubject> = self
            .persons
            .iter()
            .filter(|(_, person)| {
                person.is_some_and(|person| subjects.binary_search(&person).is_ok())
            })
            .map(|(subject, _)| (*subject).clone())
            .collect();
        for subject in resolved.iter().chain(declared) {
            self.persons.invalidate(subject).await;
        }
    }

    /// Drop everything.
    pub(crate) fn drop_all(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        self.subjects.invalidate_all();
        self.teams.invalidate_all();
        self.persons.invalidate_all();
    }
}

impl Subscriber for CareCopy {
    fn notified<'a>(&'a self, _payload: &'a str) -> Dropping<'a> {
        // NOTE: our own design, the read the notification starts drops the parties it stamped.
        Box::pin(std::future::ready(()))
    }

    fn reset(&self) -> Dropping<'_> {
        self.drop_all();
        Box::pin(std::future::ready(()))
    }
}

/// The census of `team` as the store holds it.
async fn read_census(pool: &PgPool, team: Uuid) -> Result<Arc<Census>, StorageError> {
    let mut conn = acquire(pool).await?;
    let mut census: Vec<Cared> = store::care_of_team(&mut conn, team)
        .await?
        .into_iter()
        .flat_map(|(subject, rows)| {
            care_of(&rows)
                .into_iter()
                .filter(|care| care.carer() == team)
                .map(move |care| (subject, Arc::new(care)))
        })
        .collect();
    census.sort();
    let persons: Vec<Uuid> = census.iter().map(|(subject, _)| *subject).collect();
    let mut declared: BTreeMap<Uuid, Vec<EhrSubject>> = BTreeMap::new();
    for (person, subject) in super::caller::subject::declared_by(&mut conn, &persons).await? {
        declared.entry(person).or_default().push(subject);
    }
    Ok(Arc::new(Census {
        cared: census,
        declared: declared
            .into_iter()
            .map(|(person, subjects)| (person, subjects.into()))
            .collect(),
    }))
}

/// The care relationships `rows` hold: each `under_care_of` relationship of a
/// live head with the dates it holds, whether or not it holds today, so the
/// evaluator judges a discharge and the grace after it; sorted and distinct.
pub(crate) fn care_of(rows: &[FactRow]) -> Vec<CareOf> {
    let mut out: Vec<CareOf> = rows
        .iter()
        .filter(|row| row.kind == FactKind::UnderCareOf)
        .filter(|row| matches!(row.term.lifecycle_state.as_str(), "532" | "553"))
        .filter_map(|row| match (&row.value, row.term.dates) {
            (FactValue::Care(care), Some(dates)) => Some(
                CareOf::new(care.carer, dates, care.role.clone(), care.provider.clone())
                    .written_by(row.writer, care.self_declared)
                    .stored_as(
                        (row.holder, row.version.clone()),
                        care.kind,
                        care.covers_for.clone(),
                    )
                    .in_episode(care.episode.clone().map(Arc::new)),
            ),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
#[path = "care_tests.rs"]
mod tests;
