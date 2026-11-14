// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of the care relationships the access evaluator reads:
//! each subject's `under_care_of` relationships, and each care team's
//! patients.
//!
//! The copy of the caller facts ([`super::live::LiveGrants`]) holds the facts
//! of callers alone, since the patients in care grow with the record (#5615).
//! A subject's relationships and a team's patients are read on a miss, one
//! indexed statement each, and held in two caches of a fixed bound: at most
//! [`SUBJECTS`] subjects, and teams holding at most [`CENSUS`] patients
//! together. A team with more patients than that is read on every use and
//! never held. The facts notification drops each subject it stamped and every
//! team that holds one of them or that one of them is now under the care of;
//! a grant reload, and a listener that connects or loses its connection, drop
//! both; and a read that a drop overtook keeps nothing. No openEHR spec
//! governs the copy: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;
use sqlx::PgPool;
use uuid::Uuid;

use crate::party_binding::grants::caller::store::{self, FactRow};
use crate::party_binding::grants::caller::{CareTeam, FactKind, FactValue};
use crate::policy_register::listen::{Dropping, Subscriber};
use crate::storage::error::StorageError;

/// The most subjects whose care relationships one replica holds.
pub const SUBJECTS: u64 = 4096;

/// The most patients the held care teams hold together.
pub const CENSUS: u64 = 262_144;

/// One patient under a team's care, with the relationship's dates.
pub type Cared = (Uuid, CareTeam);

/// The care relationships one replica holds.
#[derive(Debug)]
pub(crate) struct CareCopy {
    subjects: Cache<Uuid, Arc<[CareTeam]>>,
    teams: Cache<Uuid, Arc<[Cared]>>,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
}

impl CareCopy {
    /// An empty copy.
    pub(crate) fn new() -> Self {
        Self {
            subjects: Cache::builder().max_capacity(SUBJECTS).build(),
            teams: Cache::builder()
                .weigher(|_, census: &Arc<[Cared]>| {
                    u32::try_from(census.len()).unwrap_or(u32::MAX).max(1)
                })
                .max_capacity(CENSUS)
                .build(),
            drops: AtomicU64::new(0),
        }
    }

    /// How many subjects and teams the copy holds, and the patients the teams
    /// hold together.
    pub(crate) fn held(&self) -> (u64, u64, u64) {
        (
            self.subjects.entry_count(),
            self.teams.entry_count(),
            self.teams.weighted_size(),
        )
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
        let mut conn = pool.acquire().await?;
        let rows = store::care_of_subject(&mut conn, subject).await?;
        let teams: Arc<[CareTeam]> = care_teams(&rows).into();
        self.subjects.insert(subject, Arc::clone(&teams)).await;
        if self.drops.load(Ordering::Acquire) != seen {
            self.subjects.invalidate(&subject).await;
        }
        Ok(teams)
    }

    /// The patients under the care of `team`, sorted by subject, from the
    /// copy or read on `pool`.
    ///
    /// # Errors
    /// [`StorageError`] when the read fails.
    pub(crate) async fn of_team(
        &self,
        pool: &PgPool,
        team: Uuid,
    ) -> Result<Arc<[Cared]>, StorageError> {
        if let Some(held) = self.teams.get(&team).await {
            return Ok(held);
        }
        let seen = self.drops.load(Ordering::Acquire);
        let mut conn = pool.acquire().await?;
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
        let census: Arc<[Cared]> = census.into();
        if u64::try_from(census.len()).is_ok_and(|patients| patients <= CENSUS) {
            self.teams.insert(team, Arc::clone(&census)).await;
            if self.drops.load(Ordering::Acquire) != seen {
                self.teams.invalidate(&team).await;
            }
        }
        Ok(census)
    }

    /// Drop each of `subjects` (sorted), every team that holds one of them,
    /// and each of `teams`.
    pub(crate) async fn drop_changed(&self, subjects: &[Uuid], teams: &[Uuid]) {
        if subjects.is_empty() && teams.is_empty() {
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
                        .binary_search_by_key(subject, |(cared, _)| *cared)
                        .is_ok()
                })
            })
            .map(|(team, _)| *team)
            .collect();
        for team in holding.iter().chain(teams) {
            self.teams.invalidate(team).await;
        }
    }

    /// Drop everything.
    pub(crate) fn drop_all(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        self.subjects.invalidate_all();
        self.teams.invalidate_all();
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
fn care_teams(rows: &[FactRow]) -> Vec<CareTeam> {
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
