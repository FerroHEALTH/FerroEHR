// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the access log its ITS-REST resources serve (#5130): a page
//! of records under a filter, newest first, and the links of a span of the
//! chain.
//!
//! The resources follow the ITS-REST 1.1.0 conventions the draft's ITS-REST
//! page names: `offset` and `fetch` paging as the Query API's
//! (`docs/specs/openehr/ITS-REST/specifications/parameters/query/offset.yaml`,
//! `fetch.yaml`). Which filters a resource offers and how a subject's own
//! reads of their log are told apart is our own design: no openEHR spec
//! governs the access log.

use jiff::Timestamp;
use sqlx::Row;

use crate::db::acquire::acquire;
use openehr_rm::draft::access_control::terminology::Term;
use openehr_rm::draft::access_control::terminology::log::{AccessorKind, Action, Outcome};
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

use super::store::{AccessLogError, AccessLogStore, link, stored};

/// The `ACCESSED_ITEM.rm_type` of an entry that records a read of the access
/// log itself (rule 4 of the draft's recording rules).
pub const LOG_RECORD: &str = "ACCESS_LOG_RECORD";

/// The `ACCESS_QUERY.language` of a read of the access log through its
/// ITS-REST resources, whose filter is the query string.
pub const LOG_QUERY: &str = "ITS-REST access log filter";

/// The party a filter names: the `PARTY_REF` of `ACCESSOR.party`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartyFilter {
    /// `PARTY_REF.namespace`.
    pub namespace: Option<String>,
    /// `PARTY_REF.type`.
    pub class: Option<String>,
    /// `PARTY_REF.id.value`.
    pub id: Option<String>,
}

/// What a read of the log selects; every filter left `None` selects all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    /// The EHR the entries are about.
    pub ehr_id: Option<String>,
    /// The first instant of `time_recorded` selected.
    pub from: Option<Timestamp>,
    /// The last instant of `time_recorded` selected.
    pub to: Option<Timestamp>,
    /// The term of `ACCESS_LOG_ENTRY.action`.
    pub action: Option<Action>,
    /// The term of `ACCESS_OUTCOME.result`.
    pub outcome: Option<Outcome>,
    /// The term of `ACCESSOR.kind`.
    pub accessor_kind: Option<AccessorKind>,
    /// `is_emergency`.
    pub emergency: Option<bool>,
    /// The accessing party.
    pub party: PartyFilter,
    /// `ACCESS_CREDENTIAL.issuer` of the accessor's credential.
    pub issuer: Option<String>,
    /// `ACCESS_CREDENTIAL.subject` of the accessor's credential.
    pub subject: Option<String>,
    /// `ACCESS_CHANNEL.request_id`.
    pub request_id: Option<String>,
    /// Whether the subject of care's own reads of the log are left out.
    pub without_own_log_reads: bool,
    /// The number of records skipped.
    pub offset: i64,
    /// The most records answered.
    pub fetch: i64,
}

impl Default for LogQuery {
    /// Every record, the first page of 100.
    fn default() -> Self {
        Self {
            ehr_id: None,
            from: None,
            to: None,
            action: None,
            outcome: None,
            accessor_kind: None,
            emergency: None,
            party: PartyFilter::default(),
            issuer: None,
            subject: None,
            request_id: None,
            without_own_log_reads: false,
            offset: 0,
            // NOTE: ITS-REST `fetch.yaml`, "the default depends on the implementation": ours.
            fetch: 100,
        }
    }
}

impl AccessLogStore {
    /// The records `query` selects, newest first, entries and tombstones,
    /// each with its link.
    ///
    /// A tombstone keeps no `ehr_id` and no accessor, so a filter on either
    /// leaves it out.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn search(&self, query: &LogQuery) -> Result<Vec<AccessLogRecord>, AccessLogError> {
        let rows = sqlx::query(
            "SELECT chain_seq, content, algorithm, previous_digest, content_digest, digest, \
             signature FROM audit.access_log_record r, LATERAL (SELECT r.content::jsonb AS doc) d \
             WHERE ($1::text IS NULL OR r.ehr_id = $1) \
             AND ($2::text IS NULL OR r.time_recorded >= $2::text::timestamptz) \
             AND ($3::text IS NULL OR r.time_recorded <= $3::text::timestamptz) \
             AND ($4::text IS NULL OR (r.action_terminology = $4 AND r.action_code = $5)) \
             AND ($6::text IS NULL OR (r.result_terminology = $6 AND r.result_code = $7)) \
             AND ($8::text IS NULL \
                  OR (d.doc #>> '{accessor,kind,defining_code,terminology_id,value}' = $8 \
                      AND d.doc #>> '{accessor,kind,defining_code,code_string}' = $9)) \
             AND ($10::boolean IS NULL OR (d.doc ->> 'is_emergency')::boolean = $10) \
             AND ($11::text IS NULL OR d.doc #>> '{accessor,party,external_ref,namespace}' = $11) \
             AND ($12::text IS NULL OR d.doc #>> '{accessor,party,external_ref,type}' = $12) \
             AND ($13::text IS NULL OR r.party_id = $13) \
             AND ($14::text IS NULL OR d.doc #>> '{accessor,credential,issuer}' = $14) \
             AND ($15::text IS NULL OR d.doc #>> '{accessor,credential,subject}' = $15) \
             AND ($16::text IS NULL OR r.request_id = $16) \
             AND NOT ($17::boolean \
                  AND d.doc #>> '{accessor,kind,defining_code,terminology_id,value}' \
                      IS NOT DISTINCT FROM $18 \
                  AND d.doc #>> '{accessor,kind,defining_code,code_string}' IS NOT DISTINCT FROM $19 \
                  AND d.doc @> jsonb_build_object('items', jsonb_build_array( \
                      jsonb_build_object('rm_type', $20::text)))) \
             ORDER BY r.time_recorded DESC, r.chain_seq DESC OFFSET $21 LIMIT $22",
        )
        .bind(query.ehr_id.as_deref())
        .bind(query.from.map(|from| from.to_string()))
        .bind(query.to.map(|to| to.to_string()))
        .bind(query.action.map(Term::terminology))
        .bind(query.action.map(Term::code))
        .bind(query.outcome.map(Term::terminology))
        .bind(query.outcome.map(Term::code))
        .bind(query.accessor_kind.map(Term::terminology))
        .bind(query.accessor_kind.map(Term::code))
        .bind(query.emergency)
        .bind(query.party.namespace.as_deref())
        .bind(query.party.class.as_deref())
        .bind(query.party.id.as_deref())
        .bind(query.issuer.as_deref())
        .bind(query.subject.as_deref())
        .bind(query.request_id.as_deref())
        .bind(query.without_own_log_reads)
        .bind(AccessorKind::SubjectOfCare.terminology())
        .bind(AccessorKind::SubjectOfCare.code())
        .bind(LOG_RECORD)
        .bind(query.offset)
        .bind(query.fetch)
        .fetch_all(&mut *acquire(self.pool()).await?)
        .await?;
        rows.iter().map(stored).collect()
    }

    /// The records the statement `sql`, bound to `values`, selects with the
    /// record columns [`Self::search`] reads, in its order: the statement an AQL query over
    /// the access log runs (`crate::aql::access_log::sql`).
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored record does not read.
    pub async fn select(
        &self,
        sql: String,
        values: sea_query_sqlx::SqlxValues,
    ) -> Result<Vec<AccessLogRecord>, AccessLogError> {
        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .persistent(false)
            .fetch_all(&mut *acquire(self.pool()).await?)
            .await?;
        rows.iter().map(stored).collect()
    }

    /// The uid and link of every access-log record from `from` to `to`, both
    /// included, in chain order, without their content.
    ///
    /// A position a gap holds has no record (#5619): [`Self::gaps`] answers
    /// those, and a position neither holds is a deletion the verifier
    /// reports.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn chain(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<(String, AccessLogLink)>, AccessLogError> {
        let rows = sqlx::query(
            "SELECT uid, chain_seq, algorithm, previous_digest, content_digest, digest, signature \
             FROM audit.access_log_record WHERE chain_seq BETWEEN $1 AND $2 ORDER BY chain_seq",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&mut *acquire(self.pool()).await?)
        .await?;
        let links = rows
            .iter()
            .map(|row| Ok((row.try_get::<String, _>("uid")?, link(row)?)))
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(links)
    }
}
