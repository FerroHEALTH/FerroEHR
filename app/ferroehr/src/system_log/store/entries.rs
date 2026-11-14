// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log entry as the stored record of the access it covers, and
//! its projection as the document the ITI-81 search and the FHIR Feed serve
//! (#5127).
//!
//! A record whose access writes an entry is stored as the entry alone, in the
//! chain the `audit_event` records share, with one row of
//! `audit.access_log_search` beside it: the columns the search filters on,
//! read from the entry's projection, and the render policy that projection is
//! written under (`migrations/audit/0030_access_log_projections.sql`). A read
//! renders the document from the entry's content. No openEHR spec governs
//! audit storage: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use jiff_sqlx::Timestamp;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use sqlx::Row;
use sqlx::postgres::{PgConnection, PgRow};

use super::{AuditRecord, AuditStore, LearnedSets, RecordEnd, stored_record};
use crate::access_log::projection::Projection;
use crate::access_log::store::AccessLogStore;
use crate::system_log::AuditError;
use crate::system_log::event::{AuditEvent, EventOutcome};
use crate::system_log::render::{CompiledPolicy, StoredRecord};

/// The index rows of a batch of entries, one element per entry, as the
/// `unnest` of [`INDEX`] reads them.
#[derive(Debug, Default)]
struct IndexRows {
    chain_seqs: Vec<i64>,
    recorded_at: Vec<Timestamp>,
    actions: Vec<&'static str>,
    outcomes: Vec<i16>,
    principals: Vec<Option<String>>,
    patient_ids: Vec<Option<String>>,
    resource_ids: Vec<Option<String>>,
    request_ids: Vec<Option<String>>,
    client_ips: Vec<Option<String>>,
    ehr_ids: Vec<Option<String>>,
    tokens: Vec<String>,
    policies: Vec<Vec<u8>>,
}

/// The index rows of a batch of entries.
const INDEX: &str = "INSERT INTO audit.access_log_search (chain_seq, recorded_at, action, \
     outcome, principal, patient_id, resource_id, request_id, client_ip, ehr_id, search_tokens, \
     render_policy) \
     SELECT u.c1, u.c2, u.c3, u.c4, u.c5, u.c6, u.c7, u.c8, u.c9, u.c10, u.c11::integer[], u.c12 \
     FROM UNNEST($1::bigint[], $2::timestamptz[], $3::text[], $4::smallint[], $5::text[], \
     $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], $11::text[], $12::bytea[]) \
     AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12)";

/// The records of a batch that are stored by their columns, with their
/// retention ends, and the records that are stored as their entry.
pub(super) fn split<'a>(
    records: &'a [AuditRecord],
    ends: &[RecordEnd],
) -> (Vec<AuditRecord>, Vec<RecordEnd>, Vec<&'a AuditRecord>) {
    let mut rows = Vec::new();
    let mut row_ends = Vec::new();
    let mut entries = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if record.entry.is_some() {
            entries.push(record);
        } else {
            rows.push(record.clone());
            row_ends.extend(ends.get(index).copied());
        }
    }
    (rows, row_ends, entries)
}

impl AuditStore {
    /// Append the entry of each of `records` on `conn`, with its index row,
    /// and enter it in the FHIR Feed's outbox when the feed delivers from this
    /// store; returns the shared token sets learned.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when an entry cannot be appended, and the
    /// projection and store errors of the index rows.
    pub(super) async fn append_entries(
        &self,
        conn: &mut PgConnection,
        records: &[&AuditRecord],
    ) -> Result<LearnedSets, AuditError> {
        let entries: Vec<AccessLogEntry> = records
            .iter()
            .filter_map(|record| record.entry.as_deref().cloned())
            .collect();
        if entries.is_empty() {
            return Ok(LearnedSets::default());
        }
        let access_log = self
            .access_log()
            .unwrap_or_else(|| AccessLogStore::new(self.pool.clone()));
        let linked = access_log.append_in(conn, entries).await?;
        let mut rows = IndexRows::default();
        let mut tokens = Vec::with_capacity(linked.len());
        for (record, entry) in records.iter().zip(&linked) {
            let writing = self.writing.of(&record.event);
            let digest = self.writing_digest(&writing).await?;
            let policy = writing.compiled().await?;
            let chain_seq = entry.link.sequence;
            let projection = Projection {
                context: &policy.context,
                categories: &policy.categories,
            };
            let (event, subject) = projection.record(entry, chain_seq)?;
            let document =
                crate::system_log::fhir::to_fhir(&event, projection.context, subject.as_deref());
            tokens.push(tokens_of(document, &event));
            rows.push(chain_seq, &event, subject, digest);
        }
        let (ids, learned) = self.tokens.record_tokens(conn, &tokens).await?;
        rows.tokens = token_texts(&ids);
        sqlx::query(INDEX)
            .bind(&rows.chain_seqs)
            .bind(&rows.recorded_at)
            .bind(&rows.actions)
            .bind(&rows.outcomes)
            .bind(&rows.principals)
            .bind(&rows.patient_ids)
            .bind(&rows.resource_ids)
            .bind(&rows.request_ids)
            .bind(&rows.client_ips)
            .bind(&rows.ehr_ids)
            .bind(&rows.tokens)
            .bind(&rows.policies)
            .execute(&mut *conn)
            .await?;
        if self.fhir_feed {
            sqlx::query(
                "INSERT INTO audit.fhir_feed_outbox (chain_seq) SELECT unnest($1::bigint[])",
            )
            .bind(&rows.chain_seqs)
            .execute(&mut *conn)
            .await?;
        }
        Ok(learned)
    }

    /// `records`, each whose access writes an access-log entry carrying it:
    /// the drain's records carry theirs, and the store builds the entry of any
    /// other under the policy it is written under, so no access the log
    /// covers is stored by its columns.
    ///
    /// # Errors
    /// The [`crate::system_log::render::RenderPolicy::compile`] errors.
    pub(super) async fn with_entries(
        &self,
        records: &[AuditRecord],
    ) -> Result<Vec<AuditRecord>, AuditError> {
        let mut entered = Vec::with_capacity(records.len());
        for record in records {
            let mut record = record.clone();
            if record.entry.is_none() && crate::access_log::entry::writes_entry(&record.event) {
                let policy = self.writing.of(&record.event).compiled().await?;
                record.entry = crate::access_log::entry::for_store(
                    &record.event,
                    record.subject.as_deref(),
                    &policy.context,
                );
            }
            entered.push(record);
        }
        Ok(entered)
    }

    /// Store `event`, an access the log covers, as its entry; returns the
    /// entry's uid.
    ///
    /// # Errors
    /// As [`AuditStore::insert_batch`].
    pub(super) async fn insert_entry(
        &self,
        event: &AuditEvent,
        subject: Option<&str>,
    ) -> Result<uuid::Uuid, AuditError> {
        let record = AuditRecord {
            event: event.clone(),
            subject: subject.map(str::to_owned),
            entry: None,
        };
        let entered = self.with_entries(std::slice::from_ref(&record)).await?;
        self.insert_batch(&entered, &[]).await?;
        entered
            .first()
            .and_then(|record| record.entry.as_ref())
            .and_then(|entry| uuid::Uuid::parse_str(entry.uid.value()).ok())
            .ok_or(AuditError::SearchToken)
    }

    /// The row of `audit.audit_record` at chain position `chain_seq`.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub(super) async fn row_at(&self, chain_seq: i64) -> Result<Option<PgRow>, AuditError> {
        use sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder};
        use sea_query_sqlx::SqlxBinder as _;

        let (sql, values) = super::newest_first(
            sea_query::Cond::all().add(Expr::col(Alias::new("chain_seq")).eq(chain_seq)),
        )
        .build_sqlx(PostgresQueryBuilder);
        Ok(sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .fetch_optional(&mut *crate::db::acquire::acquire(&self.pool).await?)
            .await?)
    }

    /// The audit record at chain position `chain_seq`, of either form, with
    /// its classification computed again and the subject of care of its EHR,
    /// as its projections read it; `None` when no record holds the position.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails, and the errors of
    /// reading the row and of the classification.
    pub async fn record(
        &self,
        chain_seq: i64,
    ) -> Result<Option<(AuditEvent, Option<String>)>, AuditError> {
        let row = self.row_at(chain_seq).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let (policy, stored) = self.record_of(&row).await?;
        crate::system_log::render::classified(stored, &policy.categories).map(Some)
    }

    /// The record `row` of `audit.audit_record` holds, and the policy it is
    /// rendered under: an entry read from its content, any other record from
    /// its columns.
    ///
    /// # Errors
    /// [`AuditError::Unreadable`] and [`AuditError::UnreadableJson`] when the
    /// row or its policy does not read, [`AuditError::Recorded`] when an
    /// entry does not read as an audit record.
    pub(crate) async fn record_of(
        &self,
        row: &PgRow,
    ) -> Result<(Arc<CompiledPolicy>, StoredRecord), AuditError> {
        if row.try_get::<Option<&str>, _>("content")?.is_none() {
            let (digest, stored) = stored_record(row)?;
            let policy = self.policy(stored.chain_seq, &digest).await?;
            return Ok((policy, stored));
        }
        let chain_seq: i64 = row.try_get("chain_seq")?;
        let entry: AccessLogEntry = crate::access_log::store::stored(row)?;
        let digest: Vec<u8> = row.try_get("render_policy")?;
        let policy = self.policy(chain_seq, &digest).await?;
        let stored = crate::access_log::recorded::stored(&entry, chain_seq)?;
        Ok((policy, stored))
    }
}

/// The ITI-81 search tokens of `document`, the rendering of `event`; none,
/// metered and logged, when it could not be rendered, since it cannot be
/// served either.
pub(super) fn tokens_of(
    document: Result<serde_json::Value, AuditError>,
    event: &AuditEvent,
) -> BTreeSet<crate::system_log::search::SearchToken> {
    match document {
        Ok(document) => crate::system_log::search::document_tokens(&document, event),
        Err(e) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_serialize_failed
                .add(1, &[]);
            tracing::warn!("ATNA audit record stored without search tokens: {e}");
            BTreeSet::new()
        }
    }
}

/// Each record's `search_tokens` as the text of the `integer[]` a batched
/// insert casts.
pub(super) fn token_texts(ids: &[Vec<i32>]) -> Vec<String> {
    ids.iter()
        .map(|ids| {
            let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
            format!("{{{}}}", ids.join(","))
        })
        .collect()
}

impl IndexRows {
    /// Adds the index row of the entry at `chain_seq` whose record is
    /// `event`, written under the policy `digest`.
    fn push(
        &mut self,
        chain_seq: i64,
        event: &AuditEvent,
        subject: Option<String>,
        digest: Vec<u8>,
    ) {
        let nonempty = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        self.chain_seqs.push(chain_seq);
        self.recorded_at.push(Timestamp::from(event.timestamp));
        self.actions.push(event.action.as_code());
        self.outcomes.push(match event.outcome {
            EventOutcome::Success => 0,
            EventOutcome::MinorFailure => 4,
            EventOutcome::SeriousFailure => 8,
        });
        self.principals.push(nonempty(&event.user_id));
        self.patient_ids.push(subject);
        self.resource_ids.push(event.object_id.clone());
        self.request_ids.push(event.request_id.clone());
        self.client_ips.push(event.client_ip.clone());
        // NOTE: no specification governs this: our own design; only a classified access names its
        // EHR in the index, as only its EHDS access record names it.
        self.ehr_ids
            .push(event.category.as_ref().and(event.ehr_id.clone()));
        self.policies.push(digest);
    }
}
