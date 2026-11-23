// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The search index of the access log (#6319).
//!
//! The columns its readers filter and order on are copied from each entry
//! into `audit.access_log_index` in one batch per frontier round, so the
//! request that writes an entry maintains no index but its key and uid.
//!
//! A reader reads the indexed entries through the index and the entries past
//! its cursor by key, in one statement (`audit.access_log_record`,
//! `audit.access_log_search`). The cursor commits with the rows it covers and
//! only passes ids below the frontier, so every committed entry is in one of
//! the two. A batch holds nothing the entries' rows do not, so it commits
//! without waiting for its flush: a batch a crash loses leaves its cursor
//! where it was, and the next round copies the entries again. No
//! specification governs audit storage: our own design.

use sqlx::Connection;

use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::db::acquire::acquire;

/// The most entry ids one batch copies.
const BATCH: i64 = 50_000;

/// The lock that orders a batch with the replacement of an indexed entry by
/// its tombstone (`audit.access_log_entry_indexed`, which takes it shared).
/// It is its own statement, so the copy after it reads a snapshot taken once
/// the lock is granted and sees every replacement committed before.
const LOCK: &str = "SELECT pg_advisory_xact_lock(hashtext('audit.access_log_index'), 0)";

/// One batch: the entries past the cursor up to `$1`, below every entry a
/// write holds and at most `$2` ids, copied into the index, and the cursor
/// moved with them; returns the cursor. `$1` is the frontier, below which no
/// entry commits any more, so no entry lands under the cursor after it moved.
const COPY: &str = "WITH s AS (SELECT x.last_id FROM audit.access_log_index_state x \
                  WHERE x.singleton FOR UPDATE), \
     b AS (SELECT s.last_id AS after, \
                  least($1::bigint, \
                        coalesce((SELECT min(h.entry_id) - 1 FROM audit.access_log_hold h), \
                                 $1::bigint), \
                        s.last_id + $2::bigint) AS upto \
             FROM s), \
     copied AS ( \
       INSERT INTO audit.access_log_index (entry_id, time_recorded, ehr_id, party_id, \
                                           retain_until, render_policy_id, patient_id, principal, \
                                           search_keys) \
       SELECT e.id, e.time_recorded, e.ehr_id, e.party_id, e.retain_until, \
              e.render_policy_id, e.patient_id, e.principal, \
              audit.search_keys(e.search_tokens, e.request_id, e.resource_id) \
         FROM b JOIN audit.access_log_entry e ON e.id > b.after AND e.id <= b.upto \
        ORDER BY e.id \
       RETURNING 1), \
     moved AS (UPDATE audit.access_log_index_state x SET last_id = b.upto FROM b \
                WHERE x.singleton AND b.upto > b.after RETURNING x.last_id) \
     SELECT coalesce((SELECT last_id FROM moved), (SELECT after FROM b))";

impl AccessLogStore {
    /// Copy the entries past the index's cursor up to `upto`, the access
    /// log's frontier, and below every entry a write holds into the index,
    /// batch after batch; returns the cursor.
    ///
    /// # Errors
    /// The store errors when a statement fails; the cursor then stays where
    /// the last committed batch left it.
    pub async fn index_to(&self, upto: i64) -> Result<i64, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut cursor = i64::MIN;
        loop {
            let mut tx = conn.begin().await?;
            // NOTE: our own design (#6319), the index is copied again from the cursor after a
            // crash, so its batch does not wait for the flush.
            sqlx::query("SET LOCAL synchronous_commit = off")
                .execute(&mut *tx)
                .await?;
            sqlx::query(LOCK).execute(&mut *tx).await?;
            let reached: i64 = sqlx::query_scalar(COPY)
                .bind(upto)
                .bind(BATCH)
                .fetch_one(&mut *tx)
                .await?;
            // NOTE: our own design (#5762), the chainer's own round, which no request makes.
            tx.commit().await?;
            if reached >= upto || reached <= cursor {
                return Ok(reached);
            }
            cursor = reached;
        }
    }
}
