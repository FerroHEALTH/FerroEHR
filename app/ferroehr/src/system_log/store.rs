// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local IHE ATNA **Audit Record Repository**: the `audit.audit_event`
//! table writer + retention reaper.
//!
//! NOTE: no openEHR spec governs audit storage — our own design/extension.
//! openEHR endorses in-system access logging and rules it out of the EHR
//! content ("read accesses by application users to EHR data should be logged
//! in the EHR system … currently openEHR does not support \[logs as part of
//! the EHR proper\]" — BASE `architecture_overview/master07-security.adoc`
//! §Access logging), so the store lives in its own `audit` schema, strictly
//! outside the EHR data (`migrations/audit/0001_schema_and_roles.sql`).
//!
//! The canonical stored form is the FHIR R4 `AuditEvent` (IHE BALP shape,
//! rendered by the `fhir` cargo feature) in the `fhir` jsonb column, the exact
//! document the RESTful-ATNA ITI-81 search serves; the promoted columns are
//! derived search keys. Rows are append-only except the per-sink delivery stamps
//! and retention reaping.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store carries an \
              already-rendered FHIR document, never a typed resource"
)]

use jiff_sqlx::Timestamp;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::system_log::AuditError;
use crate::system_log::codes::AtnaAction;
use crate::system_log::event::{AuditEvent, EventType, ObjectClass};

/// The PG-backed Audit Record Repository (the `store` sink).
#[derive(Debug, Clone)]
pub struct AuditStore {
    pool: PgPool,
}

impl AuditStore {
    /// A store over the given pool. The pool should be the plain
    /// pool: the audit trail is the node's operator surface, written by the
    /// drain task rather than by a request.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        AuditStore { pool }
    }

    /// Persist one audit record: the rendered FHIR `AuditEvent` as the
    /// canonical payload plus the promoted search columns derived from the
    /// resolved event. Returns the stored row id (for the per-sink delivery
    /// stamps).
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails.
    pub async fn insert(
        &self,
        event: &AuditEvent,
        subject: Option<&str>,
        fhir: &serde_json::Value,
    ) -> Result<Uuid, AuditError> {
        let outcome = outcome_smallint(event);
        sqlx::query(
            "INSERT INTO audit.audit_event (recorded_at, action, outcome, event_code, \
             operation, principal, organisation, patient_id, resource_class, resource_id, \
             client_ip, token_id, domain, purpose, legal_basis, result_count, \
             request_id, fhir, roles, origins, origin_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
             $15, $16, $17, $18, $19, $20, $21) \
             RETURNING id",
        )
        .bind(Timestamp::from(event.timestamp))
        .bind(action_str(event))
        .bind(outcome)
        .bind(event_code(event))
        .bind(operation(event))
        .bind(nonempty_opt(&event.user_id))
        .bind(event.organisation.as_deref())
        .bind(subject)
        .bind(resource_class(event.object))
        .bind(event.object_id.as_deref())
        .bind(event.client_ip.as_deref())
        .bind(event.token_id.as_deref())
        .bind(event.domain.as_str())
        .bind(event.purpose.as_deref())
        .bind(event.legal_basis.as_deref())
        .bind(result_count(event))
        .bind(event.request_id.as_deref())
        .bind(fhir.clone())
        .bind(roles_json(event))
        .bind(origins_json(event))
        .bind(origin_count(event))
        .fetch_one(&self.pool)
        .await?
        .try_get::<Uuid, _>("id")
        .map_err(AuditError::Store)
    }

    /// Persist a whole drained batch in ONE multi-row `INSERT` (UNNEST over
    /// parallel column arrays) — the throughput path for the default
    /// store-only posture, where per-event round trips cannot keep up with a
    /// loaded write path. No row ids are returned: the batch path is used
    /// only when no per-row delivery stamping is needed (the syslog sink
    /// keeps the per-event [`Self::insert`]).
    ///
    /// A record whose FHIR rendering failed carries no document and is
    /// skipped here — the drop is already metered where the rendering failed.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails (the whole batch fails
    /// together; the caller retries).
    pub async fn insert_batch(
        &self,
        records: &[(AuditEvent, Option<String>, Option<serde_json::Value>)],
    ) -> Result<(), AuditError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut recorded_at: Vec<Timestamp> = Vec::with_capacity(records.len());
        let mut actions: Vec<String> = Vec::with_capacity(records.len());
        let mut outcomes: Vec<i16> = Vec::with_capacity(records.len());
        let mut event_codes: Vec<&str> = Vec::with_capacity(records.len());
        let mut operations: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut principals: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut organisations: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut patient_ids: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut resource_classes: Vec<&str> = Vec::with_capacity(records.len());
        let mut resource_ids: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut client_ips: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut token_ids: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut domains: Vec<&str> = Vec::with_capacity(records.len());
        let mut purposes: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut legal_bases: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut result_counts: Vec<Option<i64>> = Vec::with_capacity(records.len());
        let mut request_ids: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut fhir_docs: Vec<serde_json::Value> = Vec::with_capacity(records.len());
        let mut roles: Vec<Option<serde_json::Value>> = Vec::with_capacity(records.len());
        let mut origins: Vec<Option<serde_json::Value>> = Vec::with_capacity(records.len());
        let mut origin_counts: Vec<Option<i64>> = Vec::with_capacity(records.len());
        for (event, subject, fhir) in records {
            let Some(fhir) = fhir else {
                continue;
            };
            recorded_at.push(Timestamp::from(event.timestamp));
            actions.push(action_str(event));
            outcomes.push(outcome_smallint(event));
            event_codes.push(event_code(event));
            operations.push(operation(event));
            principals.push(nonempty_opt(&event.user_id));
            organisations.push(event.organisation.as_deref());
            patient_ids.push(subject.as_deref());
            resource_classes.push(resource_class(event.object));
            resource_ids.push(event.object_id.as_deref());
            client_ips.push(event.client_ip.as_deref());
            token_ids.push(event.token_id.as_deref());
            domains.push(event.domain.as_str());
            purposes.push(event.purpose.as_deref());
            legal_bases.push(event.legal_basis.as_deref());
            result_counts.push(result_count(event));
            request_ids.push(event.request_id.as_deref());
            fhir_docs.push(fhir.clone());
            roles.push(roles_json(event));
            origins.push(origins_json(event));
            origin_counts.push(origin_count(event));
        }
        if fhir_docs.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO audit.audit_event (recorded_at, action, outcome, event_code, \
             operation, principal, organisation, patient_id, resource_class, resource_id, \
             client_ip, token_id, domain, purpose, legal_basis, result_count, \
             request_id, fhir, roles, origins, origin_count) \
             SELECT * FROM UNNEST($1::timestamptz[], $2::text[], $3::smallint[], $4::text[], \
             $5::text[], $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], \
             $11::text[], $12::text[], $13::text[], $14::text[], $15::text[], \
             $16::bigint[], $17::text[], $18::jsonb[], $19::jsonb[], $20::jsonb[], \
             $21::bigint[])",
        )
        .bind(recorded_at)
        .bind(actions)
        .bind(outcomes)
        .bind(event_codes)
        .bind(operations)
        .bind(principals)
        .bind(organisations)
        .bind(patient_ids)
        .bind(resource_classes)
        .bind(resource_ids)
        .bind(client_ips)
        .bind(token_ids)
        .bind(domains)
        .bind(purposes)
        .bind(legal_bases)
        .bind(result_counts)
        .bind(request_ids)
        .bind(fhir_docs)
        .bind(roles)
        .bind(origins)
        .bind(origin_counts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stamp a row as delivered by the syslog sink. Delivery stamps are
    /// best-effort bookkeeping: a failure is logged, never propagated (the
    /// record itself is already durable).
    pub async fn mark_syslog_delivered(&self, id: Uuid) {
        let outcome =
            sqlx::query("UPDATE audit.audit_event SET delivered_syslog_at = now() WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await;
        if let Err(e) = outcome {
            tracing::warn!("audit store: stamping delivered_syslog_at failed: {e}");
        }
    }

    /// Stamp a row as delivered by the FHIR feed sink (see
    /// [`Self::mark_syslog_delivered`] for the best-effort semantics).
    pub async fn mark_fhir_feed_delivered(&self, id: Uuid) {
        let outcome = sqlx::query(
            "UPDATE audit.audit_event SET delivered_fhir_feed_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await;
        if let Err(e) = outcome {
            tracing::warn!("audit store: stamping delivered_fhir_feed_at failed: {e}");
        }
    }

    /// The oldest rows not yet delivered by the FHIR feed sink (the ITI-20
    /// ATX:FHIR Feed outbox): `(id, fhir document)`, oldest first.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the SELECT fails.
    pub async fn pending_fhir_feed(
        &self,
        limit: i64,
    ) -> Result<Vec<(Uuid, serde_json::Value)>, AuditError> {
        let rows = sqlx::query(
            "SELECT id, fhir FROM audit.audit_event \
             WHERE delivered_fhir_feed_at IS NULL \
             ORDER BY stored_at ASC LIMIT $1",
        )
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<Uuid, _>("id")?,
                    row.try_get::<serde_json::Value, _>("fhir")?,
                ))
            })
            .collect()
    }

    /// Delete records older than `retention_days` (0 = keep forever).
    /// Returns the number of reaped rows.
    ///
    /// Reaping goes through `audit.reap_audit_events`, the only deletion path
    /// the table permits: it removes a PREFIX of the hash chain and advances
    /// the retention low-water mark in the same transaction, so the surviving
    /// records still verify and an unrecorded deletion of the oldest records
    /// remains detectable. A direct `DELETE` is refused by the table's trigger.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the reap fails.
    pub async fn reap(&self, retention_days: u32) -> Result<u64, AuditError> {
        if retention_days == 0 {
            return Ok(0);
        }
        let removed: i64 = sqlx::query_scalar("SELECT audit.reap_audit_events($1)")
            .bind(i32::try_from(retention_days).unwrap_or(i32::MAX))
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    /// Check the repository's tamper evidence: recompute every record's digest,
    /// re-walk every link in the hash chain, and check both ends against the
    /// recorded chain state.
    ///
    /// An empty result means the trail is intact. Any returned
    /// [`AuditChainFinding`] names one damaged record or chain boundary and what
    /// is wrong with it, the same answer
    /// `SELECT * FROM audit.verify_audit_chain()` gives from `psql`.
    ///
    /// NOTE: no openEHR spec governs audit tamper detection — our own
    /// design/extension; the chain is unkeyed, so it detects modification and
    /// deletion but cannot stop a party with unrestricted write access from
    /// recomputing it wholesale.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the verification query cannot run — which is
    /// itself a finding, never a pass.
    pub async fn verify_chain(&self) -> Result<Vec<AuditChainFinding>, AuditError> {
        let rows = sqlx::query(
            "SELECT chain_seq, record_id, recorded_at, finding \
             FROM audit.verify_audit_chain() ORDER BY chain_seq NULLS FIRST",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AuditChainFinding {
                    chain_seq: row.try_get("chain_seq")?,
                    record_id: row.try_get("record_id")?,
                    recorded_at: row
                        .try_get::<Option<Timestamp>, _>("recorded_at")?
                        .map(Timestamp::to_jiff),
                    finding: row.try_get("finding")?,
                })
            })
            .collect()
    }

    /// The RESTful-ATNA ITI-81 retrieval: filtered, newest-first stored FHIR
    /// `AuditEvent` documents plus the total match count. The filter is the
    /// supported ITI-81 search-parameter subset ([`AuditSearchFilter`]).
    ///
    /// # Errors
    /// [`AuditError::Store`] when either query fails.
    pub async fn search(
        &self,
        filter: &AuditSearchFilter,
    ) -> Result<(i64, Vec<serde_json::Value>), AuditError> {
        let (count_sql, count_values, sql, values) = build_search_statements(filter);
        let total: i64 = sqlx::query_scalar_with(sqlx::AssertSqlSafe(count_sql), count_values)
            .fetch_one(&self.pool)
            .await?;

        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .fetch_all(&self.pool)
            .await?;
        let documents = rows
            .into_iter()
            .map(|row| {
                row.try_get::<serde_json::Value, _>("fhir")
                    .map_err(AuditError::Store)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((total, documents))
    }
}

/// The `WHERE` condition for the ITI-81 filter.
///
/// Every column name is a literal here; every caller-supplied value goes through
/// `Expr::val` and therefore binds as a parameter rather than entering the SQL
/// text. That split is the property `sql_injection`-style tests pin.
fn search_condition(filter: &AuditSearchFilter) -> sea_query::Cond {
    use sea_query::{Alias, Expr, ExprTrait};

    let mut cond = sea_query::Cond::all();
    // Timestamps bind as RFC 3339 text cast to timestamptz: the sea-query
    // binder's with-jiff is unimplemented upstream (see the workspace manifest
    // note), so the value crosses as text.
    if let Some(from) = filter.from {
        cond = cond.add(
            Expr::col(Alias::new("recorded_at"))
                .gte(Expr::val(from.to_string()).cast_as("timestamptz")),
        );
    }
    if let Some(to) = filter.to {
        cond = cond.add(
            Expr::col(Alias::new("recorded_at"))
                .lte(Expr::val(to.to_string()).cast_as("timestamptz")),
        );
    }
    if let Some(patient) = &filter.patient {
        cond = cond.add(Expr::col(Alias::new("patient_id")).eq(patient.clone()));
    }
    if let Some(agent) = &filter.agent {
        cond = cond.add(Expr::col(Alias::new("principal")).eq(agent.clone()));
    }
    if let Some(entity) = &filter.entity {
        cond = cond.add(Expr::col(Alias::new("resource_id")).eq(entity.clone()));
    }
    if let Some(outcome) = filter.outcome {
        cond = cond.add(Expr::col(Alias::new("outcome")).eq(outcome));
    }
    if let Some(action) = &filter.action {
        cond = cond.add(Expr::col(Alias::new("action")).eq(action.clone()));
    }
    cond
}

/// Build the ITI-81 retrieval's two statements — the count and the page — from a
/// filter, with no database involved.
///
/// Extracted from [`AuditStore::search`] so the SQL is reachable by a test. This
/// is the server's second runtime dynamic-SQL site after the AQL engine, and the
/// property that matters is the same one: every IDENTIFIER here — the schema, the
/// table, every column, the sort direction — is a literal in this function, while
/// every caller-supplied value goes through `Expr::val` and arrives as a bound
/// parameter. `limit`/`offset` are clamped integers, never text.
///
/// The guard `scripts/checks/sql-string-building.sh` catches string-built SQL but
/// cannot see an `Alias::new` argument, which is why the closed set is pinned by a
/// test rather than only by the guard.
fn build_search_statements(
    filter: &AuditSearchFilter,
) -> (
    String,
    sea_query_sqlx::SqlxValues,
    String,
    sea_query_sqlx::SqlxValues,
) {
    use sea_query::{Alias, Expr, ExprTrait, Order, PostgresQueryBuilder, Query};
    use sea_query_sqlx::SqlxBinder as _;

    let table = (Alias::new("audit"), Alias::new("audit_event"));
    let condition = search_condition(filter);

    let (count_sql, count_values) = Query::select()
        .expr(Expr::col(sea_query::Asterisk).count())
        .from(table.clone())
        .cond_where(condition.clone())
        .build_sqlx(PostgresQueryBuilder);

    let (sql, values) = Query::select()
        .column(Alias::new("fhir"))
        .from(table)
        .cond_where(condition)
        .order_by(Alias::new("recorded_at"), Order::Desc)
        .order_by(Alias::new("stored_at"), Order::Desc)
        .limit(u64::try_from(filter.count.clamp(1, 1000)).unwrap_or(50))
        .offset(u64::try_from(Ord::max(filter.offset, 0)).unwrap_or(0))
        .build_sqlx(PostgresQueryBuilder);

    (count_sql, count_values, sql, values)
}

/// One damaged record — or one damaged chain boundary — reported by
/// [`AuditStore::verify_chain`].
#[derive(Debug, Clone)]
pub struct AuditChainFinding {
    /// The chain position the finding is about; `None` when the chain state
    /// row itself is missing.
    pub chain_seq: Option<i64>,
    /// The damaged record's id, when the finding is about a record that is
    /// still present rather than about a gap or a boundary.
    pub record_id: Option<Uuid>,
    /// The damaged record's event time, when known.
    pub recorded_at: Option<jiff::Timestamp>,
    /// What is wrong, in the operator's words.
    pub finding: String,
}

/// The supported ITI-81 search-parameter subset, resolved by the REST layer.
#[derive(Debug, Clone, Default)]
pub struct AuditSearchFilter {
    /// `date=ge…` — events at/after this instant.
    pub from: Option<jiff::Timestamp>,
    /// `date=le…` — events at/before this instant.
    pub to: Option<jiff::Timestamp>,
    /// `patient` — the recorded patient (EHR subject) id.
    pub patient: Option<String>,
    /// `agent` — the authenticated principal.
    pub agent: Option<String>,
    /// `entity` — the touched resource id.
    pub entity: Option<String>,
    /// `outcome` — the DICOM outcome indicator (0/4/8/12).
    pub outcome: Option<i16>,
    /// `action` — the DICOM action code (C/R/U/D/E).
    pub action: Option<String>,
    /// `_count` — page size (default 50, capped at 1000).
    pub count: i64,
    /// `_offset` — page offset.
    pub offset: i64,
}

fn action_str(event: &AuditEvent) -> String {
    event.action.as_char().to_string()
}

fn outcome_smallint(event: &AuditEvent) -> i16 {
    use crate::system_log::codes::AtnaOutcome;
    // The DICOM indicator values are 0/4/8/12 — always in smallint range.
    i16::try_from(event.outcome.as_i32()).unwrap_or(i16::MAX)
}

fn event_code(event: &AuditEvent) -> &'static str {
    use crate::system_log::codes::AtnaObject;
    event.object.event_id(event.action).0
}

/// The `operation` column: the ITS-REST operation id, or the DCM
/// login/logout `EventTypeCode` csd-code for authentication records.
fn operation(event: &AuditEvent) -> Option<&'static str> {
    match event.event_type {
        Some(EventType::RestOperation(op)) => Some(op),
        Some(EventType::Login) => Some("110122"),
        Some(EventType::Logout) => Some("110123"),
        None => None,
    }
}

const fn resource_class(object: ObjectClass) -> &'static str {
    match object {
        ObjectClass::Ehr => "ehr",
        ObjectClass::Composition => "composition",
        ObjectClass::Contribution => "contribution",
        ObjectClass::Directory => "directory",
        ObjectClass::Query => "query",
        ObjectClass::Template => "template",
        ObjectClass::Demographic => "demographic",
        ObjectClass::Extract => "extract",
        ObjectClass::ApplicationActivity => "application_activity",
        ObjectClass::Authentication => "authentication",
    }
}

fn nonempty_opt(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// The `result_count` column: how many records the operation served.
///
/// A count beyond `i64::MAX` is not representable in the column's `bigint`, and
/// no result set reaches it, so such a value is recorded as absent rather than
/// as a wrong number.
fn result_count(event: &AuditEvent) -> Option<i64> {
    event
        .result_count
        .and_then(|count| i64::try_from(count).ok())
}

/// The `origins` column value: the capped origin set as a JSON array, or
/// `NULL` when the operation served no version body.
fn origins_json(event: &AuditEvent) -> Option<serde_json::Value> {
    (!event.origins.is_empty()).then(|| serde_json::json!(event.origins))
}

/// The `origin_count` column value.
fn origin_count(event: &AuditEvent) -> Option<i64> {
    event
        .origin_count
        .map(|count| i64::try_from(count).unwrap_or(i64::MAX))
}

/// The `roles` column value: the role names as a JSON array, or `NULL` when
/// no principal was authenticated (an empty list on an authenticated caller
/// is a different fact and is written as `[]` by the callers that know it).
fn roles_json(event: &AuditEvent) -> Option<serde_json::Value> {
    if event.roles.is_empty() && event.user_id.is_empty() {
        return None;
    }
    Some(serde_json::Value::Array(
        event
            .roles
            .iter()
            .map(|role| serde_json::Value::String(role.clone()))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::AuditSearchFilter;
    use super::build_search_statements;

    /// Twenty payloads that break SQL when they reach the statement text rather
    /// than a bound parameter.
    const HOSTILE: [&str; 20] = [
        "'",
        "\"",
        "--",
        "/*",
        "*/",
        ";",
        "\\",
        "' OR '1'='1",
        "'; DROP TABLE audit.audit_event; --",
        "') OR 1=1 --",
        "1; SELECT pg_sleep(10)",
        "$1",
        "$$",
        "\0",
        "\n",
        "\r\n",
        "0x27",
        "%27",
        "\u{2019}",
        "' UNION SELECT fhir FROM audit.audit_event --",
    ];

    fn filter_with(value: &str) -> AuditSearchFilter {
        AuditSearchFilter {
            patient: Some(value.to_owned()),
            agent: Some(value.to_owned()),
            entity: Some(value.to_owned()),
            action: Some(value.to_owned()),
            ..AuditSearchFilter::default()
        }
    }

    /// Hostile filter values never change the generated SQL — only the bound
    /// values.
    ///
    /// The assertion is byte-equality of the statement text against a benign
    /// baseline. That is stronger than looking for the payload in the SQL: it
    /// catches a value that alters the statement's SHAPE as well as one that
    /// appears in it.
    #[test]
    fn hostile_filter_values_never_change_the_generated_sql() {
        let (baseline_count, _, baseline_page, _) = build_search_statements(&filter_with("benign"));

        for payload in HOSTILE {
            // The page statement's own values are not inspected: both statements
            // bind from the same condition, so asserting the count's bindings
            // covers both, and the page text is compared byte for byte below.
            let (count_sql, count_values, sql, _page_values) =
                build_search_statements(&filter_with(payload));
            assert_eq!(
                count_sql, baseline_count,
                "the count statement changed for {payload:?}"
            );
            assert_eq!(
                sql, baseline_page,
                "the page statement changed for {payload:?}"
            );
            // The "absent from the SQL" check skips payloads that ARE the
            // placeholder syntax (`$1`, `$$`) or a single metacharacter: `$1`
            // legitimately appears because that is what a bound parameter looks
            // like. The byte-equality above already covers them — a value that
            // altered the statement would have changed it — so this is the
            // narrower check, not a weaker one.
            let is_placeholder_shaped = payload.starts_with('$') || payload.len() < 2;
            assert!(
                is_placeholder_shaped || !sql.contains(payload),
                "the payload must not appear in the SQL text: {payload:?} in {sql}"
            );
            // The values still travel — the point is that they travel as
            // parameters, not that they are dropped. Compared in the DEBUG
            // representation on both sides: the bound values render a newline as
            // an escape, so comparing a raw control character against that text
            // would fail for the wrong reason.
            let bound = format!("{count_values:?}");
            let rendered = format!("{payload:?}");
            let rendered = rendered.trim_matches('"');
            assert!(
                bound.contains(rendered),
                "the payload must be bound as a value: {payload:?} not in {bound}"
            );
        }
    }

    /// The identifiers and the sort direction come from a closed set in this
    /// module — the enumeration, recorded so a new filter cannot quietly add one.
    ///
    /// Written as an exact-set assertion over the quoted identifiers the builder
    /// emits: a filter that introduced a caller-supplied column would appear here
    /// and fail, which is what the guard script cannot see (it reads for
    /// string-built SQL, not for an `Alias::new` argument).
    #[test]
    fn every_identifier_comes_from_the_closed_set() {
        let (count_sql, _, sql, _) = build_search_statements(&AuditSearchFilter {
            patient: Some("p".to_owned()),
            agent: Some("a".to_owned()),
            entity: Some("e".to_owned()),
            action: Some("x".to_owned()),
            outcome: Some(0),
            ..AuditSearchFilter::default()
        });

        let mut seen: Vec<String> = format!("{count_sql} {sql}")
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        seen.sort_unstable();
        seen.dedup();

        let mut expected = vec![
            "action",
            "audit",
            "audit_event",
            "fhir",
            "outcome",
            "patient_id",
            "principal",
            "recorded_at",
            "resource_id",
            "stored_at",
        ];
        expected.sort_unstable();
        assert_eq!(
            seen,
            expected.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            "the identifier set moved: {sql}"
        );
        assert!(sql.contains("DESC"), "newest-first is the ITI-81 order");
        assert!(!sql.contains("ASC"), "no ascending sort is emitted");
    }

    /// `limit`/`offset` are clamped integers, so no caller value reaches the
    /// statement as text — including a negative offset or an absurd count.
    #[test]
    fn paging_is_clamped_and_never_textual() {
        let (_, _, sql, values) = build_search_statements(&AuditSearchFilter {
            count: i64::MAX,
            offset: i64::MIN,
            ..AuditSearchFilter::default()
        });
        // Paging binds as parameters rather than being written into the
        // statement, which is stronger than the clamp alone — so the clamped
        // values are asserted where they actually travel.
        assert!(
            sql.contains("LIMIT $") && sql.contains("OFFSET $"),
            "paging must bind, not interpolate: {sql}"
        );
        let bound = format!("{values:?}");
        assert!(
            bound.contains("1000"),
            "the count clamps to the cap: {bound}"
        );
        assert!(
            !bound.contains(&i64::MIN.to_string()),
            "a negative offset must never reach the database: {bound}"
        );
    }
}
