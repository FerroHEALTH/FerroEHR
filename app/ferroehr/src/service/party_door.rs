// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether a clinical statement can read the party store's revisions (#6320).
//!
//! Where the party domain shares the clinical database, the party migrations
//! open one door into it, `party_door.party_revisions(held)`, which the clinical
//! roles may run, so a read confirms its party copies in its own clinical
//! statement (`party_binding::request::ride`). Where the party domain lives
//! in a database of its own, no clinical statement can reach it, and a
//! request reads the revisions in a statement of its own on the party pool.
//! The topology is read once per service. No openEHR spec governs the
//! confirmation: our own design.

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

use super::FerroEhrService;

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
            let (party, party_db, _): (String, String, bool) = sqlx::query_as(WHERE_SQL)
                .fetch_one(&mut *acquire(&self.demographic_pool).await?)
                .await?;
            Ok(door && clinical == party && clinical_db == party_db)
        }))
        .await
        .copied()
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
