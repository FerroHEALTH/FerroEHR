// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether a clinical statement can read the party store's revisions, and a
//! party statement the policy and definition revisions (#6320).
//!
//! Where the party domain shares the clinical database, the party migrations
//! open one door into it, `party_door.party_revisions(held)`, which the clinical
//! roles may run, so a read confirms its party copies in its own clinical
//! statement (`party_binding::request::ride`), and the clinical migrations
//! open the other, `clinical_door.policy_revisions()`, which the party roles
//! may run, so a party read confirms its policy copies in its own party
//! statement (`policy_register::observed::through_clinical_door`). Where the
//! party domain lives in a database of its own, neither reaches the other,
//! and a request reads the revisions in a statement of its own on the other
//! pool. The topology is read once per service. No openEHR spec governs the
//! confirmation: our own design.

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

use super::FerroEhrService;

/// The database a pool serves, read on the pool that runs no door: a role
/// without `USAGE` on a door's schema cannot name its function.
const DATABASE_SQL: &str =
    "SELECT s.system_identifier::text, current_database()::text FROM pg_control_system() s";

/// The database a pool serves, and on the clinical pool whether its role may
/// run the door.
const WHERE_SQL: &str = "SELECT s.system_identifier::text, current_database()::text, \
     COALESCE(has_function_privilege(to_regprocedure('party_door.party_revisions(pg_snapshot)'), 'EXECUTE'), \
     false) FROM pg_control_system() s";

impl FerroEhrService {
    /// Whether this service's clinical statements can read the party store's
    /// revisions: the clinical and party pools serve one database, and the
    /// clinical role may run `party_door.party_revisions(held)`. Read once; a
    /// failed read is not kept.
    ///
    /// # Errors
    /// [`StorageError`] when either pool does not answer.
    pub async fn party_door_open(&self) -> Result<bool, StorageError> {
        // NOTE: our own design (#6027), a request waiting on another's first read keeps no
        // connection meanwhile.
        crate::db::acquire::awaiting(self.party_door.get_or_try_init(|| async {
            let (clinical, clinical_db, door): (String, String, bool) = sqlx::query_as(WHERE_SQL)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            let (party, party_db): (String, String) = sqlx::query_as(DATABASE_SQL)
                .fetch_one(&mut *acquire(&self.demographic_pool).await?)
                .await?;
            Ok(door && clinical == party && clinical_db == party_db)
        }))
        .await
        .copied()
    }
}

/// The database a pool serves, and on the party pool whether its role may
/// run the clinical door.
const CLINICAL_DOOR_SQL: &str = "SELECT s.system_identifier::text, current_database()::text, \
     COALESCE(has_function_privilege(to_regprocedure('clinical_door.policy_revisions()'), \
     'EXECUTE'), false) FROM pg_control_system() s";

impl FerroEhrService {
    /// Whether this service's party statements can read the policy and
    /// definition revisions: the party and clinical pools serve one database,
    /// and the party role may run `clinical_door.policy_revisions()`. Read
    /// once; a failed read is not kept.
    ///
    /// # Errors
    /// [`StorageError`] when either pool does not answer.
    pub async fn clinical_door_open(&self) -> Result<bool, StorageError> {
        // NOTE: our own design (#6027), a request waiting on another's first read keeps no
        // connection meanwhile.
        crate::db::acquire::awaiting(self.clinical_door.get_or_try_init(|| async {
            let (party, party_db, door): (String, String, bool) = sqlx::query_as(CLINICAL_DOOR_SQL)
                .fetch_one(&mut *acquire(&self.demographic_pool).await?)
                .await?;
            let (clinical, clinical_db): (String, String) = sqlx::query_as(DATABASE_SQL)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            Ok(door && clinical == party && clinical_db == party_db)
        }))
        .await
        .copied()
    }

    /// `fut`, statements on the party pool, run so that a statement of it that
    /// confirms the request's policy copies reads the revisions through the
    /// clinical door where it is open ([`Self::clinical_door_open`]).
    ///
    /// # Errors
    /// [`StorageError`] when the door's first probe finds no pool answering.
    pub(crate) async fn on_party_pool<F: Future>(&self, fut: F) -> Result<F::Output, StorageError> {
        let open = self.clinical_door_open().await?;
        Ok(crate::policy_register::observed::through_clinical_door(open, fut).await)
    }
}

impl FerroEhrService {
    /// Whether the ride `ridden` may be answered, judged against the party
    /// store on this service's party pool when no statement of it read the
    /// revisions ([`crate::party_binding::request::Ridden::held`]).
    ///
    /// # Errors
    /// [`StorageError`] when the party store does not answer.
    pub async fn ride_held(
        &self,
        ridden: crate::party_binding::request::Ridden,
    ) -> Result<bool, StorageError> {
        ridden.held(&self.demographic_pool).await
    }
}
