// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The sealed windows of the two audit logs (#6242): the access log, whose
//! records are named by `audit.access_log_entry.id`, and the system records,
//! named by `audit.audit_event.seq`.
//!
//! A seal covers every record id from `first_id` to `last_id`, follows the
//! seal before it, and stores the SHA-256 of this text, which an auditor can
//! rebuild from the stored rows alone:
//!
//! ```text
//! <length>:<tag: ferroehr.access_log.seal.v1 or ferroehr.system_log.seal.v1>
//! <length>:<window_no>
//! <length>:<first_id>
//! <length>:<last_id>
//! <length>:<entries>
//! 64:<previous seal digest>
//! then, for each record of the window in id order:
//! <length>:<id>
//! 64:<content_digest>
//! ```
//!
//! Each field is its length in bytes in decimal, a colon, the field and a
//! line feed. Integers are decimal with no sign and no leading zero, digests
//! lower-case hexadecimal. The first window follows the SHA-256 of the
//! version tag (`audit.access_log_seal_genesis()`,
//! `audit.audit_event_seal_genesis()`). An access-log tombstone keeps the
//! content digest of the entry it replaced, and a reaped system record keeps
//! its position and content digest in `audit.audit_event_reaped`, so a window
//! verifies after retention removed content inside it, and a window the fold
//! removed keeps its seal. The sealer seals up to the frontier
//! ([`super::frontier`]) and below every record a write still holds, so a
//! window never changes once sealed; one sealer per log runs at a time across
//! replicas, on the lock of its seal state. No specification governs it: our
//! own design.

use std::fmt::Write as _;

use sqlx::Connection;

use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use crate::system_log::store::chains::AuditChain;

/// The version tag of the access log's seals, the first field of their text.
pub const TAG: &str = "ferroehr.access_log.seal.v1";

/// The version tag of the system records' seals.
pub const SYSTEM_TAG: &str = "ferroehr.system_log.seal.v1";

/// The statements of one sealed log, over its record, hold and seal tables.
#[derive(Debug)]
struct Queries {
    /// The seal state, locked: `last_id`, `window_no`, digest.
    state_lock: &'static str,
    /// The lowest held id.
    held: &'static str,
    /// Each record in `($1, $2]`: id and content digest, in id order.
    records: &'static str,
    /// A seal row.
    insert_seal: &'static str,
    /// The seal state moved.
    update_state: &'static str,
    /// The seals that cover an id in `[$1, $2]`, with whether folded.
    seals: &'static str,
    /// The windows after `$1`, up to a budget `$2` of entries.
    next_seals: &'static str,
    /// The digest and last id of window `$1`.
    seal_at: &'static str,
    /// Whether a window after `$1` exists.
    more: &'static str,
    /// The seal state: `window_no`, digest.
    state: &'static str,
    /// The window an incremental verification starts after.
    verified: &'static str,
    /// The verification cursor moved to `$1`.
    set_verified: &'static str,
    /// Whether a record waits past the last window.
    unsealed: &'static str,
}

/// The statements of a log, from its table names.
macro_rules! queries {
    ($table:literal, $id:literal, $records:literal, $hold:literal, $hold_id:literal,
     $seal:literal, $state:literal, $fold:literal, $verified:literal) => {
        Queries {
            state_lock: concat!("SELECT last_id, window_no, encode(digest, 'hex') FROM audit.",
                $state, " WHERE singleton FOR UPDATE SKIP LOCKED"),
            held: concat!("SELECT min(", $hold_id, ") FROM audit.", $hold),
            records: $records,
            insert_seal: concat!("INSERT INTO audit.", $seal, " (window_no, first_id, last_id, \
                entries, previous_digest, digest, proof, system_id, sealed_at) \
                VALUES ($1, $2, $3, $4, decode($5, 'hex'), decode($6, 'hex'), $7, $8, $9)"),
            update_state: concat!("UPDATE audit.", $state, " SET last_id = $1, window_no = $2, \
                digest = decode($3, 'hex') WHERE singleton"),
            seals: concat!("SELECT s.window_no, s.first_id, s.last_id, s.entries, \
                encode(s.previous_digest, 'hex'), encode(s.digest, 'hex'), \
                EXISTS (SELECT 1 FROM audit.", $fold, " f WHERE f.window_no = s.window_no) \
                FROM audit.", $seal, " s WHERE s.last_id >= $1 AND s.first_id <= $2 \
                ORDER BY s.window_no"),
            next_seals: concat!("WITH next AS ( \
                SELECT s.*, sum(s.entries) OVER (ORDER BY s.window_no) - s.entries AS before \
                  FROM audit.", $seal, " s WHERE s.window_no > $1 ORDER BY s.window_no \
                 LIMIT 4096) \
                SELECT n.window_no, n.first_id, n.last_id, n.entries, \
                       encode(n.previous_digest, 'hex'), encode(n.digest, 'hex'), \
                       EXISTS (SELECT 1 FROM audit.", $fold, " f \
                                WHERE f.window_no = n.window_no) \
                  FROM next n WHERE n.before < greatest($2, 1) ORDER BY n.window_no"),
            seal_at: concat!("SELECT encode(digest, 'hex'), last_id FROM audit.", $seal,
                " WHERE window_no = $1"),
            more: concat!("SELECT EXISTS (SELECT 1 FROM audit.", $seal, " WHERE window_no > $1)"),
            state: concat!("SELECT window_no, encode(digest, 'hex') FROM audit.", $state,
                " WHERE singleton"),
            verified: concat!("SELECT through_window FROM audit.", $verified, " WHERE singleton"),
            set_verified: concat!("INSERT INTO audit.", $verified, " AS v (singleton, \
                through_window, verified_at, full_verified_at) \
                VALUES (true, $1, now(), CASE WHEN $2 AND $3 THEN now() END) \
                ON CONFLICT (singleton) DO UPDATE SET through_window = excluded.through_window, \
                verified_at = excluded.verified_at, \
                full_verified_at = coalesce(excluded.full_verified_at, v.full_verified_at)"),
            unsealed: concat!("SELECT EXISTS (SELECT 1 FROM audit.", $table, " e WHERE e.", $id,
                " > (SELECT s.last_id FROM audit.", $state, " s WHERE s.singleton))"),
        }
    };
}

/// One of the two sealed logs: its chain, its version tag and its
/// statements.
#[derive(Debug)]
pub struct Log {
    chain: AuditChain,
    tag: &'static str,
    sql: Queries,
}

impl Log {
    /// The chain the log is.
    #[must_use]
    pub fn chain(&self) -> AuditChain {
        self.chain
    }

    /// The version tag of its seals.
    #[must_use]
    pub fn tag(&self) -> &'static str {
        self.tag
    }
}

/// The access log.
pub static ACCESS_LOG: Log = Log {
    chain: AuditChain::AccessLog,
    tag: TAG,
    sql: queries!(
        "access_log_entry",
        "id",
        "SELECT id, encode(content_digest, 'hex') FROM audit.access_log_entry \
         WHERE id > $1 AND id <= $2 ORDER BY id",
        "access_log_hold",
        "entry_id",
        "access_log_seal",
        "access_log_seal_state",
        "access_log_seal_fold",
        "access_log_seal_verified"
    ),
};

/// The system records: a record retention reaped is sealed and verified by
/// the position and content digest `audit.audit_event_reaped` keeps.
pub static SYSTEM: Log = Log {
    chain: AuditChain::System,
    tag: SYSTEM_TAG,
    sql: queries!(
        "audit_event",
        "seq",
        "SELECT seq, encode(content_digest, 'hex') FROM audit.audit_event \
         WHERE seq > $1 AND seq <= $2 \
         UNION ALL SELECT seq, encode(content_digest, 'hex') FROM audit.audit_event_reaped \
         WHERE seq > $1 AND seq <= $2 ORDER BY 1",
        "audit_event_hold",
        "seq",
        "audit_event_seal",
        "audit_event_seal_state",
        "audit_event_seal_fold",
        "audit_event_seal_verified"
    ),
};

/// The log `chain` names.
#[must_use]
pub fn log(chain: AuditChain) -> &'static Log {
    match chain {
        AuditChain::AccessLog => &ACCESS_LOG,
        AuditChain::System => &SYSTEM,
    }
}

/// The most entries one window holds.
const WINDOW_ENTRIES: i64 = 100_000;

/// One entry of a window: its id and its content digest, lower-case hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// The entry id.
    pub id: i64,
    /// The entry's content digest.
    pub content_digest: String,
}

/// The fields of a window's seal other than its entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window<'a> {
    /// The version tag of the log's seals.
    pub tag: &'a str,
    /// The window's number, one more than the window before it.
    pub number: i64,
    /// The first entry id it covers.
    pub first_id: i64,
    /// The last entry id it covers.
    pub last_id: i64,
    /// The digest of the seal before it, lower-case hex.
    pub previous: &'a str,
}

fn field(text: &mut String, value: &str) {
    // NOTE: `impl fmt::Write for String` never fails
    // (<https://doc.rust-lang.org/std/fmt/trait.Write.html#impl-Write-for-String>).
    let _written = writeln!(text, "{}:{value}", value.len());
}

/// The text a window's seal hashes.
#[must_use]
pub fn text(window: Window<'_>, entries: &[Sealed]) -> String {
    let mut text = String::with_capacity(entries.len().saturating_mul(80).saturating_add(160));
    field(&mut text, window.tag);
    field(&mut text, &window.number.to_string());
    field(&mut text, &window.first_id.to_string());
    field(&mut text, &window.last_id.to_string());
    field(&mut text, &entries.len().to_string());
    field(&mut text, window.previous);
    for entry in entries {
        field(&mut text, &entry.id.to_string());
        field(&mut text, &entry.content_digest);
    }
    text
}

/// A window's seal digest, lower-case hex.
#[must_use]
pub fn digest(window: Window<'_>, entries: &[Sealed]) -> String {
    sha256_hex(&text(window, entries))
}

/// What a sealing round did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealRound {
    /// Another replica's sealer holds the seal state.
    Busy,
    /// Nothing past the last window is below the frontier and every hold.
    Nothing {
        /// The last entry id the windows cover.
        through: i64,
    },
    /// One window sealed.
    Sealed {
        /// Its number.
        window_no: i64,
        /// The entries it holds.
        entries: usize,
    },
}

impl AccessLogStore {
    /// Seal the records of `log` past its last window up to `frontier` and
    /// below every record a write still holds, as one window of at most
    /// 100,000 ids, signed when the store has a sealer.
    ///
    /// # Errors
    /// The store errors when a statement fails, and the signing errors of the
    /// sealer.
    pub async fn seal_to(&self, log: &Log, frontier: i64) -> Result<SealRound, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut tx = conn.begin().await?;
        let state: Option<(i64, i64, String)> = sqlx::query_as(log.sql.state_lock)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((last_id, window_no, previous)) = state else {
            return Ok(SealRound::Busy);
        };
        let held: Option<i64> = sqlx::query_scalar(log.sql.held).fetch_one(&mut *tx).await?;
        let upto = held
            .map_or(frontier, |held| frontier.min(held.saturating_sub(1)))
            .min(last_id.saturating_add(WINDOW_ENTRIES));
        if upto <= last_id {
            tx.rollback().await?;
            return Ok(SealRound::Nothing { through: last_id });
        }
        let entries = window_records(&mut tx, log, last_id, upto).await?;
        let window_no = window_no.saturating_add(1);
        let window = Window {
            tag: log.tag,
            number: window_no,
            first_id: last_id.saturating_add(1),
            last_id: upto,
            previous: &previous,
        };
        let seal_digest = digest(window, &entries);
        let signed = match (self.sealer(), log.chain) {
            (Some(sealer), AuditChain::AccessLog) => {
                Some(sealer.sign_seal(upto, &seal_digest).await?)
            }
            (Some(sealer), AuditChain::System) => {
                Some(sealer.sign_system_seal(upto, &seal_digest).await?)
            }
            (None, _) => None,
        };
        let (proof, system_id, sealed_at) = match signed {
            Some(signed) => (
                Some(signed.proof),
                Some(signed.system_id),
                jiff_sqlx::Timestamp::from(signed.sealed),
            ),
            None => (
                None,
                None,
                jiff_sqlx::Timestamp::from(jiff::Timestamp::now()),
            ),
        };
        let count = i32::try_from(entries.len()).map_err(|_too_many| AccessLogError::Exhausted)?;
        sqlx::query(log.sql.insert_seal)
            .bind(window_no)
            .bind(window.first_id)
            .bind(upto)
            .bind(count)
            .bind(&previous)
            .bind(&seal_digest)
            .bind(proof)
            .bind(system_id)
            .bind(sealed_at)
            .execute(&mut *tx)
            .await?;
        sqlx::query(log.sql.update_state)
            .bind(upto)
            .bind(window_no)
            .bind(&seal_digest)
            .execute(&mut *tx)
            .await?;
        // NOTE: our own design (#5762), the sealer's own round, which no request makes.
        tx.commit().await?;
        Ok(SealRound::Sealed {
            window_no,
            entries: entries.len(),
        })
    }

    /// Whether a record of `log` waits past its last sealed window.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn unsealed(&self, log: &Log) -> Result<bool, AccessLogError> {
        Ok(sqlx::query_scalar(log.sql.unsealed)
            .fetch_one(&mut *acquire(self.pool()).await?)
            .await?)
    }
}

/// The records of `log` in `(after, upto]` with their content digests, in
/// id order, read on `conn`.
async fn window_records(
    conn: &mut sqlx::PgConnection,
    log: &Log,
    after: i64,
    upto: i64,
) -> Result<Vec<Sealed>, AccessLogError> {
    Ok(sqlx::query_as::<_, (i64, String)>(log.sql.records)
        .bind(after)
        .bind(upto)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(id, content_digest)| Sealed { id, content_digest })
        .collect())
}

impl AccessLogStore {
    /// Seal every record of `log` committed before the call and held by no
    /// write: the frontier moved until it reaches the id sequence's value
    /// read first, then every window up to it sealed, waiting out another
    /// replica's sealer.
    ///
    /// # Errors
    /// As [`Self::seal_to`].
    pub async fn seal_committed(&self, log: &Log) -> Result<(), AccessLogError> {
        let mut frontier = super::frontier::Frontier::default();
        let (access_log, system): (Option<i64>, Option<i64>) =
            sqlx::query_as(super::frontier::LAST_VALUES)
                .fetch_one(&mut *acquire(self.pool()).await?)
                .await?;
        let target = super::frontier::Reached {
            access_log: access_log.unwrap_or(0),
            system: system.unwrap_or(0),
        };
        let reached = loop {
            let reached = frontier.round(&mut *acquire(self.pool()).await?).await?;
            if reached.access_log >= target.access_log && reached.system >= target.system {
                break reached;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        };
        let upto = match log.chain {
            AuditChain::AccessLog => reached.access_log,
            AuditChain::System => reached.system,
        };
        loop {
            match self.seal_to(log, upto).await? {
                SealRound::Busy => tokio::time::sleep(std::time::Duration::from_millis(2)).await,
                SealRound::Nothing { .. } => return Ok(()),
                SealRound::Sealed { .. } => {}
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> String {
        sha256_hex(TAG)
    }

    #[test]
    fn the_text_is_length_prefixed_and_starts_with_its_version_tag() {
        let previous = genesis();
        let content = "a".repeat(64);
        let window = Window {
            tag: TAG,
            number: 1,
            first_id: 7,
            last_id: 8,
            previous: &previous,
        };
        let entries = [Sealed {
            id: 7,
            content_digest: content.clone(),
        }];
        let text = text(window, &entries);
        assert!(
            text.starts_with("27:ferroehr.access_log.seal.v1\n1:1\n1:7\n1:8\n1:1\n64:"),
            "{text}"
        );
        assert!(text.ends_with(&format!("1:7\n64:{content}\n")), "{text}");
    }

    #[test]
    fn a_digit_moved_between_two_fields_changes_the_digest() {
        let previous = genesis();
        let entries = [Sealed {
            id: 12,
            content_digest: "b".repeat(64),
        }];
        let one = digest(
            Window {
                tag: TAG,
                number: 1,
                first_id: 12,
                last_id: 13,
                previous: &previous,
            },
            &entries,
        );
        let two = digest(
            Window {
                tag: TAG,
                number: 11,
                first_id: 2,
                last_id: 13,
                previous: &previous,
            },
            &entries,
        );
        assert_ne!(one, two);
    }
}
