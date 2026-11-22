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
//! outside the EHR data (`migrations/audit/0001_baseline_audit_event.sql`).
//!
//! A record keeps its facts in columns and names the render policy it was
//! written under ([`crate::system_log::render`]). The FHIR R4 `AuditEvent`
//! (IHE BALP shape, the `fhir` cargo feature) is rendered from them when the
//! record is read by the RESTful-ATNA ITI-81 search or delivered by the FHIR
//! Feed, so the same record always renders the same document. Rows are
//! append-only except the per-sink delivery stamps and retention reaping.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store hands the \
              FHIR Feed an already-rendered FHIR document; the ITI-81 page is typed (#5716)"
)]

#[cfg(feature = "fhir")]
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(feature = "fhir")]
use std::sync::{Mutex, PoisonError};

#[cfg(feature = "fhir")]
use jiff_sqlx::Timestamp;
#[cfg(feature = "fhir")]
use sqlx::Row;
use sqlx::postgres::PgConnection;
#[cfg(feature = "fhir")]
use sqlx::postgres::PgRow;
use sqlx::{Connection, PgPool};
use uuid::Uuid;

mod access_log;
mod batch;
pub mod chain_walk;
pub mod chains;
pub mod delivery;
#[cfg(feature = "fhir")]
mod entries;
pub mod frontier;
pub mod intake;
pub(crate) mod lax;
#[cfg(feature = "fhir")]
pub mod outbox;
#[cfg(feature = "fhir")]
mod review;
#[cfg(feature = "fhir")]
mod search_page;
mod writers;

use crate::access_log::store::AccessLogStore;
use crate::db::acquire::acquire;
use crate::system_log::event::{AuditEvent, EventOutcome, EventType, ObjectClass};
use crate::system_log::render::RenderPolicy;
use crate::system_log::store::frontier::Owed;
use crate::system_log::{AuditError, content_digest};
#[cfg(feature = "fhir")]
use crate::system_log::{
    categories::{ClassifiedContent, MapDigest},
    event::{AccessDomain, ActingMode, EventActionCode},
    fhir::value_of,
    render::{CompiledPolicy, StoredRecord},
};
use batch::BatchColumns;
#[cfg(feature = "fhir")]
use chains::AuditChain;
use lax::{any_equals, any_is, lax_path};
use writers::{Pinned, WritePolicy, Writers};

#[cfg(feature = "fhir")]
use ferrohealth_smart::assurance::AssuranceLevel;

/// The batched write of the system records: each record written once into
/// `audit.audit_event`, one row per element of the parallel column arrays,
/// every row under the one render policy in `$38`, with the content digest
/// its writer computed in `$57` and its write time in `$56`, and held under
/// the reservation in `$54`, when there is one, by a row of
/// `audit.audit_event_hold` (#6203, #6242). The transaction takes its xid in
/// the materialized `own_xid` every row joins, before any row draws its
/// position, which the frontier proof rests on ([`crate::access_log::frontier`]).
/// Each record's search tokens arrive as the text of an `integer[]` in `$39`,
/// since an array of arrays unnests element by element, and its id in `$53`;
/// `$55` is the level each record is recorded for review at.
const INSERT_BATCH: &str = "WITH own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken), \
     written AS (INSERT INTO audit.audit_event (id, recorded_at, action, outcome, \
     event_code, operation, principal, patient_id, resource_class, resource_id, \
     client_ip, token_id, domain, purpose, legal_basis, result_count, request_id, roles, \
     origins, origin_count, categories, category_basis, category_evidence, category_map_digest, \
     acting_mode, assurance_level, assurance_value, professional_id, emergency_access, \
     retention_ends, retention_stated, permit, issuer, client_id, ehr_id, subject_namespace, \
     subject_lookup_failed, category_content, render_policy, search_tokens, binding, guest, \
     party, party_groups, provider, representation, external_fields, agreement, actor_chain, \
     facilities, binding_detail, summary, token_attributes, review, stored_at, content_digest) \
     SELECT u.id, u.c1, u.c2, u.c3, u.c4, u.c5, u.c6, u.c7, u.c8, u.c9, u.c10, u.c11, u.c12, \
     u.c13, u.c14, u.c15, u.c16, u.c17, u.c18, u.c19, u.c20, u.c21, u.c22, u.c23, u.c24, u.c25, \
     u.c26, u.c27, u.c28, u.c29, u.c30, u.c31, u.c32, u.c33, u.c34, u.c35, u.c36, u.c37, \
     $38::bytea, u.tokens::integer[], u.c40, u.c41, u.c42, u.c43, u.c44, u.c45, u.c46, u.c47, \
     u.c48, u.c49, u.c50, u.c51, u.c52, u.review, u.stored_at, u.content_digest \
     FROM own_xid CROSS JOIN UNNEST($1::timestamptz[], $2::text[], $3::smallint[], $4::text[], \
     $5::text[], $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], \
     $11::text[], $12::text[], $13::text[], $14::text[], $15::bigint[], $16::text[], \
     $17::jsonb[], $18::jsonb[], $19::bigint[], $20::jsonb[], $21::text[], \
     $22::jsonb[], $23::text[], $24::text[], $25::text[], $26::text[], $27::text[], \
     $28::boolean[], $29::text[]::date[], $30::boolean[], $31::text[], $32::text[], \
     $33::text[], $34::text[], $35::text[], $36::boolean[], $37::jsonb[], $39::text[], \
     $40::text[], $41::boolean[], $42::jsonb[], $43::jsonb[], $44::jsonb[], $45::jsonb[], \
     $46::jsonb[], $47::jsonb[], $48::jsonb[], $49::jsonb[], $50::jsonb[], $51::jsonb[], \
     $52::jsonb[], $53::uuid[], $55::text[], $56::timestamptz[], $57::bytea[]) \
     AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, \
     c19, c20, c21, c22, c23, c24, c25, c26, c27, c28, c29, c30, c31, c32, c33, c34, c35, c36, \
     c37, tokens, c40, c41, c42, c43, c44, c45, c46, c47, c48, c49, c50, c51, c52, id, review, \
     stored_at, content_digest) \
     RETURNING seq) \
     INSERT INTO audit.audit_event_hold (seq, reserved_by) \
     SELECT w.seq, $54::uuid FROM written w WHERE $54::uuid IS NOT NULL";

/// The columns a stored record is read back from, in the order the ITI-81
/// search and the FHIR Feed select them.
#[cfg(feature = "fhir")]
const RECORD_COLUMNS: [&str; 46] = [
    "chain",
    "chain_seq",
    "recorded_at",
    "stored_at",
    "action",
    "outcome",
    "operation",
    "principal",
    "patient_id",
    "resource_class",
    "resource_id",
    "client_ip",
    "token_id",
    "domain",
    "purpose",
    "legal_basis",
    "result_count",
    "request_id",
    "roles",
    "origins",
    "origin_count",
    "category_map_digest",
    "acting_mode",
    "assurance_level",
    "assurance_value",
    "professional_id",
    "emergency_access",
    "permit",
    "issuer",
    "client_id",
    "ehr_id",
    "subject_namespace",
    "subject_lookup_failed",
    "review",
    "category_content",
    "render_policy",
    "token_attributes",
    "row_part",
    "shared_part",
    "entry_type",
    "entry_uid",
    "entry_ehr",
    "algorithm",
    "previous_digest",
    "content_digest",
    "digest",
];

/// The first date a stored record may be deleted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordEnd {
    /// On or after this UTC date.
    On(jiff::civil::Date),
    /// Never: the record is kept without end.
    Never,
}

/// One record as the store writes it: the resolved event, the patient its
/// subject resolver found behind the event's EHR, and the openEHR access-log
/// entry the access writes beside it.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    /// The resolved event.
    pub event: AuditEvent,
    /// The patient (EHR subject) id, when the resolver found one.
    pub subject: Option<String>,
    /// The access-log entry of the access (`crate::access_log::entry`),
    /// written in the transaction that writes the record; `None` for a
    /// record of no access to EHR data.
    pub entry:
        Option<Box<openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry>>,
}

/// The policies records name, compiled once each, by digest.
#[cfg(feature = "fhir")]
type PolicyCache = Arc<Mutex<BTreeMap<Vec<u8>, Arc<CompiledPolicy>>>>;

/// The shared sets a write learned, held once it commits.
#[cfg(feature = "fhir")]
type LearnedSets = crate::system_log::search::LearnedSets;
/// A slim build stores no search tokens, so a write learns nothing.
#[cfg(not(feature = "fhir"))]
type LearnedSets = ();

/// The PG-backed Audit Record Repository (the `store` sink).
#[derive(Debug, Clone)]
pub struct AuditStore {
    pub(crate) pool: PgPool,
    /// The sinks that deliver from this store, so each record chained is
    /// owed to them.
    owed: Owed,
    /// The policies the records this store writes are rendered under.
    writing: Arc<Writers>,
    /// The policies of the records this store has read.
    #[cfg(feature = "fhir")]
    policies: PolicyCache,
    /// The search tokens and sets this store has written.
    #[cfg(feature = "fhir")]
    tokens: Arc<crate::system_log::search::TokenRegistry>,
    /// The access-log store the records' entries are appended through, which
    /// signs their links as `[audit.access_log]` asks once the server installs
    /// it ([`Self::use_access_log`]); an unsigning one until then.
    access_log: Arc<std::sync::OnceLock<AccessLogStore>>,
    /// The committed reservations the chainer's next round releases.
    releases: Arc<intake::Releases>,
    /// The gathered intake write of the entries, started on first use.
    #[cfg(feature = "fhir")]
    group: Arc<crate::access_log::intake::Gathered>,
}

impl AuditStore {
    /// A store over the given pool, writing under the default render policy
    /// until [`Self::with_render_policy`] names the deployment's. The pool
    /// should be the audit pool: the audit trail is the node's operator
    /// surface, apart from the clinical data.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        AuditStore {
            pool,
            owed: Owed::default(),
            writing: Arc::new(Writers::new(RenderPolicy::default())),
            #[cfg(feature = "fhir")]
            policies: Arc::new(Mutex::new(BTreeMap::new())),
            #[cfg(feature = "fhir")]
            tokens: Arc::new(crate::system_log::search::TokenRegistry::default()),
            access_log: Arc::default(),
            releases: Arc::default(),
            #[cfg(feature = "fhir")]
            group: Arc::default(),
        }
    }

    /// This store over `pool`, its policies, tokens and access-log store
    /// shared with `self`.
    #[must_use]
    pub fn with_pool(&self, pool: PgPool) -> Self {
        Self {
            pool,
            ..self.clone()
        }
    }

    /// This store as the FHIR Feed's source when `owed`: its frontier round
    /// moves the delivery frontier and starts the feed's cursor there, and
    /// the feed delivers every record past it (`outbox`). Without it no
    /// record is owed to the feed.
    #[must_use]
    pub fn with_fhir_feed(mut self, owed: bool) -> Self {
        self.owed.fhir_feed = owed;
        self
    }

    /// This store as the syslog route's outbox when `owed`, as
    /// [`Self::with_fhir_feed`] for the feed.
    #[must_use]
    pub fn with_syslog(mut self, owed: bool) -> Self {
        self.owed.syslog = owed;
        self
    }

    /// The sinks that deliver from this store.
    #[must_use]
    pub fn owed(&self) -> Owed {
        self.owed
    }

    /// This store writing every record under `policy`, the configuration its
    /// FHIR `AuditEvent` is rendered under when it is read.
    #[must_use]
    pub fn with_render_policy(mut self, policy: RenderPolicy) -> Self {
        self.writing = Arc::new(Writers::new(policy));
        self
    }

    /// This store writing every record from now on under the policy in force
    /// as `change` alters it; a record of an earlier map keeps that map's.
    pub fn change_render_policy(&self, change: impl FnOnce(&mut RenderPolicy)) {
        self.writing.change(change);
    }

    /// Registers the render policy each of `records` is written under, as
    /// `writing` resolves it, on a connection of its own, before the write
    /// takes the connection it writes on: a write that holds a connection of
    /// this pool asks it for no other (#5879), and a policy is registered in
    /// a transaction of its own, so a write that rolls back leaves it stored
    /// (#5906). No specification governs this: our own design.
    ///
    /// # Errors
    /// [`AuditError::Policy`] when a policy cannot be serialized, and
    /// [`AuditError::Store`] when it cannot be stored.
    async fn register_policies(
        &self,
        writing: &Pinned,
        records: &[AuditRecord],
    ) -> Result<(), AuditError> {
        if writing
            .runs(records)
            .iter()
            .all(|(policy, _, _)| policy.digest.get().is_some())
        {
            return Ok(());
        }
        Self::register_policies_on(&mut *acquire(&self.pool).await?, writing, records).await
    }

    /// [`Self::register_policies`] on `conn`, a connection of the store's
    /// pool.
    async fn register_policies_on(
        conn: &mut PgConnection,
        writing: &Pinned,
        records: &[AuditRecord],
    ) -> Result<(), AuditError> {
        for (policy, _, _) in writing.runs(records) {
            policy
                .digest
                .get_or_try_init(|| async {
                    let json = serde_json::to_value(&policy.policy)
                        .map_err(|e| AuditError::Policy(Box::new(e)))?;
                    let digest: Vec<u8> =
                        sqlx::query_scalar("SELECT audit.register_render_policy($1)")
                            .bind(json)
                            .fetch_one(&mut *conn)
                            .await?;
                    Ok::<_, AuditError>(digest)
                })
                .await?;
        }
        Ok(())
    }

    /// The digest of `writing`, which [`Self::register_policies`] stored.
    ///
    /// # Errors
    /// [`AuditError::PolicyUnregistered`] when it did not.
    fn registered_digest(writing: &WritePolicy) -> Result<Vec<u8>, AuditError> {
        writing
            .digest
            .get()
            .cloned()
            .ok_or(AuditError::PolicyUnregistered)
    }

    /// The `search_tokens` value of each record: the ITI-81 search tokens of
    /// the document it renders under the store's policy, stored on `conn`, with
    /// the shared sets learned. A record whose document cannot be rendered is
    /// stored without tokens, metered and logged, since its document cannot be
    /// served either.
    ///
    /// # Errors
    /// The [`RenderPolicy::compile`] errors, [`AuditError::Store`] when a
    /// token cannot be stored, and [`AuditError::SearchToken`] when one cannot
    /// be read back.
    #[cfg(feature = "fhir")]
    async fn search_tokens<'a>(
        &self,
        writing: &WritePolicy,
        conn: &mut PgConnection,
        records: impl Iterator<Item = (&'a AuditEvent, Option<&'a str>)>,
    ) -> Result<(Vec<Vec<i32>>, LearnedSets), AuditError> {
        let compiled = writing.compiled().await?;
        let tokens: Vec<_> = records
            .map(|(event, subject)| {
                let resource =
                    crate::system_log::fhir::to_resource(event, &compiled.context, subject);
                entries::tokens_of(resource, event)
            })
            .collect();
        self.tokens.record_tokens(conn, &tokens).await
    }

    /// Holds the shared sets of a committed write; a slim build stores no
    /// search tokens, so it learns no set.
    fn remember(&self, learned: LearnedSets) {
        #[cfg(feature = "fhir")]
        self.tokens.remember(learned);
        #[cfg(not(feature = "fhir"))]
        let _: (LearnedSets, &Self) = (learned, self);
    }

    /// The `search_tokens` of each record of a batch, as the text of an
    /// `integer[]` the batched insert casts, with the shared sets learned.
    ///
    /// # Errors
    /// As [`Self::search_tokens`].
    #[cfg(feature = "fhir")]
    async fn batch_tokens(
        store: &Self,
        writing: &WritePolicy,
        conn: &mut PgConnection,
        records: &[AuditRecord],
    ) -> Result<(Vec<String>, LearnedSets), AuditError> {
        let (ids, learned) = store
            .search_tokens(
                writing,
                conn,
                records
                    .iter()
                    .map(|record| (&record.event, record.subject.as_deref())),
            )
            .await?;
        Ok((entries::token_texts(&ids), learned))
    }

    /// A slim build refuses the store at start
    /// ([`crate::system_log::sender::start`]), so no record it writes is
    /// searched: every record carries no search tokens.
    #[cfg(not(feature = "fhir"))]
    fn batch_tokens(
        _store: &Self,
        _writing: &WritePolicy,
        _conn: &mut PgConnection,
        records: &[AuditRecord],
    ) -> std::future::Ready<Result<(Vec<String>, LearnedSets), AuditError>> {
        std::future::ready(Ok((records.iter().map(|_| "{}".to_owned()).collect(), ())))
    }

    /// Store one audit record and seal it before this returns: an access
    /// the log covers as its entry, any other record by its columns. Returns
    /// the record's id: the entry's uid, or the system record's own.
    ///
    /// # Errors
    /// As [`Self::insert_batch`].
    pub async fn insert(
        &self,
        event: &AuditEvent,
        subject: Option<&str>,
    ) -> Result<Uuid, AuditError> {
        let record = AuditRecord {
            event: event.clone(),
            subject: subject.map(str::to_owned),
            entry: None,
        };
        let (taken, wrote) = self.take_in_own(vec![record], &[], None).await?;
        self.settle(wrote).await?;
        taken.first().copied().ok_or(AuditError::SearchToken)
    }

    /// Store a batch of records and seal them before this returns, each
    /// with its own retention end (`ends`, one per record, or empty when no
    /// record states one, so each keeps the store horizon).
    ///
    /// # Errors
    /// As [`Self::write`], and the sealing errors of [`Self::settle`].
    pub async fn insert_batch(
        &self,
        records: &[AuditRecord],
        ends: &[RecordEnd],
    ) -> Result<(), AuditError> {
        let (_, wrote) = self.take_in_own(records.to_vec(), ends, None).await?;
        self.settle(wrote).await
    }

    /// Write a batch of records in one transaction, each with its own
    /// retention end (`ends`, one per record, or empty), so they are durable
    /// and readable when this returns; the sealer seals them after.
    ///
    /// A record that carries its access-log entry, or whose access writes
    /// one, is written as the entry with its index row, every other one by
    /// its columns.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails (the whole batch fails
    /// together), [`AuditError::AccessLog`] when an entry has no stored form,
    /// and [`AuditError::RetentionEnds`] when `ends` is neither empty nor one
    /// end per record.
    pub async fn write(
        &self,
        records: Vec<AuditRecord>,
        ends: &[RecordEnd],
    ) -> Result<(), AuditError> {
        self.take_in_own(records, ends, None).await.map(drop)
    }

    /// [`Self::write`], every record held under `reservation` until it is
    /// released ([`Self::release`]) or discarded (#5762).
    ///
    /// # Errors
    /// As [`Self::write`].
    pub async fn write_held(
        &self,
        records: Vec<AuditRecord>,
        ends: &[RecordEnd],
        reservation: Uuid,
    ) -> Result<(), AuditError> {
        self.take_in_own(records, ends, Some(reservation))
            .await
            .map(drop)
    }

    /// [`Self::write_held`] on `conn`, a connection of the store's pool the
    /// caller took before its own transaction's (#5964).
    ///
    /// # Errors
    /// As [`Self::write`].
    pub async fn write_held_on(
        &self,
        conn: &mut PgConnection,
        records: Vec<AuditRecord>,
        ends: &[RecordEnd],
        reservation: Uuid,
    ) -> Result<(), AuditError> {
        if records.is_empty() {
            return Ok(());
        }
        let writing = self.writing.pin();
        Self::register_policies_on(conn, &writing, &records).await?;
        // NOTE: our own design (#6256), access-log entries alone go in by one statement, atomic
        // and durable on its own with synchronous_commit on, so they need no transaction.
        #[cfg(feature = "fhir")]
        if Self::only_entries(&records) {
            return self
                .held_entries_on(conn, &writing, records, reservation)
                .await;
        }
        let mut tx = conn.begin().await?;
        let (_, learned, _) = self
            .take_in(&mut tx, &writing, records, ends, Some(reservation))
            .await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows; a held row waits for its release, which marks it.
        tx.commit().await?;
        self.remember(learned);
        Ok(())
    }

    /// [`Self::write`], returning the id of each record written.
    async fn take_in_own(
        &self,
        records: Vec<AuditRecord>,
        ends: &[RecordEnd],
        reserved_by: Option<Uuid>,
    ) -> Result<(Vec<Uuid>, intake::Chains), AuditError> {
        if records.is_empty() {
            return Ok((Vec::new(), intake::Chains::default()));
        }
        let writing = self.writing.pin();
        self.register_policies(&writing, &records).await?;
        // NOTE: our own design (#5770), entries alone go into the gathered one-statement write.
        #[cfg(feature = "fhir")]
        if reserved_by.is_none() && Self::only_entries(&records) {
            let entered = self.with_entries(&writing, records).await?;
            let (uids, intake, tokens) = self.entry_rows(&writing, entered).await?;
            self.group().write(intake, tokens).await?;
            let wrote = intake::Chains {
                access_log: !uids.is_empty(),
                system: false,
            };
            return Ok((uids, wrote));
        }
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let (ids, learned, wrote) = self
            .take_in(&mut tx, &writing, records, ends, reserved_by)
            .await?;
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this
        // transaction's rows.
        tx.commit().await?;
        self.remember(learned);
        Ok((ids, wrote))
    }

    /// The gathered intake writer over this store's pool, started on the
    /// running runtime on first use.
    #[cfg(feature = "fhir")]
    fn group(&self) -> &crate::access_log::intake::GroupWriter {
        self.group.writer(&self.pool)
    }

    /// Whether every one of `records` is written as its access-log entry, so
    /// its intake write is the one statement of [`crate::access_log::intake::take_in`].
    #[cfg(feature = "fhir")]
    fn only_entries(records: &[AuditRecord]) -> bool {
        records.iter().all(|record| {
            record.entry.is_some() || crate::access_log::entry::writes_entry(&record.event)
        })
    }

    /// The `INSERT` of the system intake on `conn`, each record with its
    /// `search_tokens` text in `tokens`; returns each record's id.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails.
    async fn insert_rows(
        conn: &mut PgConnection,
        records: &[AuditRecord],
        ends: &[RecordEnd],
        policy: Vec<u8>,
        tokens: Vec<String>,
        reserved_by: Option<Uuid>,
    ) -> Result<Vec<Uuid>, AuditError> {
        let mut c = BatchColumns::with_capacity(records.len());
        let stored_at = content_digest::to_microsecond(jiff::Timestamp::now());
        for (index, record) in records.iter().enumerate() {
            c.push(record, ends.get(index), stored_at)?;
        }
        let ids = c.ids.clone();
        let digests = c.content_digests(&policy, &tokens)?;
        sqlx::query(INSERT_BATCH)
            .bind(c.recorded_at)
            .bind(c.actions)
            .bind(c.outcomes)
            .bind(c.event_codes)
            .bind(c.operations)
            .bind(c.principals)
            .bind(c.patient_ids)
            .bind(c.resource_classes)
            .bind(c.resource_ids)
            .bind(c.client_ips)
            .bind(c.token_ids)
            .bind(c.domains)
            .bind(c.purposes)
            .bind(c.legal_bases)
            .bind(c.result_counts)
            .bind(c.request_ids)
            .bind(c.roles)
            .bind(c.origins)
            .bind(c.origin_counts)
            .bind(c.categories)
            .bind(c.bases)
            .bind(c.evidence)
            .bind(c.digests)
            .bind(c.modes)
            .bind(c.levels)
            .bind(c.values)
            .bind(c.professionals)
            .bind(c.emergency)
            .bind(c.ends)
            .bind(c.stated)
            .bind(c.permits)
            .bind(c.issuers)
            .bind(c.clients)
            .bind(c.ehr_ids)
            .bind(c.namespaces)
            .bind(c.lookups_failed)
            .bind(c.contents)
            .bind(policy)
            .bind(tokens)
            .bind(c.bindings)
            .bind(c.guests)
            .bind(c.parties)
            .bind(c.groups)
            .bind(c.providers)
            .bind(c.representations)
            .bind(c.externals)
            .bind(c.agreements)
            .bind(c.actor_chains)
            .bind(c.facilities)
            .bind(c.details)
            .bind(c.summaries)
            .bind(c.token_attributes)
            .bind(c.ids)
            .bind(reserved_by)
            .bind(c.reviews)
            .bind(c.stored_at)
            .bind(digests)
            .execute(&mut *conn)
            .await?;
        Ok(ids)
    }

    /// The compiled render policy `digest` names, read from the store the
    /// first time.
    ///
    /// # Errors
    /// [`AuditError::Unreadable`] when the store holds no such policy,
    /// [`AuditError::UnreadableJson`] when it does not read, and the
    /// [`RenderPolicy::compile`] errors when it does not compile.
    #[cfg(feature = "fhir")]
    pub(super) async fn policy(
        &self,
        chain_seq: i64,
        digest: &[u8],
    ) -> Result<Arc<CompiledPolicy>, AuditError> {
        if let Some(policy) = self
            .policies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(digest)
        {
            return Ok(Arc::clone(policy));
        }
        let stored: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT policy FROM audit.audit_render_policy WHERE digest = $1")
                .bind(digest)
                .fetch_optional(&mut *acquire(&self.pool).await?)
                .await?;
        let Some(stored) = stored else {
            return Err(AuditError::Unreadable {
                chain_seq,
                reason: "it names a render policy the store does not hold",
            });
        };
        let policy: RenderPolicy =
            serde_json::from_value(stored).map_err(|source| AuditError::UnreadableJson {
                chain_seq,
                column: "render_policy",
                source,
            })?;
        let compiled = Arc::new(policy.compile()?);
        self.policies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(digest.to_vec(), Arc::clone(&compiled));
        Ok(compiled)
    }

    /// The FHIR R4 `AuditEvent` of the record at position `chain_seq` of
    /// `chain`, rendered as the ITI-81 search serves it; `None` when no
    /// record holds that position.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the SELECT fails, and the rendering errors of
    /// a record that cannot be read back.
    #[cfg(feature = "fhir")]
    pub async fn rendered(
        &self,
        chain: AuditChain,
        chain_seq: i64,
    ) -> Result<Option<serde_json::Value>, AuditError> {
        match self.row_at(chain, chain_seq).await? {
            Some(row) => {
                value_of(&self.resource(&row, row.try_get("shared_part")?).await?).map(Some)
            }
            None => Ok(None),
        }
    }

    /// Delete every record whose own retention end is on or before `today`,
    /// and every record that states no end (written before per-record ends)
    /// once it is older than `horizon`; returns the number of reaped rows.
    ///
    /// Regulation (EU) 2025/327 Annex II 3.4 asks for periods by category and
    /// origin, so a record is never deleted before its own end and a longer-kept
    /// one outlives a shorter-kept one written the same day. The deletion and
    /// the chain tombstones are `audit.reap_audit_events_due`.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the reap fails.
    pub async fn reap_due(
        &self,
        today: jiff::civil::Date,
        horizon: crate::system_log::config::Retention,
    ) -> Result<u64, AuditError> {
        use crate::system_log::config::Retention;
        // NOTE: a NULL horizon makes the cutoff NULL, which keeps every record that states no end.
        let horizon = match horizon {
            Retention::Forever => None,
            Retention::Days(days) => Some(format!("{days} days")),
            Retention::Years(years) => Some(format!("{years} years")),
        };
        let removed: i64 = sqlx::query_scalar(
            "SELECT audit.reap_audit_events_due($1::text::date, now() - $2::text::interval)",
        )
        .bind(today.to_string())
        .bind(horizon)
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?;
        u64::try_from(removed).map_err(|e| AuditError::Store(sqlx::Error::Decode(Box::new(e))))
    }

    /// The RESTful-ATNA ITI-81 retrieval: the FHIR `AuditEvent` documents of
    /// the matching records, newest first, rendered from their columns,
    /// whether a match follows the page, and the total match count when
    /// [`AuditSearchFilter::total`] asks for it. The filter is the supported
    /// ITI-81 search-parameter subset ([`AuditSearchFilter`]).
    ///
    /// A criterion on a column is applied to that column. A criterion on an
    /// element of the document (`address` and the token parameters) is first
    /// resolved to the ids of the stored search tokens it names, and the
    /// statement keeps the records whose `search_tokens` carry one of them,
    /// through its index. Only the page is rendered.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a query fails, and the rendering errors of a
    /// record that cannot be read back.
    #[cfg(feature = "fhir")]
    pub async fn search(&self, filter: &AuditSearchFilter) -> Result<AuditPage, AuditError> {
        let Some(resolved) =
            crate::system_log::search::resolve_criteria(&self.pool, filter).await?
        else {
            return Ok(AuditPage::empty(filter));
        };
        let (count_sql, count_values, sql, values) =
            search_page::build_search_statements(filter, &resolved);
        // NOTE: our own design (#6005), a shape's text is a named statement that PostgreSQL may
        // plan once per connection (plan_cache_mode auto); any other text goes unnamed.
        let shaped = search_page::shaped(filter);
        let total = if filter.total {
            let total: i64 = sqlx::query_scalar_with(sqlx::AssertSqlSafe(count_sql), count_values)
                .persistent(shaped)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            Some(total)
        } else {
            None
        };
        let mut rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .persistent(shaped)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        // The page statement reads one record past the page, which says whether a next
        // page exists without counting the matches.
        let more = rows.len() > search_page::page_size(filter);
        rows.truncate(search_page::page_size(filter));
        let documents = self.page_documents(&rows).await?;
        Ok(AuditPage {
            total,
            documents,
            more,
        })
    }
}

/// The record columns of the records `condition` selects, newest first.
#[cfg(feature = "fhir")]
pub(super) fn newest_first(condition: sea_query::Cond) -> sea_query::SelectStatement {
    use sea_query::{Alias, Query};

    search_page::newest_first_of(
        Query::select()
            .columns(RECORD_COLUMNS.map(Alias::new))
            .from((Alias::new("audit"), Alias::new("audit_record")))
            .cond_where(condition),
    )
    .to_owned()
}

/// One page of the ITI-81 retrieval.
#[cfg(feature = "fhir")]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditPage {
    /// Every match across all pages, counted only when the filter asks for
    /// it (FHIR R4 Search §3.1.1.5.2, `_total`; `Bundle.total` is optional).
    pub total: Option<i64>,
    /// The page's documents, newest first, as `ihe-atna` wrote them.
    pub documents: Vec<fhir_types::r4::audit_event::AuditEvent>,
    /// Whether a match follows the page.
    pub more: bool,
}

#[cfg(feature = "fhir")]
impl AuditPage {
    /// The page of a filter that matches nothing.
    #[must_use]
    pub fn empty(filter: &AuditSearchFilter) -> Self {
        Self {
            total: filter.total.then_some(0),
            documents: Vec::new(),
            more: false,
        }
    }
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
    /// Whether every match is counted: `_total=accurate` or `estimate`, or
    /// `_count=0` (FHIR R4 Search §3.1.1.5.2). A page alone reads no more
    /// records than it serves, so its cost does not grow with the log.
    pub total: bool,
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
    /// A token parameter matched against an element of the rendered
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

/// The code system the role of the patient entity is coded in (FHIR R4
/// `object-role`, code `1` Patient).
pub(crate) const OBJECT_ROLE_SYSTEM: &str = "http://terminology.hl7.org/CodeSystem/object-role";

impl AuditElement {
    /// The nodes of `document` the element names, and whether each is an
    /// `Identifier` (its code in `value`) rather than a `Coding` (its code in
    /// `code`).
    pub(crate) fn nodes(self, document: &serde_json::Value) -> (Vec<&serde_json::Value>, bool) {
        let entities = || lax_path(document, &["entity"]);
        match self {
            Self::AgentIdentifier => (lax_path(document, &["agent", "who", "identifier"]), true),
            Self::PatientIdentifier => (
                entities()
                    .into_iter()
                    .filter(|entity| {
                        any_equals(entity, &["role", "code"], "1")
                            && any_equals(entity, &["role", "system"], OBJECT_ROLE_SYSTEM)
                    })
                    .flat_map(|entity| lax_path(entity, &["what", "identifier"]))
                    .collect(),
                true,
            ),
            Self::EntityIdentifier => (lax_path(document, &["entity", "what", "identifier"]), true),
            Self::SourceIdentifier => (
                lax_path(document, &["source", "observer", "identifier"]),
                true,
            ),
            Self::EntityType => (lax_path(document, &["entity", "type"]), false),
            Self::EntityRole => (lax_path(document, &["entity", "role"]), false),
            Self::Type => (lax_path(document, &["type"]), false),
            Self::Subtype => (lax_path(document, &["subtype"]), false),
        }
    }
}

impl AuditCriterion {
    /// Whether the criterion is matched against the rendered document rather
    /// than a column.
    #[must_use]
    pub fn reads_document(&self) -> bool {
        matches!(self, Self::Address(_) | Self::Element(..))
    }

    /// Whether `document` matches this criterion: any of its values does. A
    /// criterion on a column matches every document; the statement applied it.
    #[must_use]
    pub fn matches(&self, document: &serde_json::Value) -> bool {
        match self {
            // NOTE: `RESTful` ATNA §3.81.4.1.2.2, `address` matches a substring of
            // `agent.network.address`, compared without regard to case.
            Self::Address(values) => {
                let addresses: Vec<String> = lax_path(document, &["agent", "network", "address"])
                    .into_iter()
                    .map(|address| match address {
                        serde_json::Value::String(text) => text.to_lowercase(),
                        other => other.to_string().to_lowercase(),
                    })
                    .collect();
                values.iter().any(|value| {
                    let value = value.to_lowercase();
                    addresses.iter().any(|address| address.contains(&value))
                })
            }
            Self::Element(element, tokens) => {
                let (nodes, identifier) = element.nodes(document);
                tokens
                    .iter()
                    .any(|token| nodes.iter().any(|node| token.matches(node, identifier)))
            }
            Self::Patient(_)
            | Self::Agent(_)
            | Self::Entity(_)
            | Self::Outcome(_)
            | Self::Action(_) => true,
        }
    }
}

impl AuditToken {
    /// Whether `node`, an `Identifier` when `identifier` and a `Coding`
    /// otherwise, carries this token: the three token forms of FHIR R4 Search
    /// §3.1.1.4.10.
    fn matches(&self, node: &serde_json::Value, identifier: bool) -> bool {
        let key = if identifier { "value" } else { "code" };
        let code = || match &self.code {
            Some(code) => any_is(node, &[key], &serde_json::Value::String(code.clone())),
            None => any_is(node, &[key], &serde_json::Value::Null),
        };
        match &self.system {
            AuditTokenSystem::Any => code(),
            AuditTokenSystem::Absent => code() && lax_path(node, &["system"]).is_empty(),
            AuditTokenSystem::Named(system) if self.code.is_some() => {
                code() && any_equals(node, &["system"], system)
            }
            AuditTokenSystem::Named(system) => any_equals(node, &["system"], system),
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
    event.object.event_id(event.action).0
}

/// The `operation` column: the ITS-REST operation id, or the DCM
/// login/logout `EventTypeCode` csd-code for authentication records.
fn operation(event: &AuditEvent) -> Option<&str> {
    match &event.event_type {
        Some(EventType::RestOperation(op)) => Some(op),
        Some(EventType::Login) => Some(LOGIN_OPERATION),
        Some(EventType::Logout) => Some(LOGOUT_OPERATION),
        None => None,
    }
}

/// The `operation` of an authentication record: DCM 110122 "Login".
const LOGIN_OPERATION: &str = "110122";

/// The `operation` of a session end: DCM 110123 "Logout".
const LOGOUT_OPERATION: &str = "110123";

pub(crate) const fn resource_class(object: ObjectClass) -> &'static str {
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

/// The `token_attributes` column value: what the access token states, or
/// `NULL` when it states nothing.
fn token_attributes_json(event: &AuditEvent) -> Option<serde_json::Value> {
    event
        .token
        .as_ref()
        .and_then(|token| serde_json::to_value(token).ok())
}

/// The `origin_count` column value.
fn origin_count(event: &AuditEvent) -> Option<i64> {
    event
        .origin_count
        .map(|count| i64::try_from(count).unwrap_or(i64::MAX))
}

/// The `binding` and `guest` column values of `event` and its jsonb binding
/// columns ([`crate::system_log::binding::BindingRecord::json_columns`]),
/// with the `summary` column last.
fn binding_columns(
    event: &AuditEvent,
) -> (Option<&'static str>, bool, [Option<serde_json::Value>; 10]) {
    let (code, guest, binding) = event.binding.as_ref().map_or_else(
        || (None, false, Default::default()),
        |binding| {
            (
                Some(binding.code.as_str()),
                binding.guest,
                binding.json_columns(),
            )
        },
    );
    let summary = event
        .summary
        .as_ref()
        .and_then(|summary| serde_json::to_value(summary).ok());
    let mut columns: [Option<serde_json::Value>; 10] = Default::default();
    for (column, value) in columns.iter_mut().zip(binding.into_iter().chain([summary])) {
        *column = value;
    }
    (code, guest, columns)
}

/// The `ehr_id` column value: the EHR a classified access reached, which its
/// EHDS access record names; `NULL` for every other record, whose document
/// names no EHR.
///
/// NOTE: no specification governs this: our own design; a linkage record names a party, and
/// the EHR beside it would make the audit store a map from parties to their EHRs.
fn access_ehr_id(event: &AuditEvent) -> Option<&str> {
    event.category.as_ref().and(event.ehr_id.as_deref())
}

/// The `category_content` column value: what the classification read, or
/// `NULL` when the operation touched no clinical content.
///
/// # Errors
/// [`AuditError::Render`] when the content cannot be written as JSON, which
/// its plain shape does not cause.
fn category_content_json(event: &AuditEvent) -> Result<Option<serde_json::Value>, AuditError> {
    event
        .category
        .as_ref()
        .map(|record| serde_json::to_value(&record.content))
        .transpose()
        .map_err(|e| AuditError::Render(Box::new(e)))
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

/// The render policy digest and the event a stored row holds.
///
/// # Errors
/// [`AuditError::Store`] when a column does not decode,
/// [`AuditError::Unreadable`] when it holds a value this build does not write,
/// and [`AuditError::UnreadableJson`] when a JSON column does not read.
#[cfg(feature = "fhir")]
pub(super) fn stored_record(row: &PgRow) -> Result<(Vec<u8>, StoredRecord), AuditError> {
    let chain_seq: i64 = row.try_get("chain_seq")?;
    let unreadable = |reason: &'static str| AuditError::Unreadable { chain_seq, reason };
    let action = match row.try_get::<&str, _>("action")? {
        "C" => EventActionCode::Create,
        "R" => EventActionCode::Read,
        "U" => EventActionCode::Update,
        "D" => EventActionCode::Delete,
        "E" => EventActionCode::Execute,
        _ => return Err(unreadable("its action is none this build writes")),
    };
    let outcome = match row.try_get::<i16, _>("outcome")? {
        0 => EventOutcome::Success,
        4 => EventOutcome::MinorFailure,
        8 => EventOutcome::SeriousFailure,
        _ => return Err(unreadable("its outcome is none this build writes")),
    };
    let object = object_class(row.try_get("resource_class")?)
        .ok_or_else(|| unreadable("its resource class is none this build writes"))?;
    let mut event = AuditEvent::new(action, object, outcome);
    event.timestamp = row.try_get::<Timestamp, _>("recorded_at")?.to_jiff();
    event.event_type = row
        .try_get::<Option<String>, _>("operation")?
        .map(|operation| match operation.as_str() {
            LOGIN_OPERATION => EventType::Login,
            LOGOUT_OPERATION => EventType::Logout,
            _ => EventType::RestOperation(std::borrow::Cow::Owned(operation)),
        });
    event.user_id = row
        .try_get::<Option<String>, _>("principal")?
        .unwrap_or_default();
    event.issuer = row.try_get("issuer")?;
    event.client_id = row.try_get("client_id")?;
    event.roles = strings(row, "roles", chain_seq)?;
    event.client_ip = row.try_get("client_ip")?;
    event.ehr_id = row.try_get("ehr_id")?;
    event.subject_namespace = row.try_get("subject_namespace")?;
    event.subject_lookup_failed = row.try_get("subject_lookup_failed")?;
    event.review = review::level(row, chain_seq)?;
    event.object_id = row.try_get("resource_id")?;
    event.token_id = row.try_get("token_id")?;
    event.domain = access_domain(row.try_get("domain")?)
        .ok_or_else(|| unreadable("its domain is none this build writes"))?;
    event.purpose = row.try_get("purpose")?;
    event.token = row
        .try_get::<Option<serde_json::Value>, _>("token_attributes")?
        .map(serde_json::from_value::<crate::system_log::event::TokenAttributes>)
        .transpose()
        .map_err(|_unreadable| unreadable("its token attributes do not read"))?;
    event.permit = row.try_get("permit")?;
    event.emergency_access = row.try_get("emergency_access")?;
    event.legal_basis = row.try_get("legal_basis")?;
    event.result_count = count(row, "result_count", chain_seq)?;
    event.origins = strings(row, "origins", chain_seq)?;
    event.origin_count = count(row, "origin_count", chain_seq)?;
    event.actor.mode = match row.try_get::<Option<&str>, _>("acting_mode")? {
        None => None,
        Some("person") => Some(ActingMode::Person),
        Some("client") => Some(ActingMode::Client),
        Some("delegated") => Some(ActingMode::Delegated),
        Some("external") => Some(ActingMode::External),
        Some("system") => Some(ActingMode::System),
        Some(_) => return Err(unreadable("its acting mode is none this build writes")),
    };
    event.actor.assurance_level = match row.try_get::<Option<&str>, _>("assurance_level")? {
        None => None,
        Some("low") => Some(AssuranceLevel::Low),
        Some("substantial") => Some(AssuranceLevel::Substantial),
        Some("high") => Some(AssuranceLevel::High),
        Some(_) => return Err(unreadable("its assurance level is none this build writes")),
    };
    event.actor.assurance_value = row.try_get("assurance_value")?;
    event.actor.professional_id = row.try_get("professional_id")?;
    event.request_id = row.try_get("request_id")?;
    let map_digest = row
        .try_get::<Option<&str>, _>("category_map_digest")?
        .map(str::parse::<MapDigest>)
        .transpose()
        .map_err(|_unreadable| unreadable("its category map digest does not read"))?;
    event.category_map_digest = map_digest;
    let content = row
        .try_get::<Option<serde_json::Value>, _>("category_content")?
        .map(serde_json::from_value::<ClassifiedContent>)
        .transpose()
        .map_err(|source| AuditError::UnreadableJson {
            chain_seq,
            column: "category_content",
            source,
        })?;
    let stored = StoredRecord {
        chain_seq,
        event,
        subject: row.try_get("patient_id")?,
        content,
        map_digest,
    };
    Ok((row.try_get("render_policy")?, stored))
}

/// The resource class `stored` names, the inverse of [`resource_class`].
#[cfg(feature = "fhir")]
pub(crate) fn object_class(stored: &str) -> Option<ObjectClass> {
    [
        ObjectClass::Ehr,
        ObjectClass::Composition,
        ObjectClass::Contribution,
        ObjectClass::Directory,
        ObjectClass::Query,
        ObjectClass::Template,
        ObjectClass::Demographic,
        ObjectClass::Extract,
        ObjectClass::ApplicationActivity,
        ObjectClass::Authentication,
    ]
    .into_iter()
    .find(|object| resource_class(*object) == stored)
}

/// The domain `stored` names, the inverse of [`AccessDomain::as_str`].
#[cfg(feature = "fhir")]
pub(crate) fn access_domain(stored: Option<&str>) -> Option<AccessDomain> {
    let stored = stored?;
    [
        AccessDomain::Ehr,
        AccessDomain::Demographic,
        AccessDomain::Linkage,
        AccessDomain::System,
    ]
    .into_iter()
    .find(|domain| domain.as_str() == stored)
}

/// The strings of the JSON array in `column`; none for `NULL`.
///
/// # Errors
/// [`AuditError::Unreadable`] when the column holds anything but an array of
/// strings.
#[cfg(feature = "fhir")]
fn strings(row: &PgRow, column: &'static str, chain_seq: i64) -> Result<Vec<String>, AuditError> {
    let Some(stored) = row.try_get::<Option<serde_json::Value>, _>(column)? else {
        return Ok(Vec::new());
    };
    serde_json::from_value(stored).map_err(|source| AuditError::UnreadableJson {
        chain_seq,
        column,
        source,
    })
}

/// The count in `column`.
///
/// # Errors
/// [`AuditError::Unreadable`] for a negative count.
#[cfg(feature = "fhir")]
fn count(row: &PgRow, column: &'static str, chain_seq: i64) -> Result<Option<u64>, AuditError> {
    row.try_get::<Option<i64>, _>(column)?
        .map(u64::try_from)
        .transpose()
        .map_err(|_negative| AuditError::Unreadable {
            chain_seq,
            reason: "it holds a negative count",
        })
}

#[cfg(all(test, feature = "fhir"))]
mod tests;
