// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The verification of the sealed windows of both logs in bounded steps
//! (#5779, #6242).
//!
//! A walk of a log verifies its sealed windows from the cursor, or the first,
//! one step at a time. Each step reads windows up to a budget of records in
//! one snapshot ([`crate::access_log::seal`]) and moves the verification
//! cursor to its last window when it and every step before it in the walk
//! are intact, so no step grows with the backlog and every step's progress is
//! kept. A walk that finds damage goes on to the last window, so it reports
//! every finding, and moves the cursor no more. No openEHR spec governs audit
//! tamper detection: our own design.

use std::time::{Duration, Instant};

use jiff_sqlx::Timestamp;

use crate::access_log::stored_form::parts::{Cut, Derived};
use sqlx::Row;
use sqlx::postgres::{PgConnection, PgRow};

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::store::AuditStore;
use crate::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope};

/// How long one step of a walk aims to take: well under any statement
/// timeout, and short enough that a step holds the database briefly.
pub const STEP_BUDGET: Duration = Duration::from_millis(500);

/// The fewest records a step verifies, however slow the last one was.
pub const STEP_FLOOR: i32 = 500;

/// The most records a step verifies, however fast the last one was.
pub const STEP_CEILING: i32 = 100_000;

/// The longest pause after a step, as a multiple of the step's duration, so
/// a walk under the heaviest load still verifies a sixteenth of the time.
pub const PAUSE_CEILING: u32 = 16;

/// The records the first step of a walk verifies.
const STEP_FIRST: i32 = 5_000;

/// The size of a walk's next step, adapted to how long its last one took,
/// and the pause owed before it, adapted to how loaded the database was.
///
/// A step's cost per record grows with the load on the database, so a fixed
/// count can near the statement timeout (#5804). The same cost, against the
/// lowest this pace has measured, reads the load: a step that cost `k` times
/// the quiet cost per record is followed by a pause `k²` times its duration,
/// within [`PAUSE_CEILING`], so a walk leaves a loaded database to the
/// requests and takes about half a core of a quiet one. No openEHR spec
/// governs the verification schedule: our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepPace {
    records: i32,
    /// The duration of the last step measured, and its cost per record in
    /// nanoseconds; `None` before the first.
    last: Option<(Duration, u128)>,
    /// The lowest cost per record measured, in nanoseconds.
    quiet: Option<u128>,
}

impl Default for StepPace {
    fn default() -> Self {
        Self {
            records: STEP_FIRST,
            last: None,
            quiet: None,
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
            ..Self::default()
        }
    }

    /// The records the next step verifies.
    #[must_use]
    pub fn records(self) -> i32 {
        self.records
    }

    /// Adapt to a step of [`Self::records`] that took `took`: toward
    /// [`STEP_BUDGET`], at most doubling, within [`STEP_FLOOR`] and
    /// [`STEP_CEILING`]; and record its cost per record for [`Self::pause`].
    pub fn took(&mut self, took: Duration) {
        let records = i128::from(self.records);
        let cost = took
            .as_nanos()
            .checked_div(u128::from(self.records.unsigned_abs()))
            .unwrap_or(u128::MAX)
            .max(1);
        self.last = Some((took, cost));
        self.quiet = Some(self.quiet.map_or(cost, |quiet| quiet.min(cost)));
        let took = i128::try_from(took.as_micros()).unwrap_or(i128::MAX).max(1);
        let budget = i128::try_from(STEP_BUDGET.as_micros()).unwrap_or(i128::MAX);
        let next = records
            .saturating_mul(budget)
            .checked_div(took)
            .unwrap_or(i128::from(STEP_CEILING))
            .min(records.saturating_mul(2))
            .clamp(i128::from(STEP_FLOOR), i128::from(STEP_CEILING));
        // NOTE: our own design; the clamp above keeps it inside the i32 floor and ceiling.
        self.records = i32::try_from(next).unwrap_or(STEP_CEILING);
    }

    /// The pause owed after the last step measured: its duration times the
    /// square of its cost per record over the quiet cost, at least the
    /// step's duration and at most [`PAUSE_CEILING`] times it; none before
    /// the first step.
    #[must_use]
    pub fn pause(self) -> Duration {
        let Some((took, cost)) = self.last else {
            return Duration::ZERO;
        };
        let quiet = self.quiet.unwrap_or(cost).max(1);
        let ceiling = took.saturating_mul(PAUSE_CEILING);
        let nanos = took
            .as_nanos()
            .saturating_mul(cost)
            .checked_div(quiet)
            .unwrap_or(u128::MAX)
            .saturating_mul(cost)
            .checked_div(quiet)
            .unwrap_or(u128::MAX);
        let pause = u64::try_from(nanos).map_or(ceiling, Duration::from_nanos);
        pause.clamp(took, ceiling)
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
    /// The window the walk has verified through.
    after: i64,
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
        let after = if full {
            0
        } else {
            self.chaining_store()
                .verified_window(crate::access_log::seal::log(chain))
                .await
                .map_err(AuditError::from)?
        };
        Ok(ChainWalk {
            chain,
            full,
            head: Some(after),
            after,
            intact: true,
            done: false,
        })
    }

    /// Verify the next windows of `walk`, up to `records` records, and move
    /// the cursor to the last of them when they and every window before them
    /// are intact; the last step also checks the seal state. Returns the
    /// step's findings; a walk that is done returns none.
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
        let step = self
            .chaining_store()
            .verify_window_step(
                crate::access_log::seal::log(walk.chain),
                walk.after,
                i64::from(records),
                walk.full,
                walk.intact,
            )
            .await
            .map_err(AuditError::from)?;
        walk.intact &= step.findings.is_empty();
        walk.after = step.through;
        walk.head = Some(step.through);
        walk.done = step.done;
        Ok(step.findings)
    }

    /// [`Self::chain_step`] sized by `pace`, which then adapts to how long
    /// the step took; returns the step's findings and the pause the walk
    /// owes before its next step ([`StepPace::pause`]), none once it is
    /// done. The last step, which may verify fewer records, leaves `pace`
    /// as it was.
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
        if walk.done {
            return Ok((findings, Duration::ZERO));
        }
        pace.took(started.elapsed());
        Ok((findings, pace.pause()))
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
}

/// How many system records one page of the content check reads.
const SYSTEM_PAGE: i64 = 1000;

/// A system record that no longer digests to its `content_digest`: its
/// position, id and event time.
pub type Modified = (i64, uuid::Uuid, jiff::Timestamp);

/// One page of the system records from `$1` (exclusive) to `$2`, at most
/// `$3`, each with its position, uuid, time, stored digest and the object
/// its content digest is computed over.
static MODIFIED_PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "SELECT e.seq, e.id, e.recorded_at, e.content_digest, {} FROM audit.audit_event e \
         WHERE e.seq > $1 AND e.seq <= $2 ORDER BY e.seq LIMIT $3",
        crate::system_log::content_digest::STORED
    )
});

/// The system records in `(after, upto]` that no longer digest to their
/// `content_digest` ([`crate::system_log::content_digest`]), read a page at a
/// time.
///
/// # Errors
/// [`AuditError::Store`] when a statement fails, and [`AuditError::Digest`]
/// when a record read back has no RFC 8785 form.
pub async fn system_modified_between(
    conn: &mut PgConnection,
    mut after: i64,
    upto: i64,
) -> Result<Vec<Modified>, AuditError> {
    let mut modified = Vec::new();
    loop {
        let rows: Vec<(i64, uuid::Uuid, Timestamp, Vec<u8>, serde_json::Value)> =
            sqlx::query_as(sqlx::AssertSqlSafe(MODIFIED_PAGE.as_str()))
                .bind(after)
                .bind(upto)
                .bind(SYSTEM_PAGE)
                .fetch_all(&mut *conn)
                .await?;
        let Some((last, ..)) = rows.last() else {
            return Ok(modified);
        };
        after = *last;
        for (seq, id, recorded_at, stored, document) in rows {
            let recomputed = crate::system_log::content_digest::of_stored(document)
                .map_err(AuditError::Digest)?;
            if recomputed.as_slice() != stored.as_slice() {
                modified.push((seq, id, recorded_at.to_jiff()));
            }
        }
    }
}

/// The positions in `(after, upto]` whose record was changed after it was
/// written, read a page at a time: an entry that no longer digests to its
/// link's `content_digest`, an entry or tombstone whose derived columns are
/// not those its content decides (#6005), and one under the search index's
/// cursor whose index row is missing or holds other columns than its own row
/// (#6319). The rows are read from
/// `access_log_entry` itself, which the index row is checked against.
pub(crate) async fn modified_between(
    conn: &mut PgConnection,
    mut after: i64,
    upto: i64,
) -> Result<Vec<i64>, AuditError> {
    let mut modified = Vec::new();
    loop {
        let rows = sqlx::query(concat!(
            "SELECT r.chain_seq, ",
            crate::access_log::store::entry_parts!(),
            ", r.content_digest, encode(r.shared, 'hex') AS shared_hex, r.action_terminology, r.action_code, r.result_terminology, \
             r.result_code, r.party_id, r.retain_until, r.time_removed, r.fold_after, \
             (r.chain_seq > (SELECT x.last_id FROM audit.access_log_index_state x WHERE x.singleton) \
              OR EXISTS (SELECT 1 FROM audit.access_log_index i WHERE i.entry_id = r.chain_seq)) \
             AND NOT EXISTS (SELECT 1 FROM audit.access_log_index i WHERE i.entry_id = r.chain_seq \
                AND (i.time_recorded, i.ehr_id, i.party_id, i.retain_until, \
                     i.render_policy_id, i.patient_id, i.principal, i.search_keys) \
                    IS DISTINCT FROM (r.time_recorded, r.ehr_id, r.party_id, r.retain_until, \
                     r.render_policy_id, r.patient_id, r.principal, \
                     audit.search_keys(r.search_tokens, r.request_id, r.resource_id))) AS index_holds \
             FROM (SELECT e.id AS chain_seq, e.* FROM audit.access_log_entry e \
                    WHERE NOT EXISTS (SELECT 1 FROM audit.access_log_hold h WHERE h.entry_id = e.id)) r \
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
/// holds what the content decides, and its search index row, when it has one,
/// holds its row's columns. A record whose parts no longer join was changed
/// after it was written.
fn record_intact(row: &PgRow) -> Result<bool, AuditError> {
    if !row.try_get::<bool, _>("index_holds")? {
        return Ok(false);
    }
    // NOTE: our own design (#6005); a record whose parts no longer join was changed after it was
    // written.
    let document = match crate::access_log::store::document_of(row, row.try_get("shared_part")?) {
        Ok(document) => document,
        Err(crate::access_log::store::AccessLogError::Unreadable { .. }) => return Ok(false),
        Err(other) => return Err(other.into()),
    };
    // NOTE: our own design (#6322), the facts digest names the shared part by its key, so the
    // part's content must still hash to that key.
    let shared_hex: Option<String> = row.try_get("shared_hex")?;
    let shared_part: Option<&str> = row.try_get("shared_part")?;
    if shared_part.map(crate::digest::sha256_hex) != shared_hex {
        return Ok(false);
    }
    let record_type: &str = row.try_get("entry_type")?;
    if record_type == "ACCESS_LOG_ENTRY" {
        let recorded: Vec<u8> = row.try_get("content_digest")?;
        let Ok(facts) = crate::access_log::stored_form::parts::facts::facts_digest(
            row.try_get("stored_form")?,
            row.try_get("row_part")?,
            shared_hex.as_deref(),
            &crate::access_log::store::row_columns(row)?,
        ) else {
            return Ok(false);
        };
        if facts.as_slice() != recorded.as_slice() {
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

    /// A step at the quiet cost per record pauses as long as it took, one
    /// that cost twice as much per record pauses four times as long, no
    /// pause passes the ceiling, and the pause falls back once the load has
    /// passed (#5804).
    #[test]
    fn the_pause_grows_with_the_cost_per_record_over_the_quiet_cost() {
        let mut pace = StepPace::starting_at(5_000);
        assert_eq!(pace.pause(), Duration::ZERO, "no pause before a step");
        pace.took(STEP_BUDGET);
        assert_eq!(pace.pause(), STEP_BUDGET, "a quiet step pauses as long");
        pace.took(STEP_BUDGET * 2);
        assert_eq!(
            pace.pause(),
            STEP_BUDGET * 8,
            "twice the cost per record pauses four times the step's duration"
        );
        assert_eq!(pace.records(), 2_500, "and halves the next step");
        pace.took(STEP_BUDGET * 10);
        assert_eq!(pace.pause(), STEP_BUDGET * 10 * PAUSE_CEILING);
        assert_eq!(pace.records(), STEP_FLOOR);
        pace.took(STEP_BUDGET / 10);
        assert_eq!(pace.pause(), STEP_BUDGET / 10, "the load has passed");
    }
}
