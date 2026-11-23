// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The search index of the access log (#6319, #6322).
//!
//! The columns its readers filter and order on are copied from each entry
//! into `audit.access_log_index` in one batch per frontier round, with the
//! entry's ITI-81 search tokens, which the round renders from the stored
//! entry by its stored form's renderer, so the request that writes an entry
//! renders no projection and maintains no index but its key and uid
//! (`crate::system_log::store::AuditStore::index_access_log`).
//!
//! A reader reads the indexed entries through the index and the entries past
//! its cursor by key, in one statement (`audit.access_log_record`,
//! `audit.access_log_search`). The cursor commits with the rows it covers and
//! only passes ids below the frontier, so every committed entry is in one of
//! the two. An entry past the cursor carries no search tokens, so a search on
//! a token parameter moves the frontier and the index past every entry
//! committed before it first. A batch holds nothing the entries' rows and
//! the render policies do not, so it commits without waiting for its flush: a
//! batch a crash loses leaves its cursor where it was, and the next round
//! renders and copies the entries again. No specification governs audit
//! storage: our own design.

/// The most entry ids one batch copies: each is rendered in the batch.
pub(crate) const BATCH: i64 = 2_000;

/// The lock that orders a batch with the replacement of an indexed entry by
/// its tombstone (`audit.access_log_entry_indexed`, which takes it shared).
/// It is its own statement, so the reads after it take a snapshot once the
/// lock is granted and see every replacement committed before.
pub(crate) const LOCK: &str = "SELECT pg_advisory_xact_lock(hashtext('audit.access_log_index'), 0)";

/// The cursor, locked, and the last id of the batch after it: up to `$1`,
/// the frontier, below every entry a write holds and at most `$2` ids on.
pub(crate) const BOUNDS: &str = "SELECT s.last_id, \
       least($1::bigint, \
             coalesce((SELECT min(h.entry_id) - 1 FROM audit.access_log_hold h), $1::bigint), \
             s.last_id + $2::bigint) \
  FROM audit.access_log_index_state s WHERE s.singleton FOR UPDATE";

/// The cursor, read without a lock.
pub(crate) const CURSOR: &str =
    "SELECT s.last_id FROM audit.access_log_index_state s WHERE s.singleton";

/// The entries of the batch `($1, $2]`, as the renderer reads them, with the
/// digest of the render policy an indexed entry is written under.
#[cfg(feature = "fhir")]
pub(crate) const ROWS: &str = "SELECT e.id AS chain_seq, e.content AS row_part, \
       sh.content AS shared_part, e.record_type AS entry_type, e.uid AS entry_uid, \
       e.time_recorded AS recorded_at, e.ehr_id AS entry_ehr, e.request_id, e.resource_id, \
       e.form AS stored_form, p.digest AS render_policy \
  FROM audit.access_log_entry e \
  LEFT JOIN audit.access_log_shared sh ON sh.digest = e.shared \
  LEFT JOIN audit.audit_render_policy p ON p.id = e.render_policy_id \
 WHERE e.id > $1 AND e.id <= $2 ORDER BY e.id";

/// The digests of the render policies the entries of `($1, $2]` name.
pub(crate) const POLICIES: &str = "SELECT DISTINCT p.digest \
  FROM audit.access_log_entry e JOIN audit.audit_render_policy p ON p.id = e.render_policy_id \
 WHERE e.id > $1 AND e.id <= $2";

/// The index rows of the batch `($1, $2]`, each indexed entry with the search
/// tokens `$4` holds for it by the position of its id in `$3`, and the cursor
/// moved to `$2`.
pub(crate) const COPY: &str = "WITH t AS (SELECT * FROM unnest($3::bigint[], $4::text[]) AS t(id, tokens)), \
     copied AS ( \
       INSERT INTO audit.access_log_index (entry_id, time_recorded, ehr_id, party_id, \
                                           retain_until, render_policy_id, patient_id, principal, \
                                           search_tokens, search_keys) \
       SELECT e.id, e.time_recorded, e.ehr_id, e.party_id, e.retain_until, \
              e.render_policy_id, e.patient_id, e.principal, \
              CASE WHEN e.render_policy_id IS NOT NULL THEN coalesce(t.tokens, '{}')::integer[] END, \
              audit.search_keys(CASE WHEN e.render_policy_id IS NOT NULL \
                                     THEN coalesce(t.tokens, '{}')::integer[] END, \
                                e.request_id, e.resource_id) \
         FROM audit.access_log_entry e LEFT JOIN t ON t.id = e.id \
        WHERE e.id > $1 AND e.id <= $2 \
        ORDER BY e.id \
       RETURNING 1) \
     UPDATE audit.access_log_index_state x SET last_id = $2 \
      WHERE x.singleton AND $2 > x.last_id \
        AND (SELECT count(*) FROM copied) >= 0";
