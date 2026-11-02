// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Admin statistics (`list_contributions` / `contribution_count` /
//! `versioned_composition_count` / `composition_version_count`).
//!
//! Each call takes a [`PlatformService`] (`a_service`, the versioned-content
//! service to report on) and an optional ISO 8601 date-time interval matched
//! against the CONTRIBUTION / version audit `time_committed`. No openEHR spec
//! governs the calls or the SQL that answers them —
//! our own design over the greenfield `contribution` / `version` /
//! `commit_audit` tables (`clinical/0003_change_control.sql`), whose
//! statements are `crate::storage::statistics`.

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::admin::types::StatTimeRange;
use crate::service::platform_service::PlatformService;
use crate::storage::statistics::{self, CompositionCount, ContributionScope};

/// The contributions a [`PlatformService`] holds: the EHR-scoped ones, the
/// ehr-less ones, or none when the service is not a versioned-content service
/// (statistics are then trivially empty/0).
///
/// `a_service` names a versioned-content service. Only `Ehr` (EHR-scoped) and
/// `Demographic` (ehr-less) hold contributions in this CDR; the remaining
/// members (`Admin`/`Definitions`/`Ehr_index`/`Message`/`Query`/`System_log`)
/// are not versioned-content services and yield nothing — a defensible reading,
/// not a gap.
fn contribution_scope(service: PlatformService) -> Option<ContributionScope> {
    match service {
        PlatformService::Ehr => Some(ContributionScope::Ehr),
        PlatformService::Demographic => Some(ContributionScope::Demographic),
        _ => None,
    }
}

impl FerroEhrService {
    /// The ids of all CONTRIBUTIONs of the named
    /// versioned-content service within the (optional) time range, ordered by
    /// commit time. A non-content service yields the empty list.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a bound of `time_range` is not a
    ///   valid ISO 8601 date-time, or the bounded pair has its lower bound
    ///   after its upper bound (BASE `Interval` invariant `Limits_consistent`).
    /// - `exception` — a database fault while querying.
    pub async fn admin_list_contributions(
        &self,
        a_service: PlatformService,
        time_range: StatTimeRange,
    ) -> Result<Vec<String>, ServiceError> {
        let (lo, hi) = super::parse_range(time_range)?;
        let Some(scope) = contribution_scope(a_service) else {
            return Ok(Vec::new());
        };
        Ok(statistics::contribution_ids(
            self.contribution_pool(scope),
            scope,
            lo.as_deref(),
            hi.as_deref(),
        )
        .await?)
    }

    /// The count of all CONTRIBUTIONs of the named
    /// service within the (optional) time range. A non-content service → 0.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a bound of `time_range` is not a
    ///   valid ISO 8601 date-time, or the bounded pair has its lower bound
    ///   after its upper bound (BASE `Interval` invariant `Limits_consistent`).
    /// - `exception` — a database fault while querying.
    pub async fn admin_contribution_count(
        &self,
        a_service: PlatformService,
        time_range: StatTimeRange,
    ) -> Result<i64, ServiceError> {
        let (lo, hi) = super::parse_range(time_range)?;
        let Some(scope) = contribution_scope(a_service) else {
            return Ok(0);
        };
        Ok(statistics::contribution_count(
            self.contribution_pool(scope),
            scope,
            lo.as_deref(),
            hi.as_deref(),
        )
        .await?)
    }

    /// The pool holding the CONTRIBUTIONs of `scope`: the clinical one for the
    /// EHR-scoped ones, the demographic one otherwise.
    ///
    /// The two pseudonymisation domains keep their own `contribution` and
    /// `audit` relations, so a statistic over demographic change control is
    /// answered where that change control lives. No openEHR spec governs
    /// storage layout — our own design/extension.
    fn contribution_pool(&self, scope: ContributionScope) -> &sqlx::PgPool {
        match scope {
            ContributionScope::Ehr => &self.pool,
            ContributionScope::Demographic => &self.demographic_pool,
        }
    }

    /// The COMPOSITION count `what` names within the (optional) range: the
    /// distinct versioned COMPOSITIONs with a version committed in it, or the
    /// COMPOSITION version rows committed in it.
    ///
    /// COMPOSITIONs are EHR-scoped, so only `a_service = Ehr` yields a
    /// non-zero count; every other member → 0 (COMPOSITIONs are not in its
    /// scope).
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a bound of `time_range` is not a
    ///   valid ISO 8601 date-time, or the bounded pair has its lower bound
    ///   after its upper bound (BASE `Interval` invariant `Limits_consistent`).
    /// - `exception` — a database fault while querying.
    pub async fn composition_count(
        &self,
        a_service: PlatformService,
        time_range: StatTimeRange,
        what: CompositionCount,
    ) -> Result<i64, ServiceError> {
        let (lo, hi) = super::parse_range(time_range)?;
        if a_service != PlatformService::Ehr {
            return Ok(0);
        }
        Ok(statistics::composition_count(&self.pool, what, lo.as_deref(), hi.as_deref()).await?)
    }
}
