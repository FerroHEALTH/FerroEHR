// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What each sink that delivers from the store reads next (#6242).
//!
//! It reads the oldest parked record an operator asked to retry, else the
//! older of the two logs' next records past the sink's cursor up to the
//! delivery frontier (`super::delivery`), and renders the document or message
//! from the stored record when it is read. No specification governs it: our
//! own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store hands the \
              FHIR Feed an already-rendered FHIR document"
)]

use std::sync::LazyLock;

use secrecy::SecretSlice;
use sqlx::Row;

use super::chains::AuditChain;
use super::delivery::{Due, Sink};
use super::{AuditStore, RECORD_COLUMNS};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::fhir::value_of;

/// The statement that reads what `$1`, a sink, delivers next: the oldest
/// parked record an operator asked to retry, and each log's next record past
/// the sink's cursor up to the delivery frontier that is not parked, each
/// with `retry` beside [`RECORD_COLUMNS`].
pub(super) fn next_statement() -> &'static str {
    static NEXT: LazyLock<String> = LazyLock::new(|| {
        let columns: Vec<String> = RECORD_COLUMNS
            .iter()
            .map(|column| format!("e.{column}"))
            .collect();
        let columns = columns.join(", ");
        let next = |chain: &str| {
            format!(
                "(SELECT false AS retry, {columns} FROM audit.audit_record e \
                 JOIN audit.delivery_cursor c ON c.sink = $1 AND c.chain = '{chain}' \
                 JOIN audit.delivery_frontier f ON f.chain = '{chain}' \
                 WHERE e.chain = '{chain}' AND e.chain_seq > c.through_id \
                   AND e.chain_seq <= f.through_id \
                   AND NOT EXISTS (SELECT 1 FROM audit.delivery_parked p \
                                    WHERE p.sink = $1 AND p.chain = '{chain}' \
                                      AND p.record_id = e.chain_seq) \
                 ORDER BY e.chain_seq LIMIT 1)"
            )
        };
        format!(
            "(SELECT true AS retry, {columns} FROM audit.delivery_parked p \
             JOIN audit.audit_record e ON e.chain = p.chain AND e.chain_seq = p.record_id \
             WHERE p.sink = $1 AND p.retry ORDER BY p.parked_at, p.chain, p.record_id LIMIT 1) \
             UNION ALL {} UNION ALL {}",
            next("system"),
            next("access_log")
        )
    });
    NEXT.as_str()
}

/// The statement that counts what `$1`, a sink, has still to deliver: the
/// records past its cursor up to the delivery frontier, and the records it
/// parked.
const WAITING: &str = "SELECT \
     (SELECT count(*) FROM audit.audit_event e \
        JOIN audit.delivery_cursor c ON c.sink = $1 AND c.chain = 'system' \
        JOIN audit.delivery_frontier f ON f.chain = 'system' \
       WHERE e.seq > c.through_id AND e.seq <= f.through_id) \
   + (SELECT count(*) FROM audit.access_log_entry e \
        JOIN audit.delivery_cursor c ON c.sink = $1 AND c.chain = 'access_log' \
        JOIN audit.delivery_frontier f ON f.chain = 'access_log' \
       WHERE e.id > c.through_id AND e.id <= f.through_id AND e.render_policy_id IS NOT NULL), \
     (SELECT count(*) FROM audit.delivery_parked p WHERE p.sink = $1)";

/// What a sink delivers of one record: the FHIR document or the syslog
/// message.
#[derive(Debug)]
pub enum Delivered {
    /// The FHIR R4 `AuditEvent` the feed sends.
    Document(serde_json::Value),
    /// The DICOM PS3.15 A.5 audit message the syslog frame carries, and the
    /// time it names.
    Message(jiff::Timestamp, SecretSlice<u8>),
}

/// The next record a sink owes and what it delivers of it, or why that
/// cannot be rendered.
pub type Next = (Due, Result<Delivered, AuditError>);

/// How much a sink has still to deliver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Waiting {
    /// The records past its cursor up to the delivery frontier.
    pub owed: u64,
    /// The records it parked.
    pub parked: u64,
}

impl AuditStore {
    /// The next record `sink` owes, a retried parked record before the
    /// cursor's next, the older of the two logs' next records otherwise, and
    /// how much it has still to deliver. A record that cannot be rendered
    /// comes back with the error, for the sink to park.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a SELECT fails.
    pub async fn next_owed(&self, sink: Sink) -> Result<(Option<Next>, Waiting), AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(next_statement()))
            .bind(sink.as_str())
            .fetch_all(&mut *conn)
            .await?;
        let (owed, parked): (i64, i64) = sqlx::query_as(WAITING)
            .bind(sink.as_str())
            .fetch_one(&mut *conn)
            .await?;
        drop(conn);
        let waiting = Waiting {
            owed: u64::try_from(owed).unwrap_or(0),
            parked: u64::try_from(parked).unwrap_or(0),
        };
        let mut chosen: Option<(bool, jiff::Timestamp, &sqlx::postgres::PgRow)> = None;
        for row in &rows {
            let retry: bool = row.try_get("retry")?;
            let at = row
                .try_get::<jiff_sqlx::Timestamp, _>("recorded_at")?
                .to_jiff();
            // NOTE: our own design (#6242), a retry first, then the older log's next record.
            let better = match chosen {
                None => true,
                Some((chosen_retry, chosen_at, _)) => !chosen_retry && (retry || at < chosen_at),
            };
            if better {
                chosen = Some((retry, at, row));
            }
        }
        let Some((retry, _, row)) = chosen else {
            return Ok((None, waiting));
        };
        let chain: String = row.try_get("chain")?;
        let chain = AuditChain::named(&chain).ok_or(AuditError::Unreadable {
            chain_seq: row.try_get("chain_seq")?,
            reason: "it names a log the store does not hold",
        })?;
        let owed = Due {
            chain,
            id: row.try_get("chain_seq")?,
            retry,
        };
        let delivered = match sink {
            Sink::FhirFeed => self
                .resource(row, row.try_get("shared_part")?)
                .await
                .and_then(|resource| value_of(&resource))
                .map(Delivered::Document),
            Sink::Syslog => self.message(row).await,
        };
        Ok((Some((owed, delivered)), waiting))
    }

    /// The DICOM message of the record `row` holds, under the policy it was
    /// written under.
    async fn message(&self, row: &sqlx::postgres::PgRow) -> Result<Delivered, AuditError> {
        let (policy, stored) = self.record_of(row).await?;
        let (event, subject) = crate::system_log::render::classified(stored, &policy.categories)?;
        let message =
            crate::system_log::message::to_xml(&event, &policy.context, subject.as_deref())?;
        Ok(Delivered::Message(event.timestamp, message))
    }
}
