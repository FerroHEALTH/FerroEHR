// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log entry as the stored record of the access it covers, and
//! its projection as the document the ITI-81 search and the FHIR Feed serve
//! (#5127).
//!
//! A record whose access writes an entry is stored as the entry alone, in the
//! access log's own chain, apart from the `audit_event` records, with one row
//! of `audit.access_log_search` beside it: the columns the search filters on,
//! read from the entry's projection, and the render policy that projection is
//! written under (`migrations/audit/0030_access_log_projections.sql`). A read
//! renders the document from the entry's content. Both are written into the
//! intake before the response and chained after
//! (`migrations/audit/0042_audit_intake.sql`). A record an act wrote as its
//! entry in its own transaction ([`EntryScope::Written`]) is written by that
//! act alone, so it is stored once. No openEHR spec governs
//! audit storage: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store carries an \
              already-rendered FHIR document, never a typed resource"
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use sqlx::postgres::{PgConnection, PgRow};
use sqlx::{Connection, Row};

use crate::access_log::intake::{IndexRow, IntakeEntry};

use super::chains::AuditChain;
use super::writers::Pinned;
use super::{AuditRecord, AuditStore, LearnedSets, RecordEnd, stored_record};
use crate::access_log::projection::Projection;
use crate::access_log::store::AccessLogStore;
use crate::system_log::AuditError;
use crate::system_log::event::{AuditEvent, EntryScope, EventOutcome};
use crate::system_log::render::{CompiledPolicy, StoredRecord};

/// The records of a batch that are stored by their columns, with their
/// retention ends, and the records that are stored as their entry; a record
/// whose act wrote its entry itself is neither.
pub(super) fn split(
    records: Vec<AuditRecord>,
    ends: &[RecordEnd],
) -> (Vec<AuditRecord>, Vec<RecordEnd>, Vec<AuditRecord>) {
    let mut rows = Vec::new();
    let mut row_ends = Vec::new();
    let mut entries = Vec::new();
    for (index, record) in records.into_iter().enumerate() {
        if record.event.entry == EntryScope::Written {
            continue;
        }
        if record.entry.is_some() {
            entries.push(record);
        } else {
            row_ends.extend(ends.get(index).copied());
            rows.push(record);
        }
    }
    (rows, row_ends, entries)
}

/// The intake rows of a batch of entries: each one's uid, its row and its
/// search tokens.
pub(super) type EntryRows = (
    Vec<uuid::Uuid>,
    Vec<IntakeEntry>,
    Vec<Vec<crate::access_log::intake::IntakeToken>>,
);

impl AuditStore {
    /// Write the entry of each of `records` into the intake on `conn`, each
    /// with its index row; returns the entries' uids and the shared token
    /// sets learned.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when an entry has no stored form, and the
    /// projection and store errors of the index rows.
    pub(super) async fn intake_entries(
        &self,
        conn: &mut PgConnection,
        writing: &Pinned,
        records: Vec<AuditRecord>,
        reserved_by: Option<uuid::Uuid>,
    ) -> Result<(Vec<uuid::Uuid>, LearnedSets), AuditError> {
        let (uids, intake, tokens) = self.entry_rows(writing, records).await?;
        crate::access_log::intake::take_in(conn, &intake, &tokens, reserved_by).await?;
        Ok((uids, LearnedSets::default()))
    }

    /// The intake rows of the entries `records` carry, with each one's uid
    /// and search tokens, in their order.
    ///
    /// # Errors
    /// As [`Self::intake_entries`], less the store's.
    pub(super) async fn entry_rows(
        &self,
        writing: &Pinned,
        records: Vec<AuditRecord>,
    ) -> Result<EntryRows, AuditError> {
        let mut entered = Vec::with_capacity(records.len());
        let mut tokens = Vec::with_capacity(records.len());
        for record in records {
            let Some(entry) = record.entry else {
                continue;
            };
            let under = writing.of(&record.event);
            let digest = Self::registered_digest(&under)?;
            let policy = under.compiled().await?;
            let projection = Projection {
                context: &policy.context,
                categories: &policy.categories,
            };
            let (event, subject) = projection.record(&entry, 0)?;
            let document =
                crate::system_log::fhir::to_fhir(&event, projection.context, subject.as_deref());
            tokens.push(
                tokens_of(document, &event)
                    .into_iter()
                    .map(|token| crate::access_log::intake::IntakeToken {
                        shares: super::AuditElement::shares(token.parameter),
                        parameter: token.parameter,
                        system: token.system,
                        code: token.code,
                    })
                    .collect::<Vec<_>>(),
            );
            entered.push((entry, index_row(&event, subject, digest)));
        }
        let mut uids = Vec::with_capacity(entered.len());
        let mut intake = Vec::with_capacity(entered.len());
        for (mut entry, index) in entered {
            uids.push(uuid::Uuid::parse_str(entry.uid.value()).map_err(AuditError::EntryUid)?);
            intake.push(IntakeEntry::of(&mut entry, Some(index))?);
        }
        Ok((uids, intake, tokens))
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
        writing: &Pinned,
        mut records: Vec<AuditRecord>,
    ) -> Result<Vec<AuditRecord>, AuditError> {
        for record in &mut records {
            if record.entry.is_none() && crate::access_log::entry::writes_entry(&record.event) {
                let policy = writing.of(&record.event).compiled().await?;
                record.entry = crate::access_log::entry::for_store(
                    &record.event,
                    record.subject.as_deref(),
                    &policy.context,
                );
            }
        }
        Ok(records)
    }
    /// Replace the access-log entry `uid` by its tombstone, removed at
    /// `time_removed` on `grounds`, and write `event`, the record of the
    /// change, into the intake as its own entry, in one transaction, so the
    /// log holds the tombstone and the record of its making or neither;
    /// `None` when no entry has the uid.
    ///
    /// Every intake row is chained first, so an entry written moments ago is
    /// in the log the tombstone replaces it in, and the record of the
    /// erasure is chained before this returns. The record is written once,
    /// as its entry: the act names it [`EntryScope::Written`], so the request
    /// writes no second record of it.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when the tombstone or the entry cannot be
    /// written, the chaining errors of [`Self::settle`], and the projection
    /// and store errors of the index row.
    pub async fn erase_entry(
        &self,
        uid: &str,
        time_removed: DvDateTime,
        grounds: NonEmptyVec<DvCodedText>,
        event: &AuditEvent,
    ) -> Result<Option<AccessLogTombstone>, AuditError> {
        self.settle(super::intake::Chains::BOTH).await?;
        let record = AuditRecord {
            event: event.clone(),
            subject: None,
            entry: None,
        };
        let writing = self.writing.pin();
        let recorded = self.with_entries(&writing, vec![record]).await?;
        self.register_policies(&writing, &recorded).await?;
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let tombstone = AccessLogStore::tombstone_in(&mut tx, uid, time_removed, grounds).await?;
        if tombstone.is_none() {
            return Ok(None);
        }
        let (_uids, learned) =
            Box::pin(self.intake_entries(&mut tx, &writing, recorded, None)).await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this transaction's
        // rows.
        tx.commit().await?;
        // NOTE: our own design (#5879), the connection goes back before the chain settles, which
        // would otherwise wait for a second one while every connection is held so.
        drop(conn);
        self.remember(learned);
        self.settle(super::intake::Chains::BOTH).await?;
        Ok(tombstone)
    }

    /// The row of `audit.audit_record` at position `chain_seq` of `chain`.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub(super) async fn row_at(
        &self,
        chain: AuditChain,
        chain_seq: i64,
    ) -> Result<Option<PgRow>, AuditError> {
        use sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder};
        use sea_query_sqlx::SqlxBinder as _;

        let (sql, values) = super::newest_first(
            sea_query::Cond::all()
                .add(Expr::col(Alias::new("chain")).eq(chain.as_str()))
                .add(Expr::col(Alias::new("chain_seq")).eq(chain_seq)),
        )
        .build_sqlx(PostgresQueryBuilder);
        Ok(sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .fetch_optional(&mut *crate::db::acquire::acquire(&self.pool).await?)
            .await?)
    }

    /// The audit record at position `chain_seq` of `chain`, with its
    /// classification computed again and the subject of care of its EHR, as
    /// its projections read it; `None` when no record holds the position.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails, and the errors of
    /// reading the row and of the classification.
    pub async fn record(
        &self,
        chain: AuditChain,
        chain_seq: i64,
    ) -> Result<Option<(AuditEvent, Option<String>)>, AuditError> {
        let row = self.row_at(chain, chain_seq).await?;
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

/// The index row of the entry whose record is `event`, written under the
/// policy `digest`, its search tokens still to be filled in.
fn index_row(event: &AuditEvent, subject: Option<String>, digest: Vec<u8>) -> IndexRow {
    let nonempty = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    IndexRow {
        recorded_at: event.timestamp,
        action: event.action.as_code(),
        outcome: match event.outcome {
            EventOutcome::Success => 0,
            EventOutcome::MinorFailure => 4,
            EventOutcome::SeriousFailure => 8,
        },
        principal: nonempty(&event.user_id),
        patient_id: subject,
        resource_id: event.object_id.clone(),
        request_id: event.request_id.clone(),
        client_ip: event.client_ip.clone(),
        // NOTE: no specification governs this: our own design; only a classified access names its
        // EHR in the index, as only its EHDS access record names it.
        ehr_id: event.category.as_ref().and(event.ehr_id.clone()),
        search_tokens: String::new(),
        render_policy: digest,
    }
}
