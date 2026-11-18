// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The verification of the hash chains in bounded steps (#5779).
//!
//! A walk of a chain takes the chain head when it starts and verifies the
//! chain up to it one window at a time. Each window holds at most a given
//! number of records and is one statement, which moves the verification
//! cursor to the window's end when it and every window before it in the walk
//! are intact, so no statement grows with the backlog and every step's
//! progress is kept. A walk that finds damage goes on to the head from the
//! stored links, so it reports every finding, and moves the cursor no more
//! (`migrations/audit/0041_chain_verification_steps.sql`). No openEHR spec
//! governs audit tamper detection: our own design.

use std::time::{Duration, Instant};

use jiff_sqlx::Timestamp;
use sha2::{Digest as _, Sha256};

use crate::access_log::stored_form::parts::{Cut, Derived};
use sqlx::Row;
use sqlx::postgres::{PgConnection, PgRow};

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope, finding};

/// How long one step of a walk aims to take: well under any statement
/// timeout, and short enough that a step holds the database briefly.
pub const STEP_BUDGET: Duration = Duration::from_millis(500);

/// The fewest records a step verifies, however slow the last one was.
pub const STEP_FLOOR: i32 = 500;

/// The most records a step verifies, however fast the last one was.
pub const STEP_CEILING: i32 = 100_000;

/// The records the first step of a walk verifies.
const STEP_FIRST: i32 = 5_000;

/// The size of a walk's next step, adapted to how long its last one took.
///
/// A step's cost per record grows with the load on the database, so a fixed
/// count can near the statement timeout (#5804). No openEHR spec governs the
/// verification schedule: our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepPace {
    records: i32,
}

impl Default for StepPace {
    fn default() -> Self {
        Self {
            records: STEP_FIRST,
        }
    }
}

impl StepPace {
    /// A pace whose first step verifies `records`, within [`STEP_FLOOR`]
    /// and [`STEP_CEILING`] after it.
    #[must_use]
    pub fn starting_at(records: i32) -> Self {
        Self {
            records: records.max(1),
        }
    }

    /// The records the next step verifies.
    #[must_use]
    pub fn records(self) -> i32 {
        self.records
    }

    /// Adapt to a step of [`Self::records`] that took `took`: toward
    /// [`STEP_BUDGET`], at most doubling, within [`STEP_FLOOR`] and
    /// [`STEP_CEILING`].
    pub fn took(&mut self, took: Duration) {
        let took = i128::try_from(took.as_micros()).unwrap_or(i128::MAX).max(1);
        let budget = i128::try_from(STEP_BUDGET.as_micros()).unwrap_or(i128::MAX);
        let records = i128::from(self.records);
        let next = records
            .saturating_mul(budget)
            .checked_div(took)
            .unwrap_or(i128::from(STEP_CEILING))
            .min(records.saturating_mul(2))
            .clamp(i128::from(STEP_FLOOR), i128::from(STEP_CEILING));
        // NOTE: our own design; the clamp above keeps it inside the i32 floor and ceiling.
        self.records = i32::try_from(next).unwrap_or(STEP_CEILING);
    }
}

/// How many entries one page of the content check reads.
const CONTENT_PAGE: i64 = 1000;

/// A verification walk of one chain in progress, from
/// [`AuditStore::chain_walk`] to the head it started with.
#[derive(Debug, Clone)]
pub struct ChainWalk {
    /// The chain walked.
    chain: AuditChain,
    /// Whether the walk started from the genesis.
    full: bool,
    /// The chain head the walk verifies up to; `None` when the chain state
    /// row is missing, which the walk's one step reports.
    head: Option<i64>,
    /// The position the walk has verified through.
    after: i64,
    /// The link at `after` in hexadecimal, the next window's anchor; `None`
    /// when no record or gap ends there any more.
    link: Option<String>,
    /// Whether every step so far was intact, so the next may move the cursor.
    intact: bool,
    /// Whether the walk has reached its head.
    done: bool,
}

impl ChainWalk {
    /// Whether the walk has reached the head it started with.
    #[must_use]
    pub fn done(&self) -> bool {
        self.done
    }

    /// The position the walk has verified through.
    #[must_use]
    pub fn position(&self) -> i64 {
        self.after
    }

    /// The chain head the walk verifies up to, `None` when the chain state
    /// row is missing.
    #[must_use]
    pub fn head(&self) -> Option<i64> {
        self.head
    }
}

/// How far a chain's verification cursor is behind its head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationLag {
    /// Chain positions past the cursor: the head minus the cursor.
    pub entries: i64,
    /// When the oldest record past the cursor was recorded; `None` when the
    /// cursor is at the head.
    pub oldest: Option<jiff::Timestamp>,
}

impl AuditStore {
    /// Start a verification walk of `chain` over `scope`: from the genesis
    /// for [`VerifyScope::Full`], from the cursor otherwise, to the chain
    /// head as it stands now.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn chain_walk(
        &self,
        chain: AuditChain,
        scope: VerifyScope,
    ) -> Result<ChainWalk, AuditError> {
        let full = scope == VerifyScope::Full;
        let row = sqlx::query(statements(chain).start)
            .bind(full)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(ChainWalk {
            chain,
            full,
            head: row.try_get("head_seq")?,
            after: row.try_get("after_seq")?,
            link: row.try_get("after_digest")?,
            intact: true,
            done: false,
        })
    }

    /// Verify the next window of `walk`, at most `records` records, and move
    /// the cursor to its end when it and every window before it are intact.
    /// The last window, which ends at the walk's head, also checks the head.
    /// The access log's entries in the window are digested again here first,
    /// since only the server writes their canonical form (#5703). Returns the
    /// window's findings; a walk that is done returns none.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails, which is itself a
    /// finding, never a pass; the steps before it keep their progress.
    pub async fn chain_step(
        &self,
        walk: &mut ChainWalk,
        records: i32,
    ) -> Result<Vec<AuditChainFinding>, AuditError> {
        if walk.done {
            return Ok(Vec::new());
        }
        let sql = statements(walk.chain);
        let mut conn = acquire(&self.pool).await?;
        let upto = match walk.head {
            Some(head) => {
                sqlx::query_scalar::<_, i64>(sql.step_end)
                    .bind(walk.after)
                    .bind(records)
                    .bind(head)
                    .fetch_one(&mut *conn)
                    .await?
            }
            None => walk.after,
        };
        let last = walk.head.is_none_or(|head| upto >= head);
        let step = match walk.chain {
            AuditChain::System => sqlx::query(sql.step),
            AuditChain::AccessLog => {
                sqlx::query(sql.step).bind(modified_between(&mut conn, walk.after, upto).await?)
            }
        };
        let rows = step
            .bind(walk.after)
            .bind(walk.link.as_deref())
            .bind(upto)
            .bind(walk.intact)
            .bind(last)
            .bind(walk.full)
            .fetch_all(&mut *conn)
            .await?;
        let findings = rows
            .iter()
            .map(|row| finding(walk.chain, row))
            .collect::<Result<Vec<_>, _>>()?;
        walk.intact &= findings.is_empty();
        walk.done = last;
        if !last {
            walk.link = sqlx::query_scalar(sql.link)
                .bind(upto)
                .fetch_one(&mut *conn)
                .await?;
            walk.after = upto;
        }
        Ok(findings)
    }

    /// [`Self::chain_step`] sized by `pace`, which then adapts to how long
    /// the step took; returns the step's findings and its duration.
    ///
    /// # Errors
    /// As [`Self::chain_step`].
    pub async fn chain_step_paced(
        &self,
        walk: &mut ChainWalk,
        pace: &mut StepPace,
    ) -> Result<(Vec<AuditChainFinding>, Duration), AuditError> {
        let started = Instant::now();
        let findings = self.chain_step(walk, pace.records()).await?;
        let took = started.elapsed();
        pace.took(took);
        Ok((findings, took))
    }

    /// How far `chain`'s verification cursor is behind its head.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn chain_lag(&self, chain: AuditChain) -> Result<VerificationLag, AuditError> {
        let row = sqlx::query("SELECT entries, oldest FROM audit.chain_verification_lag($1)")
            .bind(chain.as_str())
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(VerificationLag {
            entries: row.try_get("entries")?,
            oldest: row
                .try_get::<Option<Timestamp>, _>("oldest")?
                .map(Timestamp::to_jiff),
        })
    }

    /// The content check of the access log the database cannot run, since
    /// only the server writes an entry's canonical form (#5703): each entry
    /// after the verification cursor, or every entry when `full`, joined from
    /// its row and shared parts and digested again. Returns the newest
    /// position the check covered, `None` for an empty log, and the positions
    /// whose digest differs from their link's `content_digest`. It writes
    /// nothing, so it also serves a check that moves no cursor.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails.
    pub async fn access_log_contents(
        &self,
        full: bool,
    ) -> Result<(Option<i64>, Vec<i64>), AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let through: Option<i64> =
            sqlx::query_scalar("SELECT max(chain_seq) FROM audit.access_log_record")
                .fetch_one(&mut *conn)
                .await?;
        let Some(through) = through else {
            return Ok((None, Vec::new()));
        };
        let after: i64 = if full {
            0
        } else {
            sqlx::query_scalar(
                "SELECT coalesce((SELECT through_seq FROM audit.access_log_chain_verified \
                 WHERE singleton), 0)",
            )
            .fetch_one(&mut *conn)
            .await?
        };
        let modified = modified_between(&mut conn, after, through).await?;
        Ok((Some(through), modified))
    }
}

/// The statements of a walk over one chain.
struct Statements {
    /// Where the walk starts: `head_seq`, `after_seq`, `after_digest`.
    start: &'static str,
    /// Where the window after `$1` of at most `$2` records before head `$3`
    /// ends.
    step_end: &'static str,
    /// One step; the access log's takes the modified positions first.
    step: &'static str,
    /// The link at a position, the next window's anchor.
    link: &'static str,
}

/// The statements of a walk over `chain`.
const fn statements(chain: AuditChain) -> Statements {
    match chain {
        AuditChain::System => Statements {
            start: "SELECT head_seq, after_seq, after_digest \
                    FROM audit.audit_chain_verification_start($1)",
            step_end: "SELECT audit.audit_chain_verification_step_end($1, $2, $3)",
            step: "SELECT chain_seq, record_id, recorded_at, finding \
                   FROM audit.verify_audit_chain_step($1, $2, $3, $4, $5, $6)",
            link: "SELECT audit.audit_chain_verification_link($1)",
        },
        AuditChain::AccessLog => Statements {
            start: "SELECT head_seq, after_seq, after_digest \
                    FROM audit.access_log_verification_start($1)",
            step_end: "SELECT audit.access_log_verification_step_end($1, $2, $3)",
            step: "SELECT chain_seq, record_id, recorded_at, finding \
                   FROM audit.verify_access_log_chain_step($2, $3, $4, $1, $5, $6, $7)",
            link: "SELECT audit.access_log_verification_link($1)",
        },
    }
}

/// The positions in `(after, upto]` whose record was changed after it was
/// written, read a page at a time: an entry that no longer digests to its
/// link's `content_digest`, and an entry or tombstone whose derived columns
/// are not those its content decides (#6005).
async fn modified_between(
    conn: &mut PgConnection,
    mut after: i64,
    upto: i64,
) -> Result<Vec<i64>, AuditError> {
    let mut modified = Vec::new();
    loop {
        let rows = sqlx::query(concat!(
            "SELECT r.chain_seq, ",
            crate::access_log::store::entry_parts!(),
            ", r.content_digest, r.action_terminology, r.action_code, r.result_terminology, \
             r.result_code, r.party_id, r.retain_until, r.time_removed, r.fold_after \
             FROM audit.access_log_record r \
             LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared \
             WHERE r.chain_seq > $1 AND r.chain_seq <= $2 ORDER BY r.chain_seq LIMIT $3"
        ))
        .bind(after)
        .bind(upto)
        .bind(CONTENT_PAGE)
        .fetch_all(&mut *conn)
        .await?;
        let Some(last) = rows.last() else {
            return Ok(modified);
        };
        after = last.try_get("chain_seq")?;
        for row in &rows {
            if !record_intact(row)? {
                modified.push(row.try_get("chain_seq")?);
            }
        }
    }
}

/// Whether the record `row` holds is as it was written: its stored parts
/// join into a canonical form without a link, an entry's digests to its
/// `content_digest`, and every column the log derives from the content
/// holds what the content decides. A record whose parts no longer join was
/// changed after it was written.
fn record_intact(row: &PgRow) -> Result<bool, AuditError> {
    // NOTE: our own design (#6005); a record whose parts no longer join was changed after it was
    // written.
    let document = match crate::access_log::store::document_of(row) {
        Ok(document) => document,
        Err(crate::access_log::store::AccessLogError::Unreadable { .. }) => return Ok(false),
        Err(other) => return Err(other.into()),
    };
    let record_type: &str = row.try_get("entry_type")?;
    if record_type == "ACCESS_LOG_ENTRY" {
        let recorded: Vec<u8> = row.try_get("content_digest")?;
        if Sha256::digest(document.as_bytes()).as_slice() != recorded.as_slice() {
            return Ok(false);
        }
    }
    let Ok(cut) = Cut::of(&document) else {
        return Ok(false);
    };
    if cut.has("link") {
        return Ok(false);
    }
    let Ok(derived) = Derived::of(&cut) else {
        return Ok(false);
    };
    columns_hold(row, record_type, &derived)
}

/// Whether the derived columns of `row` hold `derived`, each read as the
/// database casts its text (`audit.access_log_entry_derive`).
fn columns_hold(row: &PgRow, record_type: &str, derived: &Derived) -> Result<bool, AuditError> {
    let instant = |text: &Option<String>| -> Option<Option<jiff::Timestamp>> {
        match text {
            None => Some(None),
            Some(text) => text
                .parse::<jiff::Timestamp>()
                .ok()
                .and_then(|at| at.round(jiff::Unit::Microsecond).ok())
                .map(Some),
        }
    };
    let column_instant = |name: &str| -> Result<Option<jiff::Timestamp>, sqlx::Error> {
        Ok(row
            .try_get::<Option<Timestamp>, _>(name)?
            .map(Timestamp::to_jiff))
    };
    let (Some(time_recorded), Some(time_removed)) = (
        instant(&derived.time_recorded),
        instant(&derived.time_removed),
    ) else {
        return Ok(false);
    };
    let fold_after = match (time_removed, time_recorded) {
        (Some(removed), Some(recorded)) => {
            let Ok(kept) = removed.since(recorded) else {
                return Ok(false);
            };
            let Ok(fold) = removed.checked_add(kept) else {
                return Ok(false);
            };
            Some(fold)
        }
        _ => None,
    };
    let retain_until = match &derived.retain_until {
        None => None,
        Some(text) => match text.parse::<jiff::civil::Date>() {
            Ok(date) => Some(date),
            Err(_) => return Ok(false),
        },
    };
    let uid = derived
        .uid
        .as_deref()
        .and_then(|uid| uuid::Uuid::parse_str(uid).ok());
    let ehr_id = derived
        .ehr_id
        .as_deref()
        .and_then(crate::access_log::stored_form::column_uuid);
    let text = |name: &str| row.try_get::<Option<String>, _>(name);
    Ok(derived.record_type.as_deref() == Some(record_type)
        && uid == Some(row.try_get::<uuid::Uuid, _>("entry_uid")?)
        && time_recorded == column_instant("recorded_at")?
        && ehr_id == row.try_get::<Option<uuid::Uuid>, _>("entry_ehr")?
        && derived.action_terminology == text("action_terminology")?
        && derived.action_code == text("action_code")?
        && derived.result_terminology == text("result_terminology")?
        && derived.result_code == text("result_code")?
        && derived.party_id == text("party_id")?
        && retain_until
            == row
                .try_get::<Option<jiff_sqlx::Date>, _>("retain_until")?
                .map(jiff_sqlx::Date::to_jiff)
        && time_removed == column_instant("time_removed")?
        && fold_after == column_instant("fold_after")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step slower than the budget shrinks the next, a faster one grows it
    /// at most twofold, and both stay within the floor and the ceiling.
    #[test]
    fn the_step_follows_its_duration_toward_the_budget() {
        let mut pace = StepPace::default();
        pace.took(STEP_BUDGET * 4);
        assert_eq!(
            pace.records(),
            1_250,
            "a fourfold slow step quarters the next"
        );
        pace.took(STEP_BUDGET / 100);
        assert_eq!(pace.records(), 2_500);
        pace.took(STEP_BUDGET);
        assert_eq!(pace.records(), 2_500);
        for _ in 0..20 {
            pace.took(Duration::from_secs(60));
        }
        assert_eq!(pace.records(), STEP_FLOOR);
        for _ in 0..20 {
            pace.took(Duration::ZERO);
        }
        assert_eq!(pace.records(), STEP_CEILING);
    }
}
