// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What an ITI-81 page carries beside its records (#6320).
//!
//! Where the audit store shares the clinical database and its role may read
//! `clinical.request_revisions`, the page statement selects the policy and
//! definition revisions at its own snapshot, so the request confirms its
//! policy copies after it ran, by the revisions the statement read, and
//! makes no read of them first. Elsewhere the page carries nothing, and the
//! request reads them in a statement of their own. The topology is read once
//! per service. No openEHR spec governs the confirmation: our own design.

use sqlx::PgPool;
use sqlx::postgres::PgRow;

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// What the page statement selects beside its records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Carried {
    /// The policy and definition revisions, as `policy_revision` and
    /// `definition_revision`.
    pub policy: bool,
}

/// The select item that reads the policy revision at the statement's own
/// snapshot.
pub(super) const POLICY: &str = "(SELECT rr.policy FROM clinical.request_revisions rr)";

/// The select item that reads the definition revision.
pub(super) const DEFINITIONS: &str = "(SELECT rr.definitions FROM clinical.request_revisions rr)";

/// The cluster and database a pool serves, and whether its role may read
/// `clinical.request_revisions` there.
const WHERE_SQL: &str = "SELECT s.system_identifier::text, current_database()::text, \
     COALESCE(has_table_privilege(to_regclass('clinical.request_revisions'), 'SELECT'), false) \
     FROM pg_control_system() s";

impl Carried {
    /// What a page on the audit pool `audit` may carry, judged against the
    /// clinical pool `clinical`: the policy revisions where both serve one
    /// database and the audit role may read them.
    ///
    /// # Errors
    /// [`StorageError`] when either pool does not answer.
    pub async fn probe(audit: &PgPool, clinical: &PgPool) -> Result<Self, StorageError> {
        let (cluster, database, readable): (String, String, bool) = sqlx::query_as(WHERE_SQL)
            .fetch_one(&mut *acquire(audit).await?)
            .await?;
        let (clinical_cluster, clinical_database, _): (String, String, bool) =
            sqlx::query_as(WHERE_SQL)
                .fetch_one(&mut *acquire(clinical).await?)
                .await?;
        Ok(Self {
            policy: readable && cluster == clinical_cluster && database == clinical_database,
        })
    }

    /// Notes the revisions the first record of `rows` carried for the
    /// running request.
    ///
    /// # Errors
    /// [`sqlx::Error`] when a carried column does not decode.
    pub(super) fn observe(self, rows: &[PgRow]) -> Result<(), sqlx::Error> {
        match rows.first() {
            Some(row) if self.policy => crate::storage::access_projection::observe_revisions(row),
            _ => Ok(()),
        }
    }
}
