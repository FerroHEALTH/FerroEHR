// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The store as the outbox of each sink that delivers from it: the ITI-20
//! ATX: FHIR Feed (`audit.fhir_feed_outbox`) and the ITI-20 syslog route
//! (`audit.syslog_outbox`).
//!
//! The chainer writes a record's outbox rows in the transaction that chains
//! it (`migrations/audit/0006_baseline_intake.sql`), so a sink delivers only
//! stored records, rendered from the store when they are read, and a record
//! is owed until the repository accepts it. No specification governs where a
//! record waits for its delivery: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store hands the \
              FHIR Feed an already-rendered FHIR document"
)]

use std::sync::LazyLock;

use secrecy::SecretSlice;
use sqlx::Row;

use super::{AuditStore, RECORD_COLUMNS};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::fhir::value_of;

/// A sink that delivers from the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    /// The ITI-20 ATX: FHIR Feed.
    FhirFeed,
    /// The ITI-20 syslog route.
    Syslog,
}

impl Sink {
    /// The outbox table of the sink.
    const fn outbox(self) -> &'static str {
        match self {
            Self::FhirFeed => "audit.fhir_feed_outbox",
            Self::Syslog => "audit.syslog_outbox",
        }
    }

    /// The statement that counts the records the sink owes past `$1`.
    const fn waiting(self) -> &'static str {
        match self {
            Self::FhirFeed => {
                "SELECT count(*) FROM audit.fhir_feed_outbox WHERE NOT (id = ANY($1))"
            }
            Self::Syslog => "SELECT count(*) FROM audit.syslog_outbox WHERE NOT (id = ANY($1))",
        }
    }

    /// The statement that removes the record the sink owes under `$1`: the
    /// outbox row is the record's delivery state, and no record is changed
    /// (#6203).
    const fn delivered(self) -> &'static str {
        match self {
            Self::FhirFeed => "DELETE FROM audit.fhir_feed_outbox WHERE id = $1",
            Self::Syslog => "DELETE FROM audit.syslog_outbox WHERE id = $1",
        }
    }

    /// The statement that reads the oldest record the sink owes past the
    /// outbox ids in `$1`: its outbox id beside [`RECORD_COLUMNS`].
    pub(super) fn oldest(self) -> &'static str {
        static FHIR_FEED: LazyLock<String> = LazyLock::new(|| oldest_of(Sink::FhirFeed));
        static SYSLOG: LazyLock<String> = LazyLock::new(|| oldest_of(Sink::Syslog));
        match self {
            Self::FhirFeed => FHIR_FEED.as_str(),
            Self::Syslog => SYSLOG.as_str(),
        }
    }
}

/// The statement of [`Sink::oldest`].
fn oldest_of(sink: Sink) -> String {
    let columns: Vec<String> = RECORD_COLUMNS
        .iter()
        .map(|column| format!("e.{column}"))
        .collect();
    format!(
        "SELECT o.id AS owed, {} FROM {} o \
         JOIN audit.audit_record e ON e.chain = o.chain AND e.chain_seq = o.chain_seq \
         WHERE NOT (o.id = ANY($1)) ORDER BY o.id ASC LIMIT 1",
        columns.join(", "),
        sink.outbox()
    )
}

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

/// The oldest record a sink owes: its outbox id and what the sink delivers
/// of it, or why that cannot be rendered.
pub type Oldest = (i64, Result<Delivered, AuditError>);

impl AuditStore {
    /// The oldest record `sink` still owes, past the outbox ids in `skip`,
    /// and how many such records there are. A record that cannot be rendered
    /// comes back as its outbox id with the error, for the sink to hold aside.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a SELECT fails.
    pub async fn oldest_owed(
        &self,
        sink: Sink,
        skip: &[i64],
    ) -> Result<(Option<Oldest>, i64), AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(sink.oldest()))
            .bind(skip)
            .fetch_optional(&mut *conn)
            .await?;
        let waiting: i64 = sqlx::query_scalar(sink.waiting())
            .bind(skip)
            .fetch_one(&mut *conn)
            .await?;
        drop(conn);
        let Some(row) = row else {
            return Ok((None, waiting));
        };
        let owed = row.try_get::<i64, _>("owed")?;
        let delivered = match sink {
            Sink::FhirFeed => self
                .resource(&row)
                .await
                .and_then(|resource| value_of(&resource))
                .map(Delivered::Document),
            Sink::Syslog => self.message(&row).await,
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

    /// Removes the record `sink` owes under `owed` from its outbox: the
    /// outbox row is the record's delivery state, and the record is never
    /// changed (#6203).
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails; the record is then
    /// delivered again, which the repository takes as a second copy.
    pub async fn mark_delivered(&self, sink: Sink, owed: i64) -> Result<(), AuditError> {
        sqlx::query(sink.delivered())
            .bind(owed)
            .execute(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(())
    }
}
