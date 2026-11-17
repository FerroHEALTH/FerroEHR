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
//! subjects they declare ([`Census::bytes`]). A team whose census alone is
//! larger is read on every use and never held. The facts notification drops each subject it stamped and every
//! team that holds one of them or that one of them is now under the care of;
//! a grant reload, and a listener that connects or loses its connection, drop
//! both; and a read that a drop overtook keeps nothing. No openEHR spec
//! governs the copy: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::party_binding::grants::caller::store::{self, FactRow};
use crate::party_binding::grants::caller::{CareTeam, FactKind, FactValue};
use crate::policy_register::listen::{Dropping, Subscriber};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The most subjects whose care relationships one replica holds.
pub const SUBJECTS: u64 = 4096;

/// The most bytes the held care teams' censuses take together, by
/// [`Census::bytes`]: about 270 000 patients that each declare one record
/// subject (#5760).
pub const CENSUS_BYTES: u64 = 64 * 1024 * 1024;

/// What a `BTreeMap` spends on each entry beyond its key and value: a leaf
/// node of the standard library holds up to eleven entries beside its
/// parent link and lengths, so an entry carries a share of that header.
const MAP_ENTRY: usize = 8;

/// One patient under a team's care, with the relationship's dates.
pub type Cared = (Uuid, CareTeam);

/// The patients under one team's care, sorted by subject, and the record
/// subjects each declares.
#[derive(Debug, Default)]
pub(crate) struct Census {
    /// Each patient with its relationship to the team.
    pub(crate) cared: Vec<Cared>,
    /// The live record subjects of each patient that declares any.
    pub(crate) declared: BTreeMap<Uuid, Vec<EhrSubject>>,
}

impl Census {
    /// The bytes the census holds on the heap and inline: its patients, and
    /// each declared entry with its subjects and their text. An estimate of
    /// our own design that leaves out the allocator's rounding (#5760).
    pub(crate) fn bytes(&self) -> u64 {
        let cared = self.cared.capacity().saturating_mul(size_of::<Cared>());
        let declared = self
            .declared
            .values()
            .map(|subjects| {
                let text: usize = subjects
                    .iter()
                    .map(|subject| subject.namespace.capacity() + subject.id.capacity())
                    .sum();
                MAP_ENTRY
                    + size_of::<Uuid>()
                    + size_of::<Vec<EhrSubject>>()
                    + subjects.capacity() * size_of::<EhrSubject>()
                    + text
            })
            .fold(0_usize, usize::saturating_add);
        u64::try_from(
            size_of::<Self>()
                .saturating_add(cared)
                .saturating_add(declared),
        )
        .unwrap_or(u64::MAX)
    }

    /// How many record subjects the census's patients declare.
    fn declared_subjects(&self) -> usize {
        self.declared.values().map(Vec::len).sum()
    }
}

/// What the care copy holds, for the copies' gauges (#5760).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CareHeld {
    /// The subjects whose care relationships are held.
    pub subjects: u64,
    /// The care teams whose censuses are held.
    pub teams: u64,
    /// The patients those censuses hold together.
    pub patients: u64,
    /// The record subjects those patients declare.
    pub declared: u64,
    /// The bytes those censuses take, by [`Census::bytes`].
    pub bytes: u64,
}

/// The care relationships one replica holds.
#[derive(Debug)]
pub(crate) struct CareCopy {
    subjects: Cache<Uuid, Arc<[CareTeam]>>,
    teams: Cache<Uuid, Arc<Census>>,
    /// The `PERSON` each record subject names; `None` for one no `PERSON`
    /// declares.
    persons: Cache<EhrSubject, Option<Uuid>>,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
}

impl CareCopy {
    /// An empty copy.
    pub(crate) fn new() -> Self {
        Self::bounded(CENSUS_BYTES)
    }

    /// An empty copy whose censuses take at most `census_bytes` together.
    fn bounded(census_bytes: u64) -> Self {
        Self {
            subjects: Cache::builder().max_capacity(SUBJECTS).build(),
            teams: Cache::builder()
                .weigher(|_, census: &Arc<Census>| {
                    u32::try_from(census.bytes()).unwrap_or(u32::MAX).max(1)
                })
                .max_capacity(census_bytes)
                .build(),
            persons: Cache::builder().max_capacity(SUBJECTS).build(),
            drops: AtomicU64::new(0),
        }
    }

    /// What the copy holds: its subjects and teams, the patients and
    /// declared subjects of those teams, and their bytes.
    pub(crate) fn held(&self) -> CareHeld {
        let (patients, declared) =
            self.teams
                .iter()
                .fold((0_usize, 0_usize), |(patients, declared), (_, census)| {
                    (
                        patients.saturating_add(census.cared.len()),
                        declared.saturating_add(census.declared_subjects()),
                    )
                });
        CareHeld {
            subjects: self.subjects.entry_count(),
            teams: self.teams.entry_count(),
            patients: u64::try_from(patients).unwrap_or(u64::MAX),
            declared: u64::try_from(declared).unwrap_or(u64::MAX),
            bytes: self.teams.weighted_size(),
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
    ) -> Result<Arc<[CareTeam]>, StorageError> {
        if let Some(held) = self.subjects.get(&subject).await {
            return Ok(held);
        }
        let seen = self.drops.load(Ordering::Acquire);
        let mut conn = acquire(pool).await?;
        let rows = store::care_of_subject(&mut conn, subject).await?;
        let teams: Arc<[CareTeam]> = care_teams(&rows).into();
        self.subjects.insert(subject, Arc::clone(&teams)).await;
        if self.drops.load(Ordering::Acquire) != seen {
            self.subjects.invalidate(&subject).await;
        }
        Ok(teams)
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
        if let Some(held) = self.persons.get(subject).await {
            return Ok(held);
        }
        let seen = self.drops.load(Ordering::Acquire);
        let mut conn = acquire(pool).await?;
        let person = super::caller::subject::declaring(&mut conn, subject).await?;
        self.persons.insert(subject.clone(), person).await;
        if self.drops.load(Ordering::Acquire) != seen {
            self.persons.invalidate(subject).await;
        }
        Ok(person)
    }

    /// The patients under the care of `team`, sorted by subject, with the
    /// record subjects they declare, from the copy or read on `pool`.
    ///
    /// # Errors
    /// [`StorageError`] when the read fails.
    pub(crate) async fn of_team(
        &self,
        pool: &PgPool,
        team: Uuid,
    ) -> Result<Arc<Census>, StorageError> {
        if let Some(held) = self.teams.get(&team).await {
            return Ok(held);
        }
        let seen = self.drops.load(Ordering::Acquire);
        let mut conn = acquire(pool).await?;
        let mut census: Vec<Cared> = store::care_of_team(&mut conn, team)
            .await?
            .into_iter()
            .flat_map(|(subject, rows)| {
                care_teams(&rows)
                    .into_iter()
                    .filter(|care| care.team() == team)
                    .map(move |care| (subject, care))
            })
            .collect();
        census.sort();
        let persons: Vec<Uuid> = census.iter().map(|(subject, _)| *subject).collect();
        let mut declared: BTreeMap<Uuid, Vec<EhrSubject>> = BTreeMap::new();
        for (person, subject) in super::caller::subject::declared_by(&mut conn, &persons).await? {
            declared.entry(person).or_default().push(subject);
        }
        let census = Arc::new(Census {
            cared: census,
            declared,
        });
        if census.bytes() <= CENSUS_BYTES {
            self.teams.insert(team, Arc::clone(&census)).await;
            if self.drops.load(Ordering::Acquire) != seen {
                self.teams.invalidate(&team).await;
            }
        }
        Ok(census)
    }

    /// Drop each of `subjects` (sorted), every team that holds one of them,
    /// each of `teams`, and every record subject that names one of
    /// `subjects` or is among `declared`.
    pub(crate) async fn drop_changed(
        &self,
        subjects: &[Uuid],
        teams: &[Uuid],
        declared: &[EhrSubject],
    ) {
        if subjects.is_empty() && teams.is_empty() && declared.is_empty() {
            return;
        }
        self.drops.fetch_add(1, Ordering::AcqRel);
        for subject in subjects {
            self.subjects.invalidate(subject).await;
        }
        let holding: Vec<Uuid> = self
            .teams
            .iter()
            .filter(|(_, census)| {
                subjects.iter().any(|subject| {
                    census
                        .cared
                        .binary_search_by_key(subject, |(cared, _)| *cared)
                        .is_ok()
                })
            })
            .map(|(team, _)| *team)
            .collect();
        for team in holding.iter().chain(teams) {
            self.teams.invalidate(team).await;
        }
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

/// The care relationships `rows` hold: each `under_care_of` relationship of a
/// live head with the dates it holds, whether or not it holds today, so the
/// evaluator judges a discharge and the grace after it; sorted and distinct.
pub(crate) fn care_teams(rows: &[FactRow]) -> Vec<CareTeam> {
    let mut out: Vec<CareTeam> = rows
        .iter()
        .filter(|row| row.kind == FactKind::UnderCareOf)
        .filter(|row| matches!(row.term.lifecycle_state.as_str(), "532" | "553"))
        .filter_map(|row| match (&row.value, row.term.dates) {
            (FactValue::Team(team), Some(dates)) => Some(CareTeam::new(*team, dates)),
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
