// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The delivery state of each sink that delivers from the store: the ITI-20
//! ATX: FHIR Feed and the ITI-20 syslog route (#6242).
//!
//! A sink delivers each log in id order from its cursor
//! (`audit.delivery_cursor`) up to the delivery frontier
//! (`audit.delivery_frontier`), which the frontier round moves while a sink
//! is configured and never past a record a write holds. The cursor moves
//! only after the repository accepted a record, so after a crash a sink
//! repeats at most the record it had in flight, and loses and skips none. A
//! record the sink cannot render, or its repository refuses, is parked
//! (`audit.delivery_parked`): counted, shown by the delivery indicator,
//! logged at `ERROR` by its id, and kept until it is delivered on an
//! operator's retry or an operator dismisses it; the cursor moves past it
//! only because it is parked. Retention removes or replaces a record a sink
//! has not delivered only a day ([`DELIVERY_GRACE_DAYS`]) after its end, and
//! parks it then (`migrations/audit/0003_baseline_delivery.sql`). What a sink
//! reads next and renders is `super::outbox`. No specification governs where
//! a record waits for its delivery: our own design.

use super::AuditStore;
use super::chains::AuditChain;
use crate::db::acquire::acquire;
use crate::system_log::AuditError;

/// How many days past its end retention waits for a sink to deliver a record.
///
/// Retention then removes or replaces it anyway and parks it for that sink.
/// No text sets it: our own design (#6242), long enough to ride out a
/// repository's outage of a working day.
pub const DELIVERY_GRACE_DAYS: u32 = 1;

/// A sink that delivers from the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sink {
    /// The ITI-20 ATX: FHIR Feed.
    FhirFeed,
    /// The ITI-20 syslog route.
    Syslog,
}

impl Sink {
    /// Both sinks.
    pub const ALL: [Self; 2] = [Self::FhirFeed, Self::Syslog];

    /// The sink's name as the delivery tables hold it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FhirFeed => "fhir_feed",
            Self::Syslog => "syslog",
        }
    }

    /// The sink the delivery tables' `sink` names; `None` for another name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sink| sink.as_str() == name)
    }
}

/// Why a sink parked a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parked {
    /// The record could not be rendered as the sink delivers it.
    Unrenderable,
    /// The sink's repository refused it.
    Refused,
}

impl Parked {
    /// The reason as `audit.delivery_parked.reason` holds it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unrenderable => "unrenderable",
            Self::Refused => "refused",
        }
    }
}

/// One record a sink owes: its log, its id, and whether an operator asked
/// to retry it after it was parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// The log the record is in.
    pub chain: AuditChain,
    /// The record's id in its log.
    pub id: i64,
    /// Whether it is a parked record an operator asked to retry.
    pub retry: bool,
}

impl Due {
    /// The record as one number the forwarder hands back: its id doubled,
    /// plus one for the access log.
    ///
    /// # Errors
    /// [`AuditError::Unreadable`] for a negative or overlarge id.
    pub fn sequence(self) -> Result<u64, AuditError> {
        let doubled = u64::try_from(self.id)
            .ok()
            .and_then(|id| id.checked_mul(2))
            .ok_or(AuditError::Unreadable {
                chain_seq: self.id,
                reason: "its id has no delivery sequence",
            })?;
        Ok(doubled + u64::from(self.chain == AuditChain::AccessLog))
    }

    /// The record a sequence of [`Self::sequence`] names, not retried.
    #[must_use]
    pub fn of_sequence(sequence: u64) -> Option<Self> {
        let chain = if sequence % 2 == 1 {
            AuditChain::AccessLog
        } else {
            AuditChain::System
        };
        Some(Self {
            chain,
            id: i64::try_from(sequence >> 1).ok()?,
            retry: false,
        })
    }
}

impl AuditStore {
    /// Notes that `sink`'s repository accepted `owed`: a retried parked
    /// record leaves the parked set, any other moves the sink's cursor to it.
    /// The record itself is never changed (#6203).
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails; the record is then
    /// delivered again, which the repository takes as a second copy.
    pub async fn mark_delivered(&self, sink: Sink, owed: Due) -> Result<(), AuditError> {
        sqlx::query(
            "WITH retried AS (DELETE FROM audit.delivery_parked \
              WHERE sink = $1 AND chain = $2 AND record_id = $3 RETURNING 1) \
             UPDATE audit.delivery_cursor SET through_id = $3 \
              WHERE sink = $1 AND chain = $2 AND through_id < $3 \
                AND NOT EXISTS (SELECT 1 FROM retried)",
        )
        .bind(sink.as_str())
        .bind(owed.chain.as_str())
        .bind(owed.id)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(())
    }

    /// Parks `owed` for `sink` with `reason` and moves the cursor past it: it
    /// stays parked until it is delivered on an operator's retry or an
    /// operator dismisses it. A retried record parked again waits for the
    /// next retry.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails; the record is then
    /// offered again.
    pub async fn park(&self, sink: Sink, owed: Due, reason: Parked) -> Result<(), AuditError> {
        sqlx::query(
            "WITH parked AS (INSERT INTO audit.delivery_parked (sink, chain, record_id, reason) \
              VALUES ($1, $2, $3, $4) ON CONFLICT (sink, chain, record_id) \
              DO UPDATE SET reason = excluded.reason, retry = false, parked_at = now() \
              RETURNING 1) \
             UPDATE audit.delivery_cursor SET through_id = $3 \
              WHERE sink = $1 AND chain = $2 AND through_id < $3 \
                AND EXISTS (SELECT 1 FROM parked)",
        )
        .bind(sink.as_str())
        .bind(owed.chain.as_str())
        .bind(owed.id)
        .bind(reason.as_str())
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(())
    }
}

/// One sink's cursor of one log, as the operator reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CursorView {
    /// The sink.
    pub sink: &'static str,
    /// The log.
    pub chain: &'static str,
    /// The id at or below which every record was delivered or parked.
    pub through_id: i64,
    /// The records past it the sink has still to deliver.
    pub behind: i64,
    /// Whether this server's configuration names the sink.
    pub configured: bool,
}

/// One parked record, as the operator reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ParkedView {
    /// The sink that parked it.
    pub sink: &'static str,
    /// Its log.
    pub chain: &'static str,
    /// Its id in its log.
    pub record_id: i64,
    /// Why it was parked.
    pub reason: String,
    /// When it was parked, an RFC 3339 instant.
    pub parked_at: String,
    /// Whether an operator asked to retry it.
    pub retry: bool,
}

/// The delivery state of every sink the store holds a cursor of.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct DeliveryView {
    /// Each cursor, configured or not.
    pub cursors: Vec<CursorView>,
    /// Each parked record, oldest first, at most [`PARKED_LISTED`].
    pub parked: Vec<ParkedView>,
}

/// The most parked records [`AuditStore::delivery_view`] lists.
pub const PARKED_LISTED: i64 = 1000;

/// The records past a cursor `c` of its log that its sink has still to
/// receive, up to no frontier: what a sink taken out of the configuration
/// missed.
const BEHIND: &str = "CASE c.chain \
     WHEN 'system' THEN (SELECT count(*) FROM audit.audit_event e WHERE e.seq > c.through_id) \
     ELSE (SELECT count(*) FROM audit.access_log_entry e \
            WHERE e.id > c.through_id AND e.render_policy_id IS NOT NULL) END";

impl AuditStore {
    /// Starts the cursor of each configured sink that has none at the last
    /// id each log has drawn: a sink delivers every record written from the
    /// moment it is configured, none written before.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn start_cursors(&self) -> Result<(), AuditError> {
        let sinks: Vec<&str> = Sink::ALL
            .into_iter()
            .filter(|sink| self.configured(*sink))
            .map(Sink::as_str)
            .collect();
        if sinks.is_empty() {
            return Ok(());
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "WITH drawn(access_log, system) AS ({}) \
             INSERT INTO audit.delivery_cursor (sink, chain, through_id) \
             SELECT s.sink, l.chain, coalesce(l.last, 0) FROM drawn \
              CROSS JOIN LATERAL (VALUES ('access_log', drawn.access_log), \
                                         ('system', drawn.system)) AS l(chain, last) \
              CROSS JOIN unnest($1::text[]) AS s(sink) \
             ON CONFLICT (sink, chain) DO NOTHING",
            crate::access_log::frontier::LAST_VALUES
        )))
        .bind(&sinks)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(())
    }
}

impl AuditStore {
    /// Whether this server's configuration names `sink`.
    fn configured(&self, sink: Sink) -> bool {
        match sink {
            Sink::FhirFeed => self.owed.fhir_feed,
            Sink::Syslog => self.owed.syslog,
        }
    }

    /// Every cursor the store holds, with how far it is behind, and the
    /// records parked, oldest first.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails, and
    /// [`AuditError::Unreadable`] for a sink or log name the store should
    /// not hold.
    pub async fn delivery_view(&self) -> Result<DeliveryView, AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let cursors: Vec<(String, String, i64, i64)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT c.sink, c.chain, c.through_id, {BEHIND} FROM audit.delivery_cursor c \
                 ORDER BY c.sink, c.chain"
            )))
            .fetch_all(&mut *conn)
            .await?;
        let parked: Vec<(String, String, i64, String, jiff_sqlx::Timestamp, bool)> =
            sqlx::query_as(
                "SELECT sink, chain, record_id, reason, parked_at, retry \
                 FROM audit.delivery_parked ORDER BY parked_at, sink, chain, record_id LIMIT $1",
            )
            .bind(PARKED_LISTED)
            .fetch_all(&mut *conn)
            .await?;
        let mut view = DeliveryView::default();
        for (sink, chain, through_id, behind) in cursors {
            let (sink, chain) = names(&sink, &chain, through_id)?;
            view.cursors.push(CursorView {
                sink: sink.as_str(),
                chain: chain.as_str(),
                through_id,
                behind,
                configured: self.configured(sink),
            });
        }
        for (sink, chain, record_id, reason, parked_at, retry) in parked {
            let (sink, chain) = names(&sink, &chain, record_id)?;
            view.parked.push(ParkedView {
                sink: sink.as_str(),
                chain: chain.as_str(),
                record_id,
                reason,
                parked_at: parked_at.to_jiff().to_string(),
                retry,
            });
        }
        Ok(view)
    }

    /// Each sink the store holds a cursor of that this server's
    /// configuration does not name, with how many records it has not
    /// received.
    ///
    /// # Errors
    /// As [`Self::delivery_view`].
    pub async fn unconfigured_sinks(&self) -> Result<Vec<(Sink, i64)>, AuditError> {
        let mut behind: std::collections::BTreeMap<Sink, i64> = std::collections::BTreeMap::new();
        for cursor in self.delivery_view().await?.cursors {
            if let (false, Some(sink)) = (cursor.configured, Sink::named(cursor.sink)) {
                *behind.entry(sink).or_default() += cursor.behind;
            }
        }
        Ok(behind.into_iter().collect())
    }

    /// Asks that `sink` deliver its parked record `id` of `chain` again,
    /// before the next record past its cursor; returns whether it was
    /// parked.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn retry_parked(
        &self,
        sink: Sink,
        chain: AuditChain,
        id: i64,
    ) -> Result<bool, AuditError> {
        let changed = sqlx::query(
            "UPDATE audit.delivery_parked SET retry = true \
             WHERE sink = $1 AND chain = $2 AND record_id = $3",
        )
        .bind(sink.as_str())
        .bind(chain.as_str())
        .bind(id)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(changed.rows_affected() > 0)
    }

    /// Removes `sink`'s parked record `id` of `chain`, which an operator
    /// decided it will not deliver; returns whether it was parked.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn dismiss_parked(
        &self,
        sink: Sink,
        chain: AuditChain,
        id: i64,
    ) -> Result<bool, AuditError> {
        let changed = sqlx::query(
            "DELETE FROM audit.delivery_parked WHERE sink = $1 AND chain = $2 AND record_id = $3",
        )
        .bind(sink.as_str())
        .bind(chain.as_str())
        .bind(id)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(changed.rows_affected() > 0)
    }

    /// Removes `sink`'s cursors and parked records, which an operator decided
    /// it will not deliver; returns how many cursors and parked records went.
    /// A sink configured again starts at the delivery frontier then.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn forget_sink(&self, sink: Sink) -> Result<(u64, u64), AuditError> {
        let (cursors, parked): (i64, i64) = sqlx::query_as(
            "WITH cursors AS (DELETE FROM audit.delivery_cursor WHERE sink = $1 RETURNING 1), \
             parked AS (DELETE FROM audit.delivery_parked WHERE sink = $1 RETURNING 1) \
             SELECT (SELECT count(*) FROM cursors), (SELECT count(*) FROM parked)",
        )
        .bind(sink.as_str())
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?;
        Ok((
            u64::try_from(cursors).unwrap_or(0),
            u64::try_from(parked).unwrap_or(0),
        ))
    }
}

/// The sink and log a delivery row names, or why it does not read.
fn names(sink: &str, chain: &str, id: i64) -> Result<(Sink, AuditChain), AuditError> {
    match (Sink::named(sink), AuditChain::named(chain)) {
        (Some(sink), Some(chain)) => Ok((sink, chain)),
        _ => Err(AuditError::Unreadable {
            chain_seq: id,
            reason: "a delivery row names a sink or log the store does not hold",
        }),
    }
}
