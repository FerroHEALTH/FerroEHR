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
//! A record keeps its facts in columns and names the render policy it was
//! written under ([`crate::system_log::render`]). The FHIR R4 `AuditEvent`
//! (IHE BALP shape, the `fhir` cargo feature) is rendered from them when the
//! record is read by the RESTful-ATNA ITI-81 search or delivered by the FHIR
//! Feed, so the same record always renders the same document. Rows are
//! append-only except the per-sink delivery stamps and retention reaping.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the store carries an \
              already-rendered FHIR document, never a typed resource"
)]

#[cfg(feature = "fhir")]
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
#[cfg(feature = "fhir")]
use std::sync::{Mutex, PoisonError};

use jiff_sqlx::Timestamp;
use sqlx::postgres::{PgConnection, PgRow};
use sqlx::{Connection, PgPool, Row};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::system_log::AuditError;
#[cfg(feature = "fhir")]
use crate::system_log::categories::{ClassifiedContent, MapDigest};
use crate::system_log::codes::AtnaAction;
#[cfg(feature = "fhir")]
use crate::system_log::event::{AccessDomain, EventActionCode};
use crate::system_log::event::{ActingMode, AuditEvent, EventOutcome, EventType, ObjectClass};
use crate::system_log::render::RenderPolicy;
#[cfg(feature = "fhir")]
use crate::system_log::render::{CompiledPolicy, StoredRecord};

use ferrohealth_smart::assurance::AssuranceLevel;

/// The single-record insert, returning the stored row id.
const INSERT_ONE: &str = "INSERT INTO audit.audit_event (recorded_at, action, outcome, \
     event_code, operation, principal, organisation, patient_id, resource_class, resource_id, \
     client_ip, token_id, domain, purpose, legal_basis, result_count, request_id, roles, \
     origins, origin_count, categories, category_basis, category_evidence, category_map_digest, \
     acting_mode, assurance_level, assurance_value, professional_id, emergency_access, permit, \
     issuer, client_id, ehr_id, subject_namespace, subject_lookup_failed, category_content, \
     render_policy, search_tokens) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, \
     $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35, $36, \
     $37, $38) \
     RETURNING id";

/// The batched insert: one row per element of the parallel column arrays, every
/// row under the one render policy in `$39`; each record's search tokens arrive
/// as the text of an `integer[]` in `$40`, since an array of arrays unnests
/// element by element.
const INSERT_BATCH: &str = "INSERT INTO audit.audit_event (recorded_at, action, outcome, \
     event_code, operation, principal, organisation, patient_id, resource_class, resource_id, \
     client_ip, token_id, domain, purpose, legal_basis, result_count, request_id, roles, \
     origins, origin_count, categories, category_basis, category_evidence, category_map_digest, \
     acting_mode, assurance_level, assurance_value, professional_id, emergency_access, \
     retention_ends, retention_stated, permit, issuer, client_id, ehr_id, subject_namespace, \
     subject_lookup_failed, category_content, render_policy, search_tokens) \
     SELECT u.c1, u.c2, u.c3, u.c4, u.c5, u.c6, u.c7, u.c8, u.c9, u.c10, u.c11, u.c12, u.c13, \
     u.c14, u.c15, u.c16, u.c17, u.c18, u.c19, u.c20, u.c21, u.c22, u.c23, u.c24, u.c25, u.c26, \
     u.c27, u.c28, u.c29, u.c30, u.c31, u.c32, u.c33, u.c34, u.c35, u.c36, u.c37, u.c38, \
     $39::bytea, u.tokens::integer[] \
     FROM UNNEST($1::timestamptz[], $2::text[], $3::smallint[], $4::text[], \
     $5::text[], $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], $11::text[], \
     $12::text[], $13::text[], $14::text[], $15::text[], $16::bigint[], $17::text[], \
     $18::jsonb[], $19::jsonb[], $20::bigint[], $21::jsonb[], $22::text[], \
     $23::jsonb[], $24::text[], $25::text[], $26::text[], $27::text[], $28::text[], \
     $29::boolean[], $30::text[]::date[], $31::boolean[], $32::text[], $33::text[], \
     $34::text[], $35::text[], $36::text[], $37::boolean[], $38::jsonb[], $40::text[]) \
     AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, c19, \
     c20, c21, c22, c23, c24, c25, c26, c27, c28, c29, c30, c31, c32, c33, c34, c35, c36, c37, \
     c38, tokens)";

/// The columns a stored record is read back from, in the order the ITI-81
/// search and the FHIR Feed select them.
#[cfg(feature = "fhir")]
const RECORD_COLUMNS: [&str; 35] = [
    "chain_seq",
    "recorded_at",
    "stored_at",
    "action",
    "outcome",
    "operation",
    "principal",
    "organisation",
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
    "category_content",
    "render_policy",
];

/// The oldest record the FHIR Feed owes past the chain positions in `$1`,
/// read as [`RECORD_COLUMNS`].
#[cfg(feature = "fhir")]
const OLDEST_OWED: &str = "SELECT \
     e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation, \
     e.principal, e.organisation, e.patient_id, e.resource_class, e.resource_id, \
     e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count, \
     e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest, \
     e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id, \
     e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id, \
     e.subject_namespace, e.subject_lookup_failed, e.category_content, e.render_policy \
     FROM audit.fhir_feed_outbox o \
     JOIN audit.audit_event e ON e.chain_seq = o.chain_seq \
     WHERE NOT (o.chain_seq = ANY($1)) \
     ORDER BY o.chain_seq ASC LIMIT 1";

/// The first date a stored record may be deleted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordEnd {
    /// On or after this UTC date.
    On(jiff::civil::Date),
    /// Never: the record is kept without end.
    Never,
}

/// One record as the store writes it: the resolved event and the patient its
/// subject resolver found behind the event's EHR.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    /// The resolved event.
    pub event: AuditEvent,
    /// The patient (EHR subject) id, when the resolver found one.
    pub subject: Option<String>,
}

/// [`INSERT_ONE`] that also enters the record in the FHIR Feed's outbox, in
/// the same statement.
static INSERT_ONE_OWED: LazyLock<String> = LazyLock::new(|| {
    format!(
        "WITH stored AS ({INSERT_ONE}, chain_seq), \
         owed AS (INSERT INTO audit.fhir_feed_outbox (chain_seq) SELECT chain_seq FROM stored) \
         SELECT id FROM stored"
    )
});

/// [`INSERT_BATCH`] that also enters every record in the FHIR Feed's outbox,
/// in the same statement.
static INSERT_BATCH_OWED: LazyLock<String> = LazyLock::new(|| {
    format!(
        "WITH stored AS ({INSERT_BATCH} RETURNING chain_seq) \
         INSERT INTO audit.fhir_feed_outbox (chain_seq) SELECT chain_seq FROM stored"
    )
});

/// The render policy a store writes its records under, registered in the
/// store the first time a record is written.
#[derive(Debug)]
struct WritePolicy {
    policy: RenderPolicy,
    /// The policy's digest, once the store holds the policy.
    digest: OnceCell<Vec<u8>>,
    /// The policy compiled, which renders each record's document for its
    /// search tokens.
    #[cfg(feature = "fhir")]
    compiled: OnceCell<Arc<CompiledPolicy>>,
}

impl WritePolicy {
    fn new(policy: RenderPolicy) -> Self {
        Self {
            policy,
            digest: OnceCell::new(),
            #[cfg(feature = "fhir")]
            compiled: OnceCell::new(),
        }
    }
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
    pub(super) pool: PgPool,
    /// Whether the FHIR Feed delivers from this store, so each record written
    /// is owed to it.
    fhir_feed: bool,
    /// The policy every record this store writes is rendered under.
    writing: Arc<WritePolicy>,
    /// The policies of the records this store has read.
    #[cfg(feature = "fhir")]
    policies: PolicyCache,
    /// The search tokens and sets this store has written.
    #[cfg(feature = "fhir")]
    tokens: Arc<crate::system_log::search::TokenRegistry>,
}

impl AuditStore {
    /// A store over the given pool, writing under the default render policy
    /// until [`Self::with_render_policy`] names the deployment's. The pool
    /// should be the plain pool: the audit trail is the node's operator
    /// surface, written by the drain task rather than by a request.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        AuditStore {
            pool,
            fhir_feed: false,
            writing: Arc::new(WritePolicy::new(RenderPolicy::default())),
            #[cfg(feature = "fhir")]
            policies: Arc::new(Mutex::new(BTreeMap::new())),
            #[cfg(feature = "fhir")]
            tokens: Arc::new(crate::system_log::search::TokenRegistry::default()),
        }
    }

    /// `owed` when the FHIR Feed delivers from this store, else `plain`.
    fn owed(&self, plain: &'static str, owed: &'static LazyLock<String>) -> &'static str {
        if self.fhir_feed { owed.as_str() } else { plain }
    }

    /// This store as the FHIR Feed's outbox when `owed`: every record it
    /// writes from now on is owed to the feed until
    /// [`Self::mark_fhir_feed_delivered`]. Without it a record is owed to no
    /// sink.
    #[must_use]
    pub fn with_fhir_feed(mut self, owed: bool) -> Self {
        self.fhir_feed = owed;
        self
    }

    /// This store writing every record under `policy`, the configuration its
    /// FHIR `AuditEvent` is rendered under when it is read.
    #[must_use]
    pub fn with_render_policy(mut self, policy: RenderPolicy) -> Self {
        self.writing = Arc::new(WritePolicy::new(policy));
        self
    }

    /// The digest of the policy this store writes under, storing the policy
    /// the first time.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the policy cannot be stored.
    async fn writing_digest(&self) -> Result<Vec<u8>, AuditError> {
        let writing = &self.writing;
        writing
            .digest
            .get_or_try_init(|| async {
                let policy = serde_json::to_value(&writing.policy)
                    .map_err(|e| AuditError::Policy(Box::new(e)))?;
                let digest: Vec<u8> = sqlx::query_scalar("SELECT audit.register_render_policy($1)")
                    .bind(policy)
                    .fetch_one(&mut *acquire(&self.pool).await?)
                    .await?;
                Ok::<_, AuditError>(digest)
            })
            .await
            .cloned()
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
        conn: &mut PgConnection,
        records: impl Iterator<Item = (&'a AuditEvent, Option<&'a str>)>,
    ) -> Result<(Vec<Vec<i32>>, LearnedSets), AuditError> {
        let compiled = self
            .writing
            .compiled
            .get_or_try_init(|| async { self.writing.policy.compile().map(Arc::new) })
            .await?;
        let tokens: Vec<_> = records
            .map(|(event, subject)| {
                match crate::system_log::fhir::to_fhir(event, &compiled.context, subject) {
                    Ok(document) => crate::system_log::search::document_tokens(&document, event),
                    Err(e) => {
                        crate::telemetry::metrics::metrics()
                            .atna_audit_serialize_failed
                            .add(1, &[]);
                        tracing::warn!("ATNA audit record stored without search tokens: {e}");
                        std::collections::BTreeSet::new()
                    }
                }
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
        conn: &mut PgConnection,
        records: &[AuditRecord],
    ) -> Result<(Vec<String>, LearnedSets), AuditError> {
        let (ids, learned) = store
            .search_tokens(
                conn,
                records
                    .iter()
                    .map(|record| (&record.event, record.subject.as_deref())),
            )
            .await?;
        let texts = ids
            .iter()
            .map(|ids| {
                let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
                format!("{{{}}}", ids.join(","))
            })
            .collect();
        Ok((texts, learned))
    }

    /// A slim build refuses the store at start
    /// ([`crate::system_log::sender::start`]), so no record it writes is
    /// searched: every record carries no search tokens.
    #[cfg(not(feature = "fhir"))]
    fn batch_tokens(
        _store: &Self,
        _conn: &mut PgConnection,
        records: &[AuditRecord],
    ) -> std::future::Ready<Result<(Vec<String>, LearnedSets), AuditError>> {
        std::future::ready(Ok((records.iter().map(|_| "{}".to_owned()).collect(), ())))
    }

    /// Persist one audit record: its facts as columns, under the store's
    /// render policy, in one transaction with its search tokens. Returns the
    /// stored row id (for the per-sink delivery stamps).
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails.
    pub async fn insert(
        &self,
        event: &AuditEvent,
        subject: Option<&str>,
    ) -> Result<Uuid, AuditError> {
        let policy = self.writing_digest().await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        #[cfg(feature = "fhir")]
        let (tokens, learned) = {
            let (mut tokens, learned) = self
                .search_tokens(&mut tx, std::iter::once((event, subject)))
                .await?;
            (tokens.pop().ok_or(AuditError::SearchToken)?, learned)
        };
        // NOTE: a slim build refuses the store at start, so no record it writes is searched.
        #[cfg(not(feature = "fhir"))]
        let (tokens, learned): (Vec<i32>, LearnedSets) = (Vec::new(), ());
        let outcome = outcome_smallint(event);
        let id = sqlx::query(self.owed(INSERT_ONE, &INSERT_ONE_OWED))
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
            .bind(event.permit.as_deref())
            .bind(event.issuer.as_deref())
            .bind(event.client_id.as_deref())
            .bind(access_ehr_id(event))
            .bind(event.subject_namespace.as_deref())
            .bind(event.subject_lookup_failed)
            .bind(category_content_json(event)?)
            .bind(policy)
            .bind(tokens)
            .fetch_one(&mut *tx)
            .await?
            .try_get::<Uuid, _>("id")
            .map_err(AuditError::Store)?;
        tx.commit().await?;
        self.remember(learned);
        Ok(id)
    }

    /// Persist a whole drained batch in ONE multi-row `INSERT` (UNNEST over
    /// parallel column arrays) — the throughput path for the default
    /// store-only posture, where per-event round trips cannot keep up with a
    /// loaded write path. No row ids are returned: the batch path is used
    /// only when no per-row delivery stamping is needed.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails (the whole batch fails
    /// together; the caller retries).
    pub async fn insert_batch(&self, records: &[AuditRecord]) -> Result<(), AuditError> {
        self.write_batch(records, &[]).await
    }

    /// Persist a drained batch as [`Self::insert_batch`] does, each record with
    /// the first date it may be deleted on (`ends`, one per record), which the
    /// reaper honours ([`Self::reap_due`]).
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails, and [`AuditError::RetentionEnds`]
    /// when `ends` does not hold one end per record.
    pub async fn insert_batch_ending(
        &self,
        records: &[AuditRecord],
        ends: &[RecordEnd],
    ) -> Result<(), AuditError> {
        if ends.len() != records.len() {
            return Err(AuditError::RetentionEnds {
                ends: ends.len(),
                records: records.len(),
            });
        }
        self.write_batch(records, ends).await
    }

    /// The multi-row `INSERT` behind [`Self::insert_batch`] and
    /// [`Self::insert_batch_ending`]; a record past the end of `ends` states no
    /// end and keeps the store horizon.
    async fn write_batch(
        &self,
        records: &[AuditRecord],
        ends: &[RecordEnd],
    ) -> Result<(), AuditError> {
        if records.is_empty() {
            return Ok(());
        }
        let policy = self.writing_digest().await?;
        // NOTE: one transaction holds the tokens and the records, so a batch costs one commit.
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let (tokens, learned) = Self::batch_tokens(self, &mut tx, records).await?;
        self.insert_rows(&mut tx, records, ends, policy, tokens)
            .await?;
        tx.commit().await?;
        self.remember(learned);
        Ok(())
    }

    /// The multi-row `INSERT` of [`Self::write_batch`] on `conn`, each record
    /// with its `search_tokens` text in `tokens`.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the INSERT fails.
    async fn insert_rows(
        &self,
        conn: &mut PgConnection,
        records: &[AuditRecord],
        ends: &[RecordEnd],
        policy: Vec<u8>,
        tokens: Vec<String>,
    ) -> Result<(), AuditError> {
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
        let mut declared = DeclaredColumns::with_capacity(records.len());
        let mut result_counts: Vec<Option<i64>> = Vec::with_capacity(records.len());
        let mut request_ids: Vec<Option<&str>> = Vec::with_capacity(records.len());
        let mut roles: Vec<Option<serde_json::Value>> = Vec::with_capacity(records.len());
        let mut origins: Vec<Option<serde_json::Value>> = Vec::with_capacity(records.len());
        let mut origin_counts: Vec<Option<i64>> = Vec::with_capacity(records.len());
        let mut classified = CategoryColumns::with_capacity(records.len());
        let mut actors = ActorColumns::with_capacity(records.len());
        let mut retention = RetentionColumns::with_capacity(records.len());
        let mut subjects = SubjectColumns::with_capacity(records.len());
        for (index, AuditRecord { event, subject }) in records.iter().enumerate() {
            retention.push(ends.get(index));
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
            declared.push(event);
            result_counts.push(result_count(event));
            request_ids.push(event.request_id.as_deref());
            roles.push(roles_json(event));
            origins.push(origins_json(event));
            origin_counts.push(origin_count(event));
            classified.push(event)?;
            actors.push(event);
            subjects.push(event);
        }
        sqlx::query(self.owed(INSERT_BATCH, &INSERT_BATCH_OWED))
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
            .bind(declared.purposes)
            .bind(declared.legal_bases)
            .bind(result_counts)
            .bind(request_ids)
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
            .bind(declared.emergency)
            .bind(retention.ends)
            .bind(retention.stated)
            .bind(declared.permits)
            .bind(subjects.issuers)
            .bind(subjects.clients)
            .bind(subjects.ehr_ids)
            .bind(subjects.namespaces)
            .bind(subjects.lookups_failed)
            .bind(classified.contents)
            .bind(policy)
            .bind(tokens)
            .execute(&mut *conn)
            .await?;
        Ok(())
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

    /// The FHIR R4 `AuditEvent` of a stored row, rendered under the policy it
    /// names.
    ///
    /// # Errors
    /// [`AuditError::Unreadable`] and [`AuditError::UnreadableJson`] when the
    /// row or its policy does not read, [`AuditError::Render`] when the
    /// document cannot be written.
    #[cfg(feature = "fhir")]
    async fn render(&self, row: &PgRow) -> Result<serde_json::Value, AuditError> {
        let (digest, stored) = stored_record(row)?;
        let policy = self.policy(stored.chain_seq, &digest).await?;
        crate::system_log::render::render(stored, &policy)
    }

    /// The FHIR R4 `AuditEvent` of the record at chain position `chain_seq`,
    /// rendered from its columns as the ITI-81 search serves it; `None` when
    /// no record holds that position.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the SELECT fails, and the rendering errors of
    /// a record that cannot be read back.
    #[cfg(feature = "fhir")]
    pub async fn rendered(&self, chain_seq: i64) -> Result<Option<serde_json::Value>, AuditError> {
        use sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder};
        use sea_query_sqlx::SqlxBinder as _;

        let (sql, values) = newest_first(
            sea_query::Cond::all().add(Expr::col(Alias::new("chain_seq")).eq(chain_seq)),
        )
        .build_sqlx(PostgresQueryBuilder);
        let row = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .fetch_optional(&mut *acquire(&self.pool).await?)
            .await?;
        match row {
            Some(row) => self.render(&row).await.map(Some),
            None => Ok(None),
        }
    }

    /// The oldest record the FHIR Feed still owes, past the chain positions in
    /// `skip`, as `(chain_seq, its FHIR document)`, and how many such records
    /// there are. A record whose document cannot be rendered comes back as its
    /// chain position with the error, for the feed to hold aside.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a SELECT fails.
    #[cfg(feature = "fhir")]
    pub async fn oldest_fhir_feed(
        &self,
        skip: &[i64],
    ) -> Result<(Option<(i64, Result<serde_json::Value, AuditError>)>, i64), AuditError> {
        let mut conn = acquire(&self.pool).await?;
        let row = sqlx::query(OLDEST_OWED)
            .bind(skip)
            .fetch_optional(&mut *conn)
            .await?;
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit.fhir_feed_outbox WHERE NOT (chain_seq = ANY($1))",
        )
        .bind(skip)
        .fetch_one(&mut *conn)
        .await?;
        drop(conn);
        let oldest = match row {
            Some(row) => {
                let chain_seq = row.try_get::<i64, _>("chain_seq")?;
                Some((chain_seq, self.render(&row).await))
            }
            None => None,
        };
        Ok((oldest, waiting))
    }

    /// Enters the records written before the store kept an outbox, and never
    /// delivered by the FHIR Feed, in the outbox; the first call does it and
    /// every later one finds nothing to do. Returns how many it entered.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn enqueue_fhir_feed_backlog(&self) -> Result<u64, AuditError> {
        let moved: i64 = sqlx::query_scalar("SELECT audit.enqueue_fhir_feed_backlog()")
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        u64::try_from(moved).map_err(|e| AuditError::Store(sqlx::Error::Decode(Box::new(e))))
    }

    /// Stamps the record at chain position `chain_seq` as delivered by the
    /// FHIR Feed and removes it from the outbox, in one statement.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails; the record is then
    /// delivered again, which the repository takes as a second copy.
    pub async fn mark_fhir_feed_delivered(&self, chain_seq: i64) -> Result<(), AuditError> {
        sqlx::query(
            "WITH delivered AS (DELETE FROM audit.fhir_feed_outbox WHERE chain_seq = $1) \
             UPDATE audit.audit_event SET delivered_fhir_feed_at = now() WHERE chain_seq = $1",
        )
        .bind(chain_seq)
        .execute(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(())
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
        findings(rows)
    }

    /// Check the repository's tamper evidence over `scope`, and move the
    /// verification checkpoint to the newest record when the run finds nothing.
    ///
    /// [`VerifyScope::SinceCheckpoint`] recomputes only the records written
    /// after the newest link an earlier run found intact, so its cost follows
    /// the records written since rather than the size of the trail; a change to
    /// a record before that link needs [`VerifyScope::Full`], the pass
    /// [`Self::verify_chain`] makes. Both check the chain head. The run is
    /// `audit.verify_audit_chain_incremental`.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the verification query cannot run, which is
    /// itself a finding, never a pass.
    pub async fn verify_chain_advancing(
        &self,
        scope: VerifyScope,
    ) -> Result<Vec<AuditChainFinding>, AuditError> {
        let rows = sqlx::query(
            "SELECT chain_seq, record_id, recorded_at, finding \
             FROM audit.verify_audit_chain_incremental($1)",
        )
        .bind(scope == VerifyScope::Full)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        findings(rows)
    }

    /// The RESTful-ATNA ITI-81 retrieval: the FHIR `AuditEvent` documents of
    /// the matching records, newest first, rendered from their columns, plus
    /// the total match count. The filter is the supported ITI-81
    /// search-parameter subset ([`AuditSearchFilter`]).
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
    pub async fn search(
        &self,
        filter: &AuditSearchFilter,
    ) -> Result<(i64, Vec<serde_json::Value>), AuditError> {
        let Some(resolved) =
            crate::system_log::search::resolve_criteria(&self.pool, filter).await?
        else {
            return Ok((0, Vec::new()));
        };
        let (count_sql, count_values, sql, values) = build_search_statements(filter, &resolved);
        // NOTE: an unnamed statement is planned with the token ids it binds (PostgreSQL 18,
        // "PREPARE", plan_cache_mode), so a rare token reaches its index rather than a generic plan.
        let total: i64 = sqlx::query_scalar_with(sqlx::AssertSqlSafe(count_sql), count_values)
            .persistent(false)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        let rows = sqlx::query_with(sqlx::AssertSqlSafe(sql), values)
            .persistent(false)
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        let mut documents = Vec::with_capacity(rows.len());
        for row in &rows {
            documents.push(self.render(row).await?);
        }
        Ok((total, documents))
    }
}

/// Build the ITI-81 retrieval's two statements — the count and the page — from a
/// filter and the token ids its criteria on the document resolved to, with no
/// database involved.
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
#[cfg(feature = "fhir")]
fn build_search_statements(
    filter: &AuditSearchFilter,
    resolved: &[Option<crate::system_log::search::TokenMatch>],
) -> (
    String,
    sea_query_sqlx::SqlxValues,
    String,
    sea_query_sqlx::SqlxValues,
) {
    use sea_query::{Alias, Expr, ExprTrait, PostgresQueryBuilder, Query};
    use sea_query_sqlx::SqlxBinder as _;

    let table = (Alias::new("audit"), Alias::new("audit_event"));
    let condition = crate::system_log::search::condition::search_condition(filter, resolved);

    let (count_sql, count_values) = Query::select()
        .expr(Expr::col(sea_query::Asterisk).count())
        .from(table)
        .cond_where(condition.clone())
        .build_sqlx(PostgresQueryBuilder);

    let (sql, values) = newest_first(condition)
        .limit(u64::try_from(filter.count.clamp(1, 1000)).unwrap_or(50))
        .offset(u64::try_from(Ord::max(filter.offset, 0)).unwrap_or(0))
        .build_sqlx(PostgresQueryBuilder);

    (count_sql, count_values, sql, values)
}

/// The record columns of the records `condition` selects, newest first.
#[cfg(feature = "fhir")]
pub(super) fn newest_first(condition: sea_query::Cond) -> sea_query::SelectStatement {
    use sea_query::{Alias, Order, Query};

    Query::select()
        .columns(RECORD_COLUMNS.map(Alias::new))
        .from((Alias::new("audit"), Alias::new("audit_event")))
        .cond_where(condition)
        .order_by(Alias::new("recorded_at"), Order::Desc)
        .order_by(Alias::new("stored_at"), Order::Desc)
        .order_by(Alias::new("chain_seq"), Order::Desc)
        .to_owned()
}

/// The findings a verification query returned, in its order.
fn findings(rows: Vec<PgRow>) -> Result<Vec<AuditChainFinding>, AuditError> {
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

/// How much of the hash chain one verification recomputes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyScope {
    /// Every record, from the start of the chain.
    Full,
    /// The records written after the verification checkpoint.
    SinceCheckpoint,
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
const OBJECT_ROLE_SYSTEM: &str = "http://terminology.hl7.org/CodeSystem/object-role";

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

/// The items `steps` reaches from `from`, read as an SQL/JSON path in lax mode
/// reads them: an array is unwrapped before each member step and once at the
/// end, and a missing member yields nothing (ISO/IEC 9075-2 SQL/JSON path, as
/// PostgreSQL 18 documents it in "The SQL/JSON Path Language",
/// <https://www.postgresql.org/docs/18/functions-json.html#FUNCTIONS-SQLJSON-PATH>).
pub(crate) fn lax_path<'a>(
    from: &'a serde_json::Value,
    steps: &[&str],
) -> Vec<&'a serde_json::Value> {
    let unwrap = |items: Vec<&'a serde_json::Value>| -> Vec<&'a serde_json::Value> {
        items
            .into_iter()
            .flat_map(|item| match item {
                serde_json::Value::Array(elements) => elements.iter().collect::<Vec<_>>(),
                other => vec![other],
            })
            .collect()
    };
    let mut items = vec![from];
    for step in steps {
        items = unwrap(items)
            .into_iter()
            .filter_map(|item| item.get(*step))
            .collect();
    }
    unwrap(items)
}

/// Whether any item `steps` reaches from `from` is `value`.
fn any_is(from: &serde_json::Value, steps: &[&str], value: &serde_json::Value) -> bool {
    lax_path(from, steps).into_iter().any(|item| item == value)
}

/// Whether any item `steps` reaches from `from` is the string `text`.
fn any_equals(from: &serde_json::Value, steps: &[&str], text: &str) -> bool {
    lax_path(from, steps)
        .into_iter()
        .any(|item| item.as_str() == Some(text))
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

/// The columns of a batch that say on whose authority and why: the declared
/// purpose, the deployment's legal basis, the declared data permit and the
/// emergency mark.
struct DeclaredColumns<'a> {
    purposes: Vec<Option<&'a str>>,
    legal_bases: Vec<Option<&'a str>>,
    permits: Vec<Option<&'a str>>,
    emergency: Vec<bool>,
}

impl<'a> DeclaredColumns<'a> {
    fn with_capacity(n: usize) -> Self {
        Self {
            purposes: Vec::with_capacity(n),
            legal_bases: Vec::with_capacity(n),
            permits: Vec::with_capacity(n),
            emergency: Vec::with_capacity(n),
        }
    }

    fn push(&mut self, event: &'a AuditEvent) {
        self.purposes.push(event.purpose.as_deref());
        self.legal_bases.push(event.legal_basis.as_deref());
        self.permits.push(event.permit.as_deref());
        self.emergency.push(event.emergency_access);
    }
}

/// The retention columns of a batch: each record's end as a `YYYY-MM-DD` text
/// the statement casts to `date`, and whether the record states one.
struct RetentionColumns {
    ends: Vec<Option<String>>,
    stated: Vec<bool>,
}

impl RetentionColumns {
    fn with_capacity(n: usize) -> Self {
        Self {
            ends: Vec::with_capacity(n),
            stated: Vec::with_capacity(n),
        }
    }

    fn push(&mut self, end: Option<&RecordEnd>) {
        self.ends.push(match end {
            Some(RecordEnd::On(date)) => Some(date.to_string()),
            Some(RecordEnd::Never) | None => None,
        });
        self.stated.push(end.is_some());
    }
}

/// The four classification columns of a batched insert, one entry per record.
struct CategoryColumns {
    categories: Vec<Option<serde_json::Value>>,
    bases: Vec<Option<&'static str>>,
    evidence: Vec<Option<serde_json::Value>>,
    digests: Vec<Option<String>>,
    contents: Vec<Option<serde_json::Value>>,
}

impl CategoryColumns {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            categories: Vec::with_capacity(capacity),
            bases: Vec::with_capacity(capacity),
            evidence: Vec::with_capacity(capacity),
            digests: Vec::with_capacity(capacity),
            contents: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, event: &AuditEvent) -> Result<(), AuditError> {
        self.categories.push(categories_json(event));
        self.bases.push(category_basis(event));
        self.evidence.push(category_evidence_json(event));
        self.digests.push(category_map_digest(event));
        self.contents.push(category_content_json(event)?);
        Ok(())
    }
}

/// The columns of a batch that name who the caller and the subject are beyond
/// the principal and the patient: the token's issuer and client, the EHR, the
/// subject's namespace and whether its lookup failed.
struct SubjectColumns<'a> {
    issuers: Vec<Option<&'a str>>,
    clients: Vec<Option<&'a str>>,
    ehr_ids: Vec<Option<&'a str>>,
    namespaces: Vec<Option<&'a str>>,
    lookups_failed: Vec<bool>,
}

impl<'a> SubjectColumns<'a> {
    fn with_capacity(n: usize) -> Self {
        Self {
            issuers: Vec::with_capacity(n),
            clients: Vec::with_capacity(n),
            ehr_ids: Vec::with_capacity(n),
            namespaces: Vec::with_capacity(n),
            lookups_failed: Vec::with_capacity(n),
        }
    }

    fn push(&mut self, event: &'a AuditEvent) {
        self.issuers.push(event.issuer.as_deref());
        self.clients.push(event.client_id.as_deref());
        self.ehr_ids.push(access_ehr_id(event));
        self.namespaces.push(event.subject_namespace.as_deref());
        self.lookups_failed.push(event.subject_lookup_failed);
    }
}

/// The `ehr_id` column value: the EHR a classified access reached, which its
/// EHDS access record names; `NULL` for every other record, whose document
/// names no EHR.
///
/// NOTE: no specification governs this: our own design; a linkage record names a party, and
/// the EHR beside it would make the audit store a second copy of the linkage map.
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
    event.organisation = row.try_get("organisation")?;
    event.roles = strings(row, "roles", chain_seq)?;
    event.client_ip = row.try_get("client_ip")?;
    event.ehr_id = row.try_get("ehr_id")?;
    event.subject_namespace = row.try_get("subject_namespace")?;
    event.subject_lookup_failed = row.try_get("subject_lookup_failed")?;
    event.object_id = row.try_get("resource_id")?;
    event.token_id = row.try_get("token_id")?;
    event.domain = access_domain(row.try_get("domain")?)
        .ok_or_else(|| unreadable("its domain is none this build writes"))?;
    event.purpose = row.try_get("purpose")?;
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
fn object_class(stored: &str) -> Option<ObjectClass> {
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
fn access_domain(stored: Option<&str>) -> Option<AccessDomain> {
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
mod tests {
    use super::{
        AuditCriterion, AuditElement, AuditSearchFilter, AuditToken, AuditTokenSystem,
        RECORD_COLUMNS, build_search_statements,
    };

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

    /// The token ids the criteria of [`filter_with`] resolve to: none, so
    /// each is read as its column condition.
    const RESOLVED: [Option<crate::system_log::search::TokenMatch>; 0] = [];

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
        let (baseline_count, _, baseline_page, _) =
            build_search_statements(&filter_with("benign"), &RESOLVED);

        for payload in HOSTILE {
            // The page statement's own values are not inspected: both statements
            // bind from the same condition, so asserting the count's bindings
            // covers both, and the page text is compared byte for byte below.
            let (count_sql, count_values, sql, _page_values) =
                build_search_statements(&filter_with(payload), &RESOLVED);
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
        let (count_sql, _, sql, _) = build_search_statements(
            &AuditSearchFilter {
                criteria: vec![
                    AuditCriterion::Patient(vec!["p".to_owned()]),
                    AuditCriterion::Agent(vec!["a".to_owned()]),
                    AuditCriterion::Entity(vec!["e".to_owned()]),
                    AuditCriterion::Action(vec!["x".to_owned()]),
                    AuditCriterion::Outcome(vec![0]),
                ],
                ..AuditSearchFilter::default()
            },
            &[],
        );

        let mut seen: Vec<String> = format!("{count_sql} {sql}")
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        seen.sort_unstable();
        seen.dedup();

        let mut expected: Vec<&str> = RECORD_COLUMNS.to_vec();
        expected.extend(["audit", "audit_event"]);
        expected.sort_unstable();
        expected.dedup();
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
        let (_, _, sql, values) = build_search_statements(
            &AuditSearchFilter {
                count: i64::MAX,
                offset: i64::MIN,
                ..AuditSearchFilter::default()
            },
            &[],
        );
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

    fn token(system: AuditTokenSystem, code: Option<&str>) -> AuditToken {
        AuditToken {
            system,
            code: code.map(str::to_owned),
        }
    }

    /// The FHIR Feed reads a record back by the same columns the search does.
    #[test]
    fn the_feed_reads_every_record_column() {
        let selected: Vec<&str> = super::OLDEST_OWED
            .split(" FROM ")
            .next()
            .unwrap_or_default()
            .trim_start_matches("SELECT")
            .split(',')
            .map(|column| column.trim().trim_start_matches("e."))
            .collect();
        assert_eq!(selected, RECORD_COLUMNS);
    }

    /// A rendered document with an agent, the patient and an object entity.
    fn document() -> serde_json::Value {
        serde_json::json!({
            "type": {"system": "http://dicom.nema.org/resources/ontology/DCM", "code": "110110"},
            "subtype": [{"code": "composition_get"}],
            "agent": [
                {"who": {"identifier": {"value": "alice"}}, "network": {"address": "10.0.0.9"}},
                {"who": {"identifier": {"system": "urn:sys", "value": "ferroehr"}}}
            ],
            "entity": [
                {
                    "role": {"system": "http://terminology.hl7.org/CodeSystem/object-role", "code": "1"},
                    "what": {"identifier": {"value": "patient-42"}}
                },
                {"what": {"identifier": {"value": "8fa1::ferroehr::1"}}}
            ]
        })
    }

    /// The three token forms of FHIR R4 Search §3.1.1.4.10 match an
    /// `Identifier` by its `value` and a `Coding` by its `code`, as the stored
    /// document's SQL/JSON path did.
    #[test]
    fn a_token_matches_the_document_element_it_names() {
        let document = document();
        let element = |element, tokens| AuditCriterion::Element(element, tokens);
        let any = |code| token(AuditTokenSystem::Any, Some(code));
        let cases = [
            (
                element(AuditElement::AgentIdentifier, vec![any("alice")]),
                true,
            ),
            (
                element(
                    AuditElement::AgentIdentifier,
                    vec![token(AuditTokenSystem::Absent, Some("alice"))],
                ),
                true,
            ),
            (
                element(
                    AuditElement::AgentIdentifier,
                    vec![token(AuditTokenSystem::Absent, Some("ferroehr"))],
                ),
                false,
            ),
            (
                element(
                    AuditElement::AgentIdentifier,
                    vec![token(
                        AuditTokenSystem::Named("urn:sys".to_owned()),
                        Some("ferroehr"),
                    )],
                ),
                true,
            ),
            (
                element(
                    AuditElement::AgentIdentifier,
                    vec![token(AuditTokenSystem::Named("urn:sys".to_owned()), None)],
                ),
                true,
            ),
            (
                element(
                    AuditElement::AgentIdentifier,
                    vec![token(
                        AuditTokenSystem::Named("urn:other".to_owned()),
                        Some("ferroehr"),
                    )],
                ),
                false,
            ),
            (
                element(AuditElement::PatientIdentifier, vec![any("patient-42")]),
                true,
            ),
            (
                element(
                    AuditElement::PatientIdentifier,
                    vec![any("8fa1::ferroehr::1")],
                ),
                false,
            ),
            (
                element(
                    AuditElement::EntityIdentifier,
                    vec![any("8fa1::ferroehr::1")],
                ),
                true,
            ),
            (element(AuditElement::Type, vec![any("110110")]), true),
            (
                element(
                    AuditElement::Subtype,
                    vec![any("composition_get"), any("nothing")],
                ),
                true,
            ),
            (element(AuditElement::Subtype, vec![any("nothing")]), false),
            (
                element(AuditElement::SourceIdentifier, vec![any("ferroehr")]),
                false,
            ),
            (AuditCriterion::Address(vec!["0.0.9".to_owned()]), true),
            (AuditCriterion::Address(vec!["10.1".to_owned()]), false),
        ];
        for (criterion, expected) in cases {
            assert!(criterion.reads_document(), "{criterion:?}");
            assert_eq!(criterion.matches(&document), expected, "{criterion:?}");
        }
    }

    /// A criterion on the document becomes a condition on `search_tokens`
    /// with the token ids it resolved to, bound as one value, and one with no
    /// resolved ids matches nothing.
    #[test]
    fn a_resolved_token_criterion_reads_the_token_index() {
        let filter = AuditSearchFilter {
            criteria: vec![
                AuditCriterion::Patient(vec!["patient-42".to_owned()]),
                AuditCriterion::Element(
                    AuditElement::PatientIdentifier,
                    vec![token(AuditTokenSystem::Any, Some("patient-42"))],
                ),
            ],
            ..AuditSearchFilter::default()
        };
        let (count_sql, values, sql, _) = build_search_statements(
            &filter,
            &[
                None,
                Some(crate::system_log::search::TokenMatch::from(vec![3, 17])),
            ],
        );
        assert!(sql.contains("search_tokens && CAST($"), "{sql}");
        assert!(count_sql.contains("search_tokens && CAST($"), "{count_sql}");
        assert!(format!("{values:?}").contains("{3,17}"), "{values:?}");
        let (_, _, unresolved, _) = build_search_statements(&filter, &[]);
        assert!(unresolved.contains("false"), "{unresolved}");
    }
}
