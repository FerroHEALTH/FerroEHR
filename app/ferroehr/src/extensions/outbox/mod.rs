// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The commit-event outbox: the reader registry `[record.change_feed]` reconciles at
//! boot, the cursor feed each reader pulls, and the retention prune.
//!
//! **No openEHR spec governs eventing: our own design/extension.** While a
//! reader is configured, every commit writes one PHI-free row into its
//! domain's `event_outbox` in the commit's own transaction. The clinical and
//! the party domain each keep their own outbox, with their own `seq`, and a
//! reader keeps one cursor per domain in that domain's `event_outbox_reader`.
//! A reader reads the rows after its cursor ([`read`]) and moves the cursor by
//! acknowledging ([`acknowledge`]); delivery onward, to a broker or anywhere
//! else, is the reader's.
//!
//! A read leaves out the clinical rows of an EHR that carries a legal mark
//! when it is read: a restriction (GDPR Art. 18(2) leaves only storage), an
//! EHDS Art. 8 access restriction of the EHR or of an object the commit
//! touched, or an unoverridden research objection (GDPR Art. 21(6)). Such a
//! row is served once the mark is lifted, while the reader's cursor has not
//! passed it (`docs/law/eu/gdpr/text.html`, `docs/law/eu/ehds/text.html`).
//!
//! Commits can end in another order than the one they drew their `seq` in. A
//! commit therefore holds a shared advisory lock on its domain's outbox from
//! its insert to its end, and a read or an acknowledgement takes it
//! exclusively first: once granted, every `seq` drawn so far belongs to a
//! finished transaction, so a cursor never passes a row that becomes visible
//! after it.
//!
//! The prune deletes a row once it is older than the retention window and at
//! or below every active reader's cursor, so a reader that is down loses
//! nothing inside the window. A reader the configuration no longer names is
//! inactive and holds nothing back.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the PHI-free event envelope is open \
              operational JSON stored as jsonb (family 6, event rows)"
)]

pub mod config;

use std::time::Duration;

use serde_json::{Map, Value};
use sqlx::types::Json;
use sqlx::{Connection as _, PgConnection, PgPool};

use crate::db::acquire::{AcquireError, acquire};
use crate::db::domain::DomainPools;
use crate::error::refusal::Missing;
use crate::error::{ErrorChain, ServiceError};
use crate::service::FerroEhrService;

/// The exclusive form of the lock every outbox insert holds shared, keyed on
/// the connection's domain schema.
const EXCLUSIVE_LOCK: &str =
    "SELECT pg_advisory_xact_lock(hashtext(current_schema() || '.event_outbox'), 0)";

/// A failure of an outbox registry, feed or prune statement.
#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    /// No connection could be obtained from the pool.
    #[error(transparent)]
    Acquire(#[from] AcquireError),
    /// The statement failed on the acquired connection.
    #[error("outbox statement failed")]
    Database(#[from] sqlx::Error),
}

impl From<OutboxError> for ServiceError {
    fn from(error: OutboxError) -> Self {
        match error {
            OutboxError::Acquire(error) => Self::Acquire(error),
            OutboxError::Database(error) => Self::Database(error),
        }
    }
}

/// The domain whose outbox a feed call reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutboxDomain {
    /// The clinical outbox: EHR, composition, folder and contribution commits,
    /// and the erasure of an EHR.
    #[default]
    Clinical,
    /// The party outbox: demographic commits and the erasure of a party.
    Party,
}

impl OutboxDomain {
    /// Both outboxes.
    pub const ALL: [Self; 2] = [Self::Clinical, Self::Party];

    /// The token the feed's `domain` parameter names.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clinical => "clinical",
            Self::Party => "party",
        }
    }

    /// The domain `token` names, `None` for any other token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|domain| domain.as_str() == token)
    }

    /// The pool that serves this domain's outbox.
    #[must_use]
    pub const fn pool(self, pools: &DomainPools) -> &PgPool {
        match self {
            Self::Clinical => &pools.clinical,
            Self::Party => &pools.party,
        }
    }
}

/// What one boot reconciliation changed in one domain's reader registry.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// The readers registered for the first time or made active again.
    pub registered: Vec<String>,
    /// The readers the configuration no longer names, now inactive.
    pub deactivated: Vec<String>,
}

/// Register every reader in `readers` as active and mark every other active
/// reader inactive, in one transaction.
///
/// A new reader starts at cursor `0`; a reader made active again keeps its
/// cursor.
///
/// # Errors
///
/// [`OutboxError::Acquire`] when no connection can be obtained, and
/// [`OutboxError::Database`] when the registry cannot be written.
pub async fn reconcile(pool: &PgPool, readers: &[String]) -> Result<Reconciled, OutboxError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    let mut registered: Vec<String> = sqlx::query_scalar(
        "INSERT INTO event_outbox_reader (reader, active) \
         SELECT name, true FROM unnest($1::text[]) AS name \
         ON CONFLICT (reader) DO UPDATE SET active = true, updated_at = now() \
            WHERE NOT event_outbox_reader.active \
         RETURNING reader",
    )
    .bind(readers)
    .fetch_all(&mut *tx)
    .await?;
    let mut deactivated: Vec<String> = sqlx::query_scalar(
        "UPDATE event_outbox_reader SET active = false, updated_at = now() \
         WHERE active AND reader <> ALL($1::text[]) RETURNING reader",
    )
    .bind(readers)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    registered.sort_unstable();
    deactivated.sort_unstable();
    Ok(Reconciled {
        registered,
        deactivated,
    })
}

/// [`reconcile`] in both outboxes, logging what each changed.
///
/// # Errors
///
/// The first [`reconcile`] failure.
pub async fn reconcile_domains(pools: &DomainPools, readers: &[String]) -> Result<(), OutboxError> {
    for domain in OutboxDomain::ALL {
        let reconciled = reconcile(domain.pool(pools), readers).await?;
        if !reconciled.registered.is_empty() || !reconciled.deactivated.is_empty() {
            tracing::info!(
                domain = domain.as_str(),
                registered = ?reconciled.registered,
                deactivated = ?reconciled.deactivated,
                "outbox readers reconciled with [record.change_feed] readers"
            );
        }
    }
    Ok(())
}

/// One event of the feed: the stored envelope with its delivery `seq`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FeedEvent {
    /// The row's place in its domain's outbox.
    pub seq: i64,
    /// The stored PHI-free envelope, its members beside `seq`.
    #[serde(flatten)]
    pub envelope: Map<String, Value>,
}

/// The active reader's cursor, under a row lock when `lock` is set; `None`
/// for a reader that is unknown or inactive.
async fn active_cursor(
    conn: &mut PgConnection,
    reader: &str,
    lock: bool,
) -> Result<Option<i64>, sqlx::Error> {
    let sql = if lock {
        "SELECT last_seq FROM event_outbox_reader WHERE reader = $1 AND active FOR UPDATE"
    } else {
        "SELECT last_seq FROM event_outbox_reader WHERE reader = $1 AND active"
    };
    sqlx::query_scalar(sql)
        .bind(reader)
        .fetch_optional(&mut *conn)
        .await
}

/// The rows after a cursor, without the clinical rows a legal mark withholds;
/// only the clinical schema has the `ehr` the marks live on.
const fn feed_sql(domain: OutboxDomain) -> &'static str {
    match domain {
        OutboxDomain::Clinical => {
            "SELECT o.seq, o.envelope FROM event_outbox o \
             WHERE o.seq > $1 \
               AND NOT EXISTS (SELECT 1 FROM ehr e WHERE e.id = o.ehr_id \
                   AND (e.restricted_at IS NOT NULL \
                        OR e.access_restricted_at IS NOT NULL \
                        OR (e.research_objected_at IS NOT NULL \
                            AND e.research_objection_ground IS NULL))) \
               AND NOT EXISTS (SELECT 1 FROM version v \
                   JOIN vo_head h ON h.vo_id = v.vo_id \
                   WHERE v.contribution_id = o.contribution_id \
                     AND h.access_restricted_at IS NOT NULL) \
             ORDER BY o.seq LIMIT $2"
        }
        OutboxDomain::Party => {
            "SELECT seq, envelope FROM event_outbox WHERE seq > $1 ORDER BY seq LIMIT $2"
        }
    }
}

/// Up to `limit` events of `domain`'s outbox after the active `reader`'s
/// cursor, in `seq` order; `None` when the reader is unknown or inactive.
///
/// # Errors
///
/// [`OutboxError::Acquire`] when no connection can be obtained, and
/// [`OutboxError::Database`] when the registry or the outbox cannot be read.
pub async fn read(
    pool: &PgPool,
    domain: OutboxDomain,
    reader: &str,
    limit: i64,
) -> Result<Option<Vec<FeedEvent>>, OutboxError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(EXCLUSIVE_LOCK).execute(&mut *tx).await?;
    let Some(cursor) = active_cursor(&mut tx, reader, false).await? else {
        tx.rollback().await?;
        return Ok(None);
    };
    let rows: Vec<(i64, Json<Map<String, Value>>)> = sqlx::query_as(feed_sql(domain))
        .bind(cursor)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(
        rows.into_iter()
            .map(|(seq, Json(envelope))| FeedEvent { seq, envelope })
            .collect(),
    ))
}

/// The answer to an acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acknowledged {
    /// The cursor is at the acknowledged `seq`, moved or already there.
    Moved,
    /// The reader is unknown or inactive.
    Unknown,
    /// The acknowledged `seq` lies below the reader's cursor.
    Behind {
        /// The reader's cursor.
        cursor: i64,
    },
    /// The acknowledged `seq` lies above the highest row of the outbox.
    PastTail {
        /// The highest `seq` the reader may acknowledge.
        tail: i64,
    },
}

/// Move the active `reader`'s cursor to `last_seq`.
///
/// Acknowledging the current cursor again is [`Acknowledged::Moved`]; a
/// `last_seq` below the cursor or above the highest row moves nothing.
///
/// # Errors
///
/// [`OutboxError::Acquire`] when no connection can be obtained, and
/// [`OutboxError::Database`] when the registry cannot be read or written.
pub async fn acknowledge(
    pool: &PgPool,
    reader: &str,
    last_seq: i64,
) -> Result<Acknowledged, OutboxError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(EXCLUSIVE_LOCK).execute(&mut *tx).await?;
    let Some(cursor) = active_cursor(&mut tx, reader, true).await? else {
        tx.rollback().await?;
        return Ok(Acknowledged::Unknown);
    };
    if last_seq < cursor {
        tx.rollback().await?;
        return Ok(Acknowledged::Behind { cursor });
    }
    let tail: i64 =
        sqlx::query_scalar("SELECT GREATEST($1, COALESCE(max(seq), 0)) FROM event_outbox")
            .bind(cursor)
            .fetch_one(&mut *tx)
            .await?;
    if last_seq > tail {
        tx.rollback().await?;
        return Ok(Acknowledged::PastTail { tail });
    }
    sqlx::query(
        "UPDATE event_outbox_reader SET last_seq = $2, updated_at = now() WHERE reader = $1",
    )
    .bind(reader)
    .bind(last_seq)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Acknowledged::Moved)
}

/// Delete the rows older than the retention window that every active reader
/// has acknowledged, in the domain `pool`'s own outbox. Returns the number
/// pruned.
///
/// The floor is the lowest cursor among the active readers, read in the same
/// statement; with none the window alone decides.
///
/// # Errors
///
/// [`OutboxError::Acquire`] when no connection can be obtained, and
/// [`OutboxError::Database`] when the delete fails.
pub async fn prune(pool: &PgPool, retention: Duration) -> Result<u64, OutboxError> {
    let cutoff = format!("{} milliseconds", retention.as_millis());
    let result = sqlx::query(
        "DELETE FROM event_outbox \
         WHERE committed_at < now() - $1::interval \
           AND seq <= COALESCE( \
                 (SELECT min(last_seq) FROM event_outbox_reader WHERE active), seq)",
    )
    .bind(cutoff)
    .execute(&mut *acquire(pool).await?)
    .await?;
    let pruned = result.rows_affected();
    if pruned > 0 {
        tracing::debug!("pruned {pruned} outbox rows past retention");
    }
    Ok(pruned)
}

/// Prune both outboxes every `interval`, for as long as the task runs.
///
/// A failed pass is logged and the next one runs on schedule.
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel: the binary aborts it \
              at exit, as it does the planner-statistics refresh"
)]
pub async fn scheduled_prune(pools: DomainPools, retention: Duration, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        for domain in OutboxDomain::ALL {
            if let Err(error) = prune(domain.pool(&pools), retention).await {
                tracing::warn!(
                    domain = domain.as_str(),
                    error = %ErrorChain::new(&error),
                    "pruning the outbox failed; the next pass runs on schedule"
                );
            }
        }
    }
}

impl FerroEhrService {
    /// The pool serving `domain`'s outbox.
    const fn outbox_pool(&self, domain: OutboxDomain) -> &PgPool {
        match domain {
            OutboxDomain::Clinical => &self.pool,
            OutboxDomain::Party => &self.demographic_pool,
        }
    }

    /// Up to `limit` events of `domain`'s outbox after `reader`'s cursor, in
    /// `seq` order.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] when the reader is unknown or inactive, and
    /// a database failure otherwise.
    pub async fn outbox_events(
        &self,
        domain: OutboxDomain,
        reader: &str,
        limit: i64,
    ) -> Result<Vec<FeedEvent>, ServiceError> {
        read(self.outbox_pool(domain), domain, reader, limit)
            .await?
            .ok_or_else(|| unknown_reader(reader))
    }

    /// Move `reader`'s cursor over `domain`'s outbox to `last_seq`.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NotFound`] when the reader is unknown or inactive,
    /// [`ServiceError::Conflict`] when `last_seq` lies below the cursor or
    /// above the highest row, and a database failure otherwise.
    pub async fn outbox_acknowledge(
        &self,
        domain: OutboxDomain,
        reader: &str,
        last_seq: i64,
    ) -> Result<(), ServiceError> {
        match acknowledge(self.outbox_pool(domain), reader, last_seq).await? {
            Acknowledged::Moved => Ok(()),
            Acknowledged::Unknown => Err(unknown_reader(reader)),
            Acknowledged::Behind { cursor } => Err(ServiceError::conflict(format!(
                "last_seq {last_seq} lies below the cursor of reader {reader:?}, which is at \
                 {cursor}; a cursor never moves back"
            ))),
            Acknowledged::PastTail { tail } => Err(ServiceError::conflict(format!(
                "last_seq {last_seq} lies past the highest outbox row the reader \
                 {reader:?} can acknowledge, {tail}"
            ))),
        }
    }
}

/// The `404` of a reader that is unknown or inactive.
fn unknown_reader(reader: &str) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!(
            "no active outbox reader {reader:?}; readers come from [record.change_feed] readers"
        ),
    )
}
