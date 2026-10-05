// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Admin statistics (SM `I_ADMIN_SERVICE.list_contributions` /
//! `contribution_count` / `versioned_composition_count` /
//! `composition_version_count`).
//!
//! Spec: `docs/specs/openehr/SM/docs/UML/classes/i_admin_service.adoc` — each
//! call takes a `PLATFORM_SERVICE` (`a_service`, "Name of a versioned content
//! service") and an optional `Interval<Iso8601_date_time>` matched against the
//! CONTRIBUTION / version audit `time_committed`. The enumeration members are
//! `platform_service.adoc`. No openEHR spec governs the SQL that answers these —
//! our own design over the greenfield `contribution` / `version` /
//! `commit_audit` tables (`clinical/0003_change_control.sql`).

use crate::service::FerroEhrService;
use crate::service::admin::types::StatTimeRange;
use crate::service::error::ServiceError;
use crate::service::platform_service::PlatformService;
use crate::service::status::SmError;

/// Whether a [`PlatformService`]'s CONTRIBUTIONs are EHR-scoped (`Some(true)` →
/// `ehr_id IS NOT NULL`), ehr-less (`Some(false)` → `ehr_id IS NULL`), or the
/// service is not a versioned-content service (`None` → statistics are trivially
/// empty/0).
///
/// NOTE (already-correct — `platform_service.adoc`): `a_service` is
/// "Name of a versioned content service". Only `Ehr` (EHR-scoped) and
/// `Demographic` (ehr-less) hold contributions in this CDR; the remaining
/// members (`Admin`/`Definitions`/`Ehr_index`/`Message`/`Query`/`System_log`)
/// are not versioned-content services and yield nothing — a defensible reading,
/// not a gap. Returned as a bool so the SQL stays static (parameterized), never
/// string-built.
fn contribution_ehr_scoped(service: PlatformService) -> Option<bool> {
    match service {
        PlatformService::Ehr => Some(true),
        PlatformService::Demographic => Some(false),
        _ => None,
    }
}

impl FerroEhrService {
    /// SM `list_contributions`: the ids of all CONTRIBUTIONs of the named
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
    ) -> Result<Vec<String>, SmError> {
        let (lo, hi) = super::parse_range(time_range)?;
        let Some(ehr_scoped) = contribution_ehr_scoped(a_service) else {
            return Ok(Vec::new());
        };
        // Two static shapes selected in Rust (the in-SQL `$3 AND … OR …`
        // scoping resisted index planning at the measured corpus scale —
        // see admin_contribution_count). The listing always joins `audit`:
        // its ORDER BY is the commit time.
        let sql = if ehr_scoped {
            "SELECT c.id::text FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NOT NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz) \
             ORDER BY a.time_committed, c.id"
        } else {
            "SELECT c.id::text FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
             WHERE c.ehr_id IS NULL \
               AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
               AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz) \
             ORDER BY a.time_committed, c.id"
        };
        let ids: Vec<String> = sqlx::query_scalar(sql)
            .bind(lo)
            .bind(hi)
            .fetch_all(self.contribution_pool(ehr_scoped))
            .await
            .map_err(ServiceError::from)?;
        Ok(ids)
    }

    /// SM `contribution_count`: the count of all CONTRIBUTIONs of the named
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
    ) -> Result<i64, SmError> {
        let (lo, hi) = super::parse_range(time_range)?;
        let Some(ehr_scoped) = contribution_ehr_scoped(a_service) else {
            return Ok(0);
        };
        // Two static shapes, selected in Rust rather than by an in-SQL OR: a
        // `($3 AND …) OR (NOT $3 AND …)` scoping predicate resists index
        // planning. Unbounded takes an index-only count over
        // `idx_contribution_ehr_id`, bounded the audit join behind
        // `idx_commit_audit_time_committed`
        // (clinical/0003_change_control.sql).
        let count: i64 = if lo.is_none() && hi.is_none() {
            let sql = if ehr_scoped {
                "SELECT count(*) FROM contribution WHERE ehr_id IS NOT NULL"
            } else {
                "SELECT count(*) FROM contribution WHERE ehr_id IS NULL"
            };
            sqlx::query_scalar(sql)
                .fetch_one(self.contribution_pool(ehr_scoped))
                .await
                .map_err(ServiceError::from)?
        } else {
            let sql = if ehr_scoped {
                "SELECT count(*) FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
                 WHERE c.ehr_id IS NOT NULL \
                   AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
                   AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)"
            } else {
                "SELECT count(*) FROM contribution c JOIN commit_audit a ON a.id = c.commit_audit_id \
                 WHERE c.ehr_id IS NULL \
                   AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
                   AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)"
            };
            sqlx::query_scalar(sql)
                .bind(lo)
                .bind(hi)
                .fetch_one(self.contribution_pool(ehr_scoped))
                .await
                .map_err(ServiceError::from)?
        };
        Ok(count)
    }

    /// The pool holding the CONTRIBUTIONs of a service: the clinical one for an
    /// EHR-scoped service, the demographic one otherwise.
    ///
    /// The two pseudonymisation domains keep their own `contribution` and
    /// `audit` relations, so a statistic over demographic change control is
    /// answered where that change control lives. No openEHR spec governs
    /// storage layout — our own design/extension.
    fn contribution_pool(&self, ehr_scoped: bool) -> &sqlx::PgPool {
        if ehr_scoped {
            &self.pool
        } else {
            &self.demographic_pool
        }
    }

    /// SM `versioned_composition_count`: the count of distinct COMPOSITION
    /// versioned objects with a version committed within the (optional) range.
    ///
    /// NOTE (already-correct — `platform_service.adoc`): COMPOSITIONs are
    /// EHR-scoped, so only `a_service = Ehr` yields a non-zero count; every
    /// other member → 0 (COMPOSITIONs are not in its scope).
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a bound of `time_range` is not a
    ///   valid ISO 8601 date-time, or the bounded pair has its lower bound
    ///   after its upper bound (BASE `Interval` invariant `Limits_consistent`).
    /// - `exception` — a database fault while querying.
    pub async fn versioned_composition_count(
        &self,
        a_service: PlatformService,
        time_range: StatTimeRange,
    ) -> Result<i64, SmError> {
        let (lo, hi) = super::parse_range(time_range)?;
        if a_service != PlatformService::Ehr {
            return Ok(0);
        }
        // Unbounded: no reason to touch `audit` at all.
        let count: i64 = if lo.is_none() && hi.is_none() {
            sqlx::query_scalar(
                "SELECT count(DISTINCT vo_id) FROM version WHERE kind = 'COMPOSITION'",
            )
            .fetch_one(&self.pool)
            .await
            .map_err(ServiceError::from)?
        } else {
            sqlx::query_scalar(
                "SELECT count(DISTINCT v.vo_id) FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id \
                 WHERE v.kind = 'COMPOSITION' \
                   AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
                   AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)",
            )
            .bind(lo)
            .bind(hi)
            .fetch_one(&self.pool)
            .await
            .map_err(ServiceError::from)?
        };
        Ok(count)
    }

    /// SM `composition_version_count`: the count of individual COMPOSITION
    /// version rows committed within the (optional) range. Scope gate as
    /// [`Self::versioned_composition_count`].
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a bound of `time_range` is not a
    ///   valid ISO 8601 date-time, or the bounded pair has its lower bound
    ///   after its upper bound (BASE `Interval` invariant `Limits_consistent`).
    /// - `exception` — a database fault while querying.
    pub async fn composition_version_count(
        &self,
        a_service: PlatformService,
        time_range: StatTimeRange,
    ) -> Result<i64, SmError> {
        let (lo, hi) = super::parse_range(time_range)?;
        if a_service != PlatformService::Ehr {
            return Ok(0);
        }
        let count: i64 = if lo.is_none() && hi.is_none() {
            sqlx::query_scalar("SELECT count(*) FROM version WHERE kind = 'COMPOSITION'")
                .fetch_one(&self.pool)
                .await
                .map_err(ServiceError::from)?
        } else {
            sqlx::query_scalar(
                "SELECT count(*) FROM version v JOIN commit_audit a ON a.id = v.commit_audit_id \
                 WHERE v.kind = 'COMPOSITION' \
                   AND ($1::timestamptz IS NULL OR a.time_committed >= $1::timestamptz) \
                   AND ($2::timestamptz IS NULL OR a.time_committed <= $2::timestamptz)",
            )
            .bind(lo)
            .bind(hi)
            .fetch_one(&self.pool)
            .await
            .map_err(ServiceError::from)?
        };
        Ok(count)
    }
}
