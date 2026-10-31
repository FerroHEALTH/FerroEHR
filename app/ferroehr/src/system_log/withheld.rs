// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The person's own access trail while data is held from them.
//!
//! Regulation (EU) 2025/327 Art. 9(1) gives the person the record of every
//! access to their data, and Art. 3(3) delays their access to newly
//! registered data "until a health professional is able to properly
//! communicate and explain" it (`docs/law/eu/ehds/text.html`). A record about
//! a held composition would tell the person it exists, so the person's read
//! leaves it out through `ehds_logging::withheld` until the hold is released
//! or expires, and then serves it unchanged with its original time. No
//! openEHR spec governs it: our own design/extension.

use ehds_logging::withheld::{Holds, Reader};
use jiff::Timestamp;
use sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder, Query};
use sea_query_sqlx::SqlxBinder as _;

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
use crate::system_log::event::AuditEvent;
use crate::system_log::fhir::{access_record, to_fhir};
use crate::system_log::message::AuditContext;
use crate::system_log::render::classified;
use crate::system_log::search::condition::search_condition;
use crate::system_log::search::resolve_criteria;
use crate::system_log::store::{AuditSearchFilter, AuditStore, newest_first, stored_record};

impl AuditStore {
    /// The EHR ids the records `filter` selects name, each once.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a query fails.
    pub async fn ehrs_of(&self, filter: &AuditSearchFilter) -> Result<Vec<String>, AuditError> {
        let Some(resolved) = resolve_criteria(&self.pool, filter).await? else {
            return Ok(Vec::new());
        };
        let (sql, values) = Query::select()
            .distinct()
            .column(Alias::new("ehr_id"))
            .from((Alias::new("audit"), Alias::new("audit_event")))
            .cond_where(search_condition(filter, &resolved))
            .and_where(Expr::col(Alias::new("ehr_id")).is_not_null())
            .build_sqlx(PostgresQueryBuilder);
        Ok(sqlx::query_scalar_with(sqlx::AssertSqlSafe(sql), values)
            .persistent(false)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?)
    }

    /// The ITI-81 retrieval of the person's own trail: the records `filter`
    /// selects that [`Holds::trail`] shows the person at `now`, newest first,
    /// and how many there are.
    ///
    /// Every matching record is read to count what the person sees, so the
    /// total and the pages agree; the service takes this path only while a
    /// hold of the person is in force.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a query fails, and the rendering errors of a
    /// record that cannot be read back.
    pub async fn search_for_person(
        &self,
        filter: &AuditSearchFilter,
        holds: &Holds,
        now: Timestamp,
    ) -> Result<(i64, Vec<serde_json::Value>), AuditError> {
        let Some(resolved) = resolve_criteria(&self.pool, filter).await? else {
            return Ok((0, Vec::new()));
        };
        let (sql, values) =
            newest_first(search_condition(filter, &resolved)).build_sqlx(PostgresQueryBuilder);
        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .persistent(false)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        let skip = usize::try_from(Ord::max(filter.offset, 0)).unwrap_or(usize::MAX);
        let take = usize::try_from(filter.count.clamp(1, 1000)).unwrap_or(50);
        let mut shown = 0_usize;
        let mut documents = Vec::new();
        for row in &rows {
            let (digest, stored) = stored_record(row)?;
            let policy = self.policy(stored.chain_seq, &digest).await?;
            let (event, subject) = classified(stored, &policy)?;
            if !shows(holds, &event, &policy.context, subject.as_deref(), now) {
                continue;
            }
            if shown >= skip && documents.len() < take {
                documents.push(to_fhir(&event, &policy.context, subject.as_deref())?);
            }
            shown = shown.saturating_add(1);
        }
        Ok((i64::try_from(shown).unwrap_or(i64::MAX), documents))
    }
}

/// Whether the person sees the record of `event` at `now`.
fn shows(
    holds: &Holds,
    event: &AuditEvent,
    ctx: &AuditContext,
    subject: Option<&str>,
    now: Timestamp,
) -> bool {
    match access_record(event, ctx, subject) {
        Some(record) => holds.shows(Reader::Person, &record, now),
        // NOTE: EHDS Art. 3(3), a record outside the access record (a hold's own mark) names
        // only its object, which the register holds by the same key.
        None => event
            .object_id
            .as_deref()
            .is_none_or(|object| !holds.withholds(object, now)),
    }
}
