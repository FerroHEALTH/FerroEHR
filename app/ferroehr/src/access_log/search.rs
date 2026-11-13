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
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

use super::store::{AccessLogError, AccessLogStore, link, stored};

/// The `ACCESSED_ITEM.rm_type` of an entry that records a read of the access
/// log itself (rule 4 of the draft's recording rules).
pub const LOG_RECORD: &str = "ACCESS_LOG_RECORD";

/// The `ACCESS_QUERY.language` of a read of the access log through its
/// ITS-REST resources, whose filter is the query string.
pub const LOG_QUERY: &str = "ITS-REST access log filter";

/// The `accessor kind` rubric of the subject of care.
pub const SUBJECT_OF_CARE: &str = "subject of care";

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    /// The EHR the entries are about.
    pub ehr_id: Option<String>,
    /// The first instant of `time_recorded` selected.
    pub from: Option<Timestamp>,
    /// The last instant of `time_recorded` selected.
    pub to: Option<Timestamp>,
    /// The `access action` rubric.
    pub action: Option<String>,
    /// The `access outcome` rubric.
    pub outcome: Option<String>,
    /// The `accessor kind` rubric.
    pub accessor_kind: Option<String>,
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
             AND ($4::text IS NULL OR r.action = $4) \
             AND ($5::text IS NULL OR r.result = $5) \
             AND ($6::text IS NULL OR d.doc #>> '{accessor,kind,defining_code,code_string}' = $6) \
             AND ($7::boolean IS NULL OR (d.doc ->> 'is_emergency')::boolean = $7) \
             AND ($8::text IS NULL OR d.doc #>> '{accessor,party,external_ref,namespace}' = $8) \
             AND ($9::text IS NULL OR d.doc #>> '{accessor,party,external_ref,type}' = $9) \
             AND ($10::text IS NULL OR r.party_id = $10) \
             AND ($11::text IS NULL OR d.doc #>> '{accessor,credential,issuer}' = $11) \
             AND ($12::text IS NULL OR d.doc #>> '{accessor,credential,subject}' = $12) \
             AND ($13::text IS NULL OR r.request_id = $13) \
             AND NOT ($14::boolean \
                  AND d.doc #>> '{accessor,kind,defining_code,code_string}' IS NOT DISTINCT FROM $15 \
                  AND d.doc @> jsonb_build_object('items', jsonb_build_array( \
                      jsonb_build_object('rm_type', $16::text)))) \
             ORDER BY r.time_recorded DESC, r.chain_seq DESC OFFSET $17 LIMIT $18",
        )
        .bind(query.ehr_id.as_deref())
        .bind(query.from.map(|from| from.to_string()))
        .bind(query.to.map(|to| to.to_string()))
        .bind(query.action.as_deref())
        .bind(query.outcome.as_deref())
        .bind(query.accessor_kind.as_deref())
        .bind(query.emergency)
        .bind(query.party.namespace.as_deref())
        .bind(query.party.class.as_deref())
        .bind(query.party.id.as_deref())
        .bind(query.issuer.as_deref())
        .bind(query.subject.as_deref())
        .bind(query.request_id.as_deref())
        .bind(query.without_own_log_reads)
        .bind(SUBJECT_OF_CARE)
        .bind(LOG_RECORD)
        .bind(query.offset)
        .bind(query.fetch)
        .fetch_all(&mut *acquire(self.pool()).await?)
        .await?;
        rows.iter().map(stored).collect()
    }

    /// The uid and link of every access-log record from `from` to `to`, both
    /// included, in chain order, without their content.
    ///
    /// # Errors
    /// [`AccessLogError::Interleaved`] when a position between holds no
    /// access-log record, and the store errors when the statement fails.
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
        let expected = to.checked_sub(from).and_then(|span| span.checked_add(1));
        if i64::try_from(links.len()).ok() != expected {
            return Err(AccessLogError::Interleaved { from, to });
        }
        Ok(links)
    }
}
