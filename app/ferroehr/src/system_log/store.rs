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

use crate::db::acquire::acquire;
use crate::service::error::ErrorChain;
use crate::system_log::AuditError;
use crate::system_log::codes::AtnaAction;
use crate::system_log::event::{ActingMode, AuditEvent, EventOutcome, EventType, ObjectClass};
use ferrohealth_smart::assurance::AssuranceLevel;

/// The single-record insert, returning the stored row id.
const INSERT_ONE: &str = "INSERT INTO audit.audit_event (recorded_at, action, outcome, \
     event_code, operation, principal, organisation, patient_id, resource_class, resource_id, \
     client_ip, token_id, domain, purpose, legal_basis, result_count, request_id, fhir, roles, \
     origins, origin_count, categories, category_basis, category_evidence, category_map_digest, \
     acting_mode, assurance_level, assurance_value, professional_id, emergency_access) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, \
     $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30) \
     RETURNING id";

/// The batched insert: one row per element of the parallel column arrays.
const INSERT_BATCH: &str = "INSERT INTO audit.audit_event (recorded_at, action, outcome, \
     event_code, operation, principal, organisation, patient_id, resource_class, resource_id, \
     client_ip, token_id, domain, purpose, legal_basis, result_count, request_id, fhir, roles, \
     origins, origin_count, categories, category_basis, category_evidence, category_map_digest, \
     acting_mode, assurance_level, assurance_value, professional_id, emergency_access) \
     SELECT * FROM UNNEST($1::timestamptz[], $2::text[], $3::smallint[], $4::text[], \
     $5::text[], $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], $11::text[], \
     $12::text[], $13::text[], $14::text[], $15::text[], $16::bigint[], $17::text[], \
     $18::jsonb[], $19::jsonb[], $20::jsonb[], $21::bigint[], $22::jsonb[], $23::text[], \
     $24::jsonb[], $25::text[], $26::text[], $27::text[], $28::text[], $29::text[], \
     $30::boolean[])";

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
        sqlx::query(INSERT_ONE)
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
            .bind(categories_json(event))
            .bind(category_basis(event))
            .bind(category_evidence_json(event))
            .bind(category_map_digest(event))
            .bind(event.actor.mode.map(ActingMode::as_str))
            .bind(event.actor.assurance_level.map(AssuranceLevel::as_str))
            .bind(event.actor.assurance_value.as_deref())
            .bind(event.actor.professional_id.as_deref())
            .bind(event.emergency_access)
            .fetch_one(&mut *acquire(&self.pool).await?)
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
        let mut classified = CategoryColumns::with_capacity(records.len());
        let mut actors = ActorColumns::with_capacity(records.len());
        let mut emergency: Vec<bool> = Vec::with_capacity(records.len());
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
            classified.push(event);
            actors.push(event);
            emergency.push(event.emergency_access);
        }
        if fhir_docs.is_empty() {
            return Ok(());
        }
        sqlx::query(INSERT_BATCH)
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
            .bind(classified.categories)
            .bind(classified.bases)
            .bind(classified.evidence)
            .bind(classified.digests)
            .bind(actors.modes)
            .bind(actors.levels)
            .bind(actors.values)
            .bind(actors.professionals)
            .bind(emergency)
            .execute(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(())
    }

    /// Stamp a row as delivered by the syslog sink. Delivery stamps are
    /// best-effort bookkeeping: a failure is logged, never propagated (the
    /// record itself is already durable).
    pub async fn mark_syslog_delivered(&self, id: Uuid) {
        let outcome = self
            .stamp(
                "UPDATE audit.audit_event SET delivered_syslog_at = now() WHERE id = $1",
                id,
            )
            .await;
        if let Err(e) = outcome {
            tracing::warn!(
                "audit store: stamping delivered_syslog_at failed: {}",
                ErrorChain::new(&e)
            );
        }
    }

    /// Stamp a row as delivered by the FHIR feed sink (see
    /// [`Self::mark_syslog_delivered`] for the best-effort semantics).
    pub async fn mark_fhir_feed_delivered(&self, id: Uuid) {
        let outcome = self
            .stamp(
                "UPDATE audit.audit_event SET delivered_fhir_feed_at = now() WHERE id = $1",
                id,
            )
            .await;
        if let Err(e) = outcome {
            tracing::warn!(
                "audit store: stamping delivered_fhir_feed_at failed: {}",
                ErrorChain::new(&e)
            );
        }
    }

    /// Runs one delivery-stamp `UPDATE` binding the row id.
    async fn stamp(&self, sql: &'static str, id: Uuid) -> Result<(), AuditError> {
        sqlx::query(sql)
            .bind(id)
            .execute(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(())
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
        .fetch_all(&mut *acquire(&self.pool).await?)
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
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    /// Delete records older than the same calendar date `retention_years`
    /// years ago (`[audit.store] retention_years`; 0 = keep forever). Returns
    /// the number of reaped rows.
    ///
    /// The calendar arithmetic and the chain tombstones are
    /// `audit.reap_audit_events_years`, the year-keyed twin of the day-keyed
    /// path [`Self::reap`] takes; both reap through one body.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the reap fails.
    pub async fn reap_years(&self, retention_years: u32) -> Result<u64, AuditError> {
        if retention_years == 0 {
            return Ok(0);
        }
        let removed: i64 = sqlx::query_scalar("SELECT audit.reap_audit_events_years($1)")
            .bind(i32::try_from(retention_years).unwrap_or(i32::MAX))
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        u64::try_from(removed).map_err(|e| AuditError::Store(sqlx::Error::Decode(Box::new(e))))
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
        .fetch_all(&mut *acquire(&self.pool).await?)
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
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;

        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .fetch_all(&mut *acquire(&self.pool).await?)
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
    if let Some(until) = filter.until {
        let bound = Expr::val(until.to_string()).cast_as("timestamptz");
        cond = cond.add(if filter.until_inclusive {
            Expr::col(Alias::new("recorded_at")).lte(bound)
        } else {
            Expr::col(Alias::new("recorded_at")).lt(bound)
        });
    }
    for criterion in &filter.criteria {
        cond = cond.add(criterion_condition(criterion));
    }
    cond
}

/// The condition of one criterion: any of its values matches.
fn criterion_condition(criterion: &AuditCriterion) -> sea_query::Cond {
    use sea_query::{Alias, Expr, ExprTrait};

    let column_in = |column: &'static str, values: &[String]| {
        values.iter().fold(sea_query::Cond::any(), |any, value| {
            any.add(Expr::col(Alias::new(column)).eq(value.clone()))
        })
    };
    match criterion {
        AuditCriterion::Patient(values) => column_in("patient_id", values),
        AuditCriterion::Agent(values) => column_in("principal", values),
        AuditCriterion::Entity(values) => column_in("resource_id", values),
        AuditCriterion::Action(values) => column_in("action", values),
        AuditCriterion::Outcome(values) => {
            values.iter().fold(sea_query::Cond::any(), |any, value| {
                any.add(Expr::col(Alias::new("outcome")).eq(*value))
            })
        }
        // NOTE: `RESTful` ATNA §3.81.4.1.2.2, `address` matches
        // `agent.network.address`; every agent of the stored document is read.
        AuditCriterion::Address(values) => {
            values.iter().fold(sea_query::Cond::any(), |any, value| {
                any.add(Expr::cust_with_values(
                    "EXISTS (SELECT 1 FROM jsonb_path_query(fhir, CAST($1 AS jsonpath)) \
                     AS address(value) WHERE strpos(lower(address.value #>> '{}'), lower($2)) > 0)",
                    ["$.agent[*].network.address".to_owned(), value.clone()],
                ))
            })
        }
        AuditCriterion::Element(element, tokens) => {
            tokens.iter().fold(sea_query::Cond::any(), |any, token| {
                let (path, vars) = token_path(*element, token);
                any.add(Expr::cust_with_values(
                    "jsonb_path_exists(fhir, CAST($1 AS jsonpath), CAST($2 AS jsonb))",
                    [path, vars.to_string()],
                ))
            })
        }
    }
}

/// The SQL/JSON path that finds `token` at `element`, and its variables: the
/// three token forms of FHIR R4 Search §3.1.1.4.10, with the values bound as
/// `$code` and `$system` rather than written into the path.
fn token_path(element: AuditElement, token: &AuditToken) -> (String, serde_json::Value) {
    let (path, identifier) = element.path();
    let by_code = if identifier {
        " ? (@.value == $code"
    } else {
        " ? (@.code == $code"
    };
    let filter: [&str; 2] = match (&token.system, token.code.is_some()) {
        (AuditTokenSystem::Any, _) => [by_code, ")"],
        (AuditTokenSystem::Absent, _) => [by_code, " && !exists(@.system))"],
        (AuditTokenSystem::Named(_), true) => [by_code, " && @.system == $system)"],
        (AuditTokenSystem::Named(_), false) => [" ? (@.system == $system", ")"],
    };
    let system = match &token.system {
        AuditTokenSystem::Named(system) => serde_json::Value::String(system.clone()),
        AuditTokenSystem::Any | AuditTokenSystem::Absent => serde_json::Value::Null,
    };
    let code = token
        .code
        .clone()
        .map_or(serde_json::Value::Null, serde_json::Value::String);
    (
        [path, filter[0], filter[1]].concat(),
        serde_json::json!({ "code": code, "system": system }),
    )
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

/// The ITI-81 retrieval the REST layer resolved: the `date` window, the
/// criteria (every one must match, any of a criterion's values matches) and
/// the page (`RESTful` ATNA §3.81.4.1.2.2).
#[derive(Debug, Clone, Default)]
pub struct AuditSearchFilter {
    /// The first instant a record's `recorded` may carry (inclusive), from
    /// the `date` parameters.
    pub from: Option<jiff::Timestamp>,
    /// The first instant past the records served, from the `date`
    /// parameters: exclusive, or inclusive when [`Self::until_inclusive`].
    pub until: Option<jiff::Timestamp>,
    /// Whether [`Self::until`] is inclusive: an end the query names below a
    /// microsecond, rounded up to the microsecond `recorded_at` keeps.
    pub until_inclusive: bool,
    /// The criteria, each of which a record must match.
    pub criteria: Vec<AuditCriterion>,
    /// `_count` — page size (default 50, capped at 1000).
    pub count: i64,
    /// `_offset` — page offset.
    pub offset: i64,
}

/// One search parameter of the retrieval: a record matches when any of its
/// values does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditCriterion {
    /// `patient` — the recorded patient (EHR subject) id.
    Patient(Vec<String>),
    /// `agent` — the authenticated principal.
    Agent(Vec<String>),
    /// `entity` — the touched resource id.
    Entity(Vec<String>),
    /// `outcome` — the DICOM outcome indicator (0/4/8/12).
    Outcome(Vec<i16>),
    /// `action` — the DICOM action code (C/R/U/D/E).
    Action(Vec<String>),
    /// `address` — a substring of an `agent.network.address`, compared
    /// without regard to case.
    Address(Vec<String>),
    /// A token parameter matched against an element of the stored
    /// `AuditEvent`.
    Element(AuditElement, Vec<AuditToken>),
}

/// The `AuditEvent` element a token parameter is matched against
/// (`RESTful` ATNA §3.81.4.1.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditElement {
    /// `agent.identifier`: `agent.who.identifier`.
    AgentIdentifier,
    /// `patient.identifier`: the `entity.what.identifier` of the entity whose
    /// role is Patient.
    PatientIdentifier,
    /// `entity.identifier`: `entity.what.identifier`.
    EntityIdentifier,
    /// `source.identifier`: `source.observer.identifier`.
    SourceIdentifier,
    /// `entity-type`: `entity.type`.
    EntityType,
    /// `entity-role`: `entity.role`.
    EntityRole,
    /// `type`: `type`.
    Type,
    /// `subtype`: `subtype`.
    Subtype,
}

impl AuditElement {
    /// The SQL/JSON path of the element, and whether it is an `Identifier`
    /// (its code in `value`) rather than a `Coding` (its code in `code`).
    const fn path(self) -> (&'static str, bool) {
        match self {
            Self::AgentIdentifier => ("$.agent[*].who.identifier", true),
            Self::PatientIdentifier => (
                "$.entity[*] ? (@.role.code == \"1\" && @.role.system == \"http://terminology.hl7.org/CodeSystem/object-role\").what.identifier",
                true,
            ),
            Self::EntityIdentifier => ("$.entity[*].what.identifier", true),
            Self::SourceIdentifier => ("$.source.observer.identifier", true),
            Self::EntityType => ("$.entity[*].type", false),
            Self::EntityRole => ("$.entity[*].role", false),
            Self::Type => ("$.type", false),
            Self::Subtype => ("$.subtype[*]", false),
        }
    }
}

/// A token value (FHIR R4 Search §3.1.1.4.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditToken {
    /// Which system the value names.
    pub system: AuditTokenSystem,
    /// The code or identifier value; `None` for `[system]|`.
    pub code: Option<String>,
}

/// The system part of an [`AuditToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditTokenSystem {
    /// `[code]`: any system, or none.
    Any,
    /// `|[code]`: no system.
    Absent,
    /// `[system]|[code]` or `[system]|`.
    Named(String),
}

fn action_str(event: &AuditEvent) -> String {
    event.action.as_code().to_owned()
}

fn outcome_smallint(event: &AuditEvent) -> i16 {
    match event.outcome {
        EventOutcome::Success => 0,
        EventOutcome::MinorFailure => 4,
        EventOutcome::SeriousFailure => 8,
    }
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

/// The four authentication columns of a batched insert, one entry per record.
struct ActorColumns<'a> {
    modes: Vec<Option<&'static str>>,
    levels: Vec<Option<&'static str>>,
    values: Vec<Option<&'a str>>,
    professionals: Vec<Option<&'a str>>,
}

impl<'a> ActorColumns<'a> {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            modes: Vec::with_capacity(capacity),
            levels: Vec::with_capacity(capacity),
            values: Vec::with_capacity(capacity),
            professionals: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, event: &'a AuditEvent) {
        self.modes.push(event.actor.mode.map(ActingMode::as_str));
        self.levels
            .push(event.actor.assurance_level.map(AssuranceLevel::as_str));
        self.values.push(event.actor.assurance_value.as_deref());
        self.professionals
            .push(event.actor.professional_id.as_deref());
    }
}

/// The four classification columns of a batched insert, one entry per record.
struct CategoryColumns {
    categories: Vec<Option<serde_json::Value>>,
    bases: Vec<Option<&'static str>>,
    evidence: Vec<Option<serde_json::Value>>,
    digests: Vec<Option<String>>,
}

impl CategoryColumns {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            categories: Vec::with_capacity(capacity),
            bases: Vec::with_capacity(capacity),
            evidence: Vec::with_capacity(capacity),
            digests: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, event: &AuditEvent) {
        self.categories.push(categories_json(event));
        self.bases.push(category_basis(event));
        self.evidence.push(category_evidence_json(event));
        self.digests.push(category_map_digest(event));
    }
}

/// The `categories` column value: the stored category words as a JSON array,
/// or `NULL` when the operation touched no clinical content.
fn categories_json(event: &AuditEvent) -> Option<serde_json::Value> {
    event
        .category
        .as_ref()
        .map(|record| serde_json::json!(record.stored_categories()))
}

/// The `category_basis` column value: what the classification was read from.
fn category_basis(event: &AuditEvent) -> Option<&'static str> {
    event
        .category
        .as_ref()
        .map(|record| record.read_from.code())
}

/// The `category_evidence` column value: the identifiers the classification
/// read, as a JSON array.
fn category_evidence_json(event: &AuditEvent) -> Option<serde_json::Value> {
    event
        .category
        .as_ref()
        .map(|record| serde_json::json!(record.stored_evidence()))
}

/// The `category_map_digest` column value.
fn category_map_digest(event: &AuditEvent) -> Option<String> {
    event.category_map_digest.map(|digest| digest.to_string())
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
    use super::build_search_statements;
    use super::{AuditCriterion, AuditElement, AuditSearchFilter, AuditToken, AuditTokenSystem};

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
            criteria: vec![
                AuditCriterion::Patient(vec![value.to_owned()]),
                AuditCriterion::Agent(vec![value.to_owned()]),
                AuditCriterion::Entity(vec![value.to_owned()]),
                AuditCriterion::Action(vec![value.to_owned()]),
                AuditCriterion::Address(vec![value.to_owned()]),
                AuditCriterion::Element(
                    AuditElement::AgentIdentifier,
                    vec![AuditToken {
                        system: AuditTokenSystem::Named(value.to_owned()),
                        code: Some(value.to_owned()),
                    }],
                ),
            ],
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
            criteria: vec![
                AuditCriterion::Patient(vec!["p".to_owned()]),
                AuditCriterion::Agent(vec!["a".to_owned()]),
                AuditCriterion::Entity(vec!["e".to_owned()]),
                AuditCriterion::Action(vec!["x".to_owned()]),
                AuditCriterion::Outcome(vec![0]),
            ],
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
