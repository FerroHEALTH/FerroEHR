// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The review and the verification of both logs' sealed windows (#6242).
//!
//! The review reads the windows back; the verification recomputes each from
//! its records in paced steps. No specification governs it: our own design.

use sqlx::Connection;

use super::{Log, Window, digest, window_records};
use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use crate::system_log::store::chains::AuditChain;

/// A sealed window as the review reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SealView {
    /// The window's number.
    pub window_no: i64,
    /// The first entry id it covers.
    pub first_id: i64,
    /// The last entry id it covers.
    pub last_id: i64,
    /// How many entries it held when sealed.
    pub entries: i32,
    /// The seal digest before it, lower-case hex.
    pub previous_digest: String,
    /// Its seal digest, lower-case hex.
    pub digest: String,
    /// Whether the fold removed its entries.
    pub folded: bool,
}

/// A row of the seals and the fold, as [`Queries::seals`] and
/// [`Queries::next_seals`] select it.
type SealRow = (i64, i64, i64, i32, String, String, bool);

impl AccessLogStore {
    /// The seals of `log`'s windows that cover an id from `from` to `to`,
    /// both included, in window order.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn seals(
        &self,
        log: &Log,
        from: i64,
        to: i64,
    ) -> Result<Vec<SealView>, AccessLogError> {
        let rows: Vec<SealRow> = sqlx::query_as(log.sql.seals)
            .bind(from)
            .bind(to)
            .fetch_all(&mut *acquire(self.pool()).await?)
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(window_no, first_id, last_id, entries, previous_digest, digest, folded)| {
                    SealView {
                        window_no,
                        first_id,
                        last_id,
                        entries,
                        previous_digest,
                        digest,
                        folded,
                    }
                },
            )
            .collect())
    }
}

/// A stored seal, as the verifier reads it.
struct StoredSeal {
    window_no: i64,
    first_id: i64,
    last_id: i64,
    entries: i32,
    previous: String,
    digest: String,
    folded: bool,
}

/// The stored windows of `log` after window `after` that start before
/// `budget` entries past it, at least one, read on `conn`.
async fn next_seals(
    conn: &mut sqlx::PgConnection,
    log: &Log,
    after: i64,
    budget: i64,
) -> Result<Vec<StoredSeal>, AccessLogError> {
    let rows: Vec<SealRow> = sqlx::query_as(log.sql.next_seals)
        .bind(after)
        .bind(budget)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(window_no, first_id, last_id, entries, previous, digest, folded)| StoredSeal {
                window_no,
                first_id,
                last_id,
                entries,
                previous,
                digest,
                folded,
            },
        )
        .collect())
}

/// Check one stored window of `log` against its rows on `conn`: a folded
/// window holds no record, and any other recomputes to its seal with every
/// record as it was written; each failure is pushed to `findings`.
async fn check_window(
    conn: &mut sqlx::PgConnection,
    log: &Log,
    seal: &StoredSeal,
    findings: &mut Vec<crate::system_log::store::chains::AuditChainFinding>,
) -> Result<(), AccessLogError> {
    let held = window_records(conn, log, seal.first_id.saturating_sub(1), seal.last_id).await?;
    if seal.folded {
        if !held.is_empty() {
            findings.push(finding(
                log,
                Some(seal.first_id),
                format!(
                    "window {} was folded and holds records again",
                    seal.window_no
                ),
            ));
        }
        return Ok(());
    }
    let modified = match log.chain {
        AuditChain::AccessLog => crate::system_log::store::chain_walk::modified_between(
            &mut *conn,
            seal.first_id.saturating_sub(1),
            seal.last_id,
        )
        .await
        .map(|ids| {
            ids.into_iter()
                .map(|id| (id, None, None))
                .collect::<Vec<_>>()
        }),
        AuditChain::System => crate::system_log::store::chain_walk::system_modified_between(
            &mut *conn,
            seal.first_id.saturating_sub(1),
            seal.last_id,
        )
        .await
        .map(|found| {
            found
                .into_iter()
                .map(|(seq, id, at)| (seq, Some(id), Some(at)))
                .collect()
        }),
    }
    .map_err(|error| AccessLogError::Chain(Box::new(error)))?;
    for (id, record_id, recorded_at) in modified {
        let mut changed = finding(
            log,
            Some(id),
            "record content was modified after it was written".to_owned(),
        );
        changed.record_id = record_id;
        changed.recorded_at = recorded_at;
        findings.push(changed);
    }
    let window = Window {
        tag: log.tag,
        number: seal.window_no,
        first_id: seal.first_id,
        last_id: seal.last_id,
        previous: &seal.previous,
    };
    if usize::try_from(seal.entries).ok() != Some(held.len())
        || digest(window, &held) != seal.digest
    {
        findings.push(finding(
            log,
            Some(seal.first_id),
            format!(
                "window {} does not match its seal: a record was changed, removed or inserted",
                seal.window_no
            ),
        ));
    }
    Ok(())
}

/// What one verification step found and where it ended.
#[derive(Debug, Clone)]
pub struct WindowStep {
    /// What the windows of the step fail.
    pub findings: Vec<crate::system_log::store::chains::AuditChainFinding>,
    /// The last window the step read; the window it started after when it
    /// read none.
    pub through: i64,
    /// Whether the step reached the sealer's last window.
    pub done: bool,
}

/// A finding about `log` at `chain_seq`.
fn finding(
    log: &Log,
    chain_seq: Option<i64>,
    text: String,
) -> crate::system_log::store::chains::AuditChainFinding {
    crate::system_log::store::chains::AuditChainFinding {
        chain: log.chain,
        chain_seq,
        record_id: None,
        recorded_at: None,
        finding: text,
    }
}

/// The render policies, search tokens and search sets that no longer digest
/// to their key, and the records naming a policy the store lacks: the shared
/// rows the system records name, checked on the last step of a walk of the
/// system records, each with the position it is about, if any.
const SHARED_ROWS_CHANGED: &str = "SELECT NULL::bigint, 'the render policy ' \
     || encode(p.digest, 'hex') || ' was modified after it was written' \
     FROM audit.audit_render_policy p \
     WHERE p.digest IS DISTINCT FROM sha256(convert_to(p.policy::text, 'UTF8')) \
     UNION ALL SELECT NULL::bigint, 'the search token ' || t.id || ' was modified after it was written' \
     FROM audit.audit_search_token t \
     WHERE t.digest IS DISTINCT FROM audit.search_token_digest(t.parameter, t.system, t.code) \
     UNION ALL SELECT NULL::bigint, 'the search set ' || s.id || ' was modified after it was written' \
     FROM audit.audit_search_set s \
     WHERE s.digest IS DISTINCT FROM audit.search_set_digest(s.tokens) \
     UNION ALL SELECT e.seq, 'the render policy the record names is missing' \
     FROM audit.audit_event e WHERE NOT EXISTS \
     (SELECT 1 FROM audit.audit_render_policy p WHERE p.digest = e.render_policy)";

/// One verification step of `log` on `conn`, which moves no cursor.
///
/// The step reads the windows after window `after` up to a budget of
/// `budget` entries, each
/// recomputed from its rows with every record checked as it was written and
/// each seal followed from the one before it; when the step reaches the last
/// window, the seal state, and for the system records the shared rows they
/// name.
///
/// # Errors
/// The store errors when a statement fails, which is never a pass.
pub async fn verify_on(
    conn: &mut sqlx::PgConnection,
    log: &Log,
    after: i64,
    budget: i64,
) -> Result<WindowStep, AccessLogError> {
    let (mut previous, mut last_id): (String, i64) = if after == 0 {
        (sha256_hex(log.tag), 0)
    } else {
        sqlx::query_as(log.sql.seal_at)
            .bind(after)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or_else(|| (String::new(), 0))
    };
    let seals = next_seals(&mut *conn, log, after, budget).await?;
    let mut findings = Vec::new();
    let mut through = after;
    for seal in &seals {
        if seal.window_no != through.saturating_add(1)
            || seal.first_id != last_id.saturating_add(1)
            || seal.previous != previous
        {
            findings.push(finding(
                log,
                Some(seal.first_id),
                format!(
                    "seal {} does not follow the seal before it: a seal was removed or changed",
                    seal.window_no
                ),
            ));
        }
        check_window(&mut *conn, log, seal, &mut findings).await?;
        through = seal.window_no;
        last_id = seal.last_id;
        previous.clone_from(&seal.digest);
    }
    let more: bool = sqlx::query_scalar(log.sql.more)
        .bind(through)
        .fetch_one(&mut *conn)
        .await?;
    if !more {
        let state: Option<(i64, String)> = sqlx::query_as(log.sql.state)
            .fetch_optional(&mut *conn)
            .await?;
        let ends_here = state.as_ref().is_some_and(|(window_no, digest)| {
            *window_no == through && (through == 0 || *digest == previous)
        });
        if !ends_here {
            findings.push(finding(
                log,
                None,
                "the seal state does not name the last seal: seals were removed from the end"
                    .to_owned(),
            ));
        }
        if log.chain == AuditChain::System {
            let changed: Vec<(Option<i64>, String)> = sqlx::query_as(SHARED_ROWS_CHANGED)
                .fetch_all(&mut *conn)
                .await?;
            findings.extend(
                changed
                    .into_iter()
                    .map(|(position, text)| finding(log, position, text)),
            );
        }
    }
    Ok(WindowStep {
        findings,
        through,
        done: !more,
    })
}

impl AccessLogStore {
    /// The window an incremental verification of `log` starts after: the
    /// newest one an earlier run found intact with every window before it, 0
    /// before the first.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn verified_window(&self, log: &Log) -> Result<i64, AccessLogError> {
        let through: Option<i64> = sqlx::query_scalar(log.sql.verified)
            .fetch_optional(&mut *acquire(self.pool()).await?)
            .await?;
        Ok(through.unwrap_or(0))
    }

    /// Verify the windows of `log` after window `after`, until they hold at
    /// least `budget` entries or the sealer's last window is reached: each
    /// window recomputed from its rows, every record checked as it was
    /// written, and each seal followed from the one before it; the last step
    /// also checks the sealer's state, and for the system records the shared
    /// rows they name. When `advance` and the step is intact, the
    /// verification cursor moves to its last window, and `full` marks a run
    /// from the first window.
    ///
    /// # Errors
    /// The store errors when a statement fails, which is never a pass.
    pub async fn verify_window_step(
        &self,
        log: &Log,
        after: i64,
        budget: i64,
        full: bool,
        advance: bool,
    ) -> Result<WindowStep, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        // NOTE: our own design (#6242), one snapshot for the step, so a window the sealer adds
        // meanwhile never reads as a seal state that names no last seal.
        let mut snapshot = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        let step = verify_on(&mut snapshot, log, after, budget).await?;
        // NOTE: our own design (#5762), a read-only snapshot of the verifier, no request's write.
        snapshot.commit().await?;
        if advance && step.findings.is_empty() && step.through > 0 {
            sqlx::query(log.sql.set_verified)
                .bind(step.through)
                .bind(full)
                .bind(step.done)
                .execute(&mut *conn)
                .await?;
        }
        Ok(step)
    }

    /// Verify every sealed window of `log`, from the first, or with
    /// `since_verified` after the newest window an earlier run found intact,
    /// in steps of 20,000 entries, each moving the cursor while
    /// every step before it was intact.
    ///
    /// # Errors
    /// As [`Self::verify_window_step`].
    pub async fn verify_windows(
        &self,
        log: &Log,
        since_verified: bool,
    ) -> Result<Vec<crate::system_log::store::chains::AuditChainFinding>, AccessLogError> {
        let mut after = if since_verified {
            self.verified_window(log).await?
        } else {
            0
        };
        let mut findings = Vec::new();
        loop {
            let step = self
                .verify_window_step(
                    log,
                    after,
                    STEP_ENTRIES,
                    !since_verified,
                    findings.is_empty(),
                )
                .await?;
            findings.extend(step.findings);
            after = step.through;
            if step.done {
                return Ok(findings);
            }
        }
    }
}

/// The entries one step of [`AccessLogStore::verify_windows`] reads at
/// most, past the first window.
const STEP_ENTRIES: i64 = 20_000;
