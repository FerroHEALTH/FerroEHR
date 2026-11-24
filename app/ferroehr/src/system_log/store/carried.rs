// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What an ITI-81 page carries beside its records (#6320).
//!
//! The policy and definition revisions where the audit store shares the
//! clinical database, and in a ride the party store's, which the request
//! confirms its copies by. The topology is read once per service. No openEHR
//! spec governs the confirmation: our own design.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// What the page statement selects beside its records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Carried {
    /// The policy and definition revisions.
    pub policy: bool,
    /// In a ride, the party store's revisions beside them.
    pub party: bool,
}

/// The select item that reads, as one record, the policy and definition
/// revisions at the statement's own snapshot, and three NULLs where a ride's
/// party store revisions go ([`PARTY`]).
pub(super) const POLICY: &str = "(SELECT ROW(rr.policy, rr.definitions, NULL::bigint, \
     NULL::boolean, NULL::bigint) FROM clinical.request_revisions rr)";

/// The select item of [`POLICY`] with the party store's grant and binding
/// revisions and whether a caller facts stamp exists that the snapshot bound
/// at `$1` did not see, read through the party door (#6074).
pub(super) const PARTY: &str = "(SELECT ROW(rr.policy, rr.definitions, pr.grants, \
     pr.facts_behind, pr.bindings) FROM clinical.request_revisions rr, \
     party_door.party_revisions($1::pg_snapshot) pr)";

/// The column the page's first record carries the revisions in.
pub(super) const CARRIED: &str = "carried_revisions";

/// The record [`POLICY`] and [`PARTY`] read.
type Revisions = (i64, i64, Option<i64>, Option<bool>, Option<i64>);

/// The cluster and database a pool serves, and whether its role may read
/// `clinical.request_revisions` and run the party door there.
const WHERE_SQL: &str = "SELECT s.system_identifier::text, current_database()::text, \
     COALESCE(has_table_privilege(to_regclass('clinical.request_revisions'), 'SELECT'), false), \
     COALESCE(has_function_privilege(to_regprocedure('party_door.party_revisions(pg_snapshot)'), \
     'EXECUTE'), false) FROM pg_control_system() s";

impl Carried {
    /// What a page on the audit pool `audit` may carry, judged against the
    /// clinical pool `clinical`: where both serve one database, the policy
    /// revisions when the audit role may read them, and the party store's
    /// when `door` (the clinical statements' door is open) and the audit role
    /// may run it.
    ///
    /// # Errors
    /// [`StorageError`] when either pool does not answer.
    pub async fn probe(
        audit: &PgPool,
        clinical: &PgPool,
        door: bool,
    ) -> Result<Self, StorageError> {
        let (cluster, database, readable, runs): (String, String, bool, bool) =
            sqlx::query_as(WHERE_SQL)
                .fetch_one(&mut *acquire(audit).await?)
                .await?;
        let (clinical_cluster, clinical_database, _, _): (String, String, bool, bool) =
            sqlx::query_as(WHERE_SQL)
                .fetch_one(&mut *acquire(clinical).await?)
                .await?;
        let shared = cluster == clinical_cluster && database == clinical_database;
        Ok(Self {
            policy: shared && readable,
            party: shared && readable && door && runs,
        })
    }

    /// Notes the revisions the first record of `rows` carried for the
    /// running request.
    ///
    /// # Errors
    /// [`sqlx::Error`] when the carried record does not decode.
    pub(super) fn observe(self, rows: &[PgRow]) -> Result<(), sqlx::Error> {
        let Some(row) = rows.first().filter(|_| self.policy) else {
            return Ok(());
        };
        let Some((policy, definitions, grants, facts_behind, bindings)) =
            row.try_get::<Option<Revisions>, _>(CARRIED)?
        else {
            return Ok(());
        };
        crate::policy_register::observed::observe(policy, definitions);
        if let (Some(grants), Some(facts_behind), Some(bindings)) = (grants, facts_behind, bindings)
        {
            crate::party_binding::request::observe_door(grants, facts_behind, bindings);
        }
        Ok(())
    }

    /// Whether the page statement selects the party store's revisions: a
    /// ride's that has not read them yet.
    pub(super) fn reads_party(self) -> bool {
        self.party && crate::party_binding::request::reads_door()
    }
}
