// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The intake of the access log: each entry written once, unchained, before
//! the response that reports its access (#5747, #5968).
//!
//! Each entry is written with every column the chain keeps, and one intake
//! row names the entries of the statement for the chainer.
//!
//! The writer supplies the row and shared parts of the entry's canonical
//! form, the content digest over the whole form, the columns the log derives
//! from the content and, when the entry is indexed, its ITI-81 index row, so
//! the chainer only computes the links
//! (`migrations/audit/0006_baseline_intake.sql`). The forms are written
//! straight from the entry, with no JSON value of it built (#5849). Each
//! derived column is read from the entry's ITS-JSON form at the path the
//! database's own derive reads (`audit.access_log_entry_derive`), and the
//! text is cast by the database as that function casts it, so a chained
//! record compares equal to its derivation, which the verifier checks. No
//! openEHR spec governs audit storage: our own design.

use std::collections::{BTreeMap, BTreeSet};

use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::invariants::check_entry;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_object_without;
use sqlx::PgConnection;

use crate::access_log::store::AccessLogError;
use crate::access_log::stored_form;
use crate::access_log::stored_form::parts::Derived;
use crate::digest::sha256_hex;
use crate::system_log::token_digest::token_digest;
use gather::{Job, Known, QUEUE, WRITERS, gather};
/// The ITI-81 index row of an entry: what the search filters on and the
/// render policy its projection is written under.
#[derive(Debug, Clone)]
pub struct IndexRow {
    /// The DICOM `EventActionCode`.
    pub action: &'static str,
    /// The DICOM `EventOutcomeIndicator`.
    pub outcome: i16,
    /// The caller, when one is named.
    pub principal: Option<String>,
    /// The subject of care of the entry's EHR.
    pub patient_id: Option<String>,
    /// The object the access reached.
    pub resource_id: Option<String>,
    /// The request the access belongs to.
    pub request_id: Option<String>,
    /// The caller's network address.
    pub client_ip: Option<String>,
    /// The EHR of a classified access.
    pub ehr_id: Option<String>,
    /// The ids of the search tokens of the projection, as the text of an
    /// `integer[]`.
    pub search_tokens: String,
    /// The digest of the render policy.
    pub render_policy: Vec<u8>,
}

/// An entry as the intake keeps it.
#[derive(Debug)]
pub struct IntakeEntry {
    row: String,
    shared: Option<(String, String)>,
    content_digest: String,
    derived: Derived,
    index: Option<IndexRow>,
}

impl IntakeEntry {
    /// `entry` as the intake keeps it, with `index` when it is indexed.
    ///
    /// The entry is checked against the draft's invariants: an entry that
    /// breaks one is never written.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when the entry has no canonical form,
    /// and [`AccessLogError::Invalid`] when it breaks an invariant.
    pub fn of(entry: &mut AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        check_entry(entry)?;
        Self::stored(entry, index)
    }

    /// The stored parts of `entry`, written straight from it (#5849).
    fn stored(entry: &AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        let whole = canonical_object_without(entry, &[]).map_err(AccessLogError::Canonical)?;
        let cut = stored_form::parts::Cut::of(&whole).map_err(AccessLogError::Canonical)?;
        let (row, shared) = cut.parts().map_err(AccessLogError::Canonical)?;
        let columns = stored_form::parts::index::IndexColumns {
            request_id: index.as_ref().and_then(|index| index.request_id.as_deref()),
            resource_id: index
                .as_ref()
                .and_then(|index| index.resource_id.as_deref()),
        };
        let row = stored_form::parts::index::without_columns(&row, &columns)
            .map_err(AccessLogError::Canonical)?;
        let derived = Derived::of(&cut).map_err(AccessLogError::Canonical)?;
        let shared = shared.map(|text| (sha256_hex(&text), text));
        let columns = row_columns(&derived, index.as_ref())?;
        let digest = stored_form::parts::facts_digest(
            stored_form::parts::FORM,
            &row,
            shared.as_ref().map(|(digest, _)| digest.as_str()),
            &columns,
        )
        .map_err(AccessLogError::Canonical)?;
        Ok(Self {
            content_digest: crate::digest::hex(&digest),
            row,
            shared,
            derived,
            index,
        })
    }
}

/// The columns a reader joins the stored parts of an entry with, as the
/// database keeps them from `derived` and `index`.
fn row_columns<'d>(
    derived: &'d Derived,
    index: Option<&'d IndexRow>,
) -> Result<stored_form::parts::RowColumns<'d>, AccessLogError> {
    let unreadable =
        |what: &'static str| AccessLogError::Canonical(serde::ser::Error::custom(what));
    Ok(stored_form::parts::RowColumns {
        record_type: derived
            .record_type
            .as_deref()
            .ok_or_else(|| unreadable("an entry names its class"))?,
        uid: derived
            .uid
            .as_deref()
            .and_then(|uid| uuid::Uuid::parse_str(uid).ok())
            .ok_or_else(|| unreadable("an entry's uid is a UUID"))?,
        time_recorded: derived
            .time_recorded
            .as_deref()
            .and_then(|at| at.parse().ok())
            .ok_or_else(|| unreadable("an entry's time_recorded is an instant"))?,
        ehr_id: derived.ehr_id.as_deref().and_then(stored_form::column_uuid),
        index: stored_form::parts::index::IndexColumns {
            request_id: index.and_then(|index| index.request_id.as_deref()),
            resource_id: index.and_then(|index| index.resource_id.as_deref()),
        },
    })
}

#[cfg(test)]
impl IntakeEntry {
    /// The canonical form a reader joins back from `entry`'s stored parts
    /// and columns, stored with `index`.
    pub(crate) fn read_back(
        entry: &AccessLogEntry,
        index: IndexRow,
    ) -> Result<String, AccessLogError> {
        let stored = Self::stored(entry, Some(index))?;
        let columns = row_columns(&stored.derived, stored.index.as_ref())?;
        let shared = stored.shared.as_ref().map(|(_, text)| text.as_str());
        stored_form::parts::whole(&stored.row, shared, &columns).map_err(AccessLogError::Canonical)
    }
}

/// The intake rows of a batch, one element per entry, as the `unnest` of
/// [`INTAKE`] reads them, borrowed from the entries.
#[derive(Debug, Default)]
struct Columns<'e> {
    record_types: Vec<Option<&'e str>>,
    uids: Vec<Option<&'e str>>,
    times: Vec<Option<&'e str>>,
    contents: Vec<&'e str>,
    shared: Vec<Option<&'e str>>,
    digests: Vec<&'e str>,
    ehr_ids: Vec<Option<&'e str>>,
    party_ids: Vec<Option<&'e str>>,
    retain_until: Vec<Option<&'e str>>,
    action_terminologies: Vec<Option<&'e str>>,
    action_codes: Vec<Option<&'e str>>,
    result_terminologies: Vec<Option<&'e str>>,
    result_codes: Vec<Option<&'e str>>,
    actions: Vec<Option<&'static str>>,
    outcomes: Vec<Option<i16>>,
    principals: Vec<Option<&'e str>>,
    patient_ids: Vec<Option<&'e str>>,
    resource_ids: Vec<Option<&'e str>>,
    request_ids: Vec<Option<&'e str>>,
    client_ips: Vec<Option<&'e str>>,
    index_ehr_ids: Vec<Option<&'e str>>,
    tokens: Vec<Option<&'e str>>,
    policies: Vec<Option<&'e [u8]>>,
}

impl<'e> Columns<'e> {
    /// `query` with the rows bound as its first 23 parameters, in the order
    /// both inserting statements read them.
    fn bound(
        self,
        query: sqlx::query::Query<'e, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'e, sqlx::Postgres, sqlx::postgres::PgArguments> {
        query
            .bind(self.record_types)
            .bind(self.uids)
            .bind(self.times)
            .bind(self.contents)
            .bind(self.shared)
            .bind(self.digests)
            .bind(self.ehr_ids)
            .bind(self.party_ids)
            .bind(self.retain_until)
            .bind(self.action_terminologies)
            .bind(self.action_codes)
            .bind(self.result_terminologies)
            .bind(self.result_codes)
            .bind(self.actions)
            .bind(self.outcomes)
            .bind(self.principals)
            .bind(self.patient_ids)
            .bind(self.resource_ids)
            .bind(self.request_ids)
            .bind(self.client_ips)
            .bind(self.index_ehr_ids)
            .bind(self.tokens)
            .bind(self.policies)
    }

    /// Adds `entry`'s row, its search tokens named by `ids` when the writer
    /// named them by id.
    fn push(&mut self, entry: &'e IntakeEntry, ids: Option<&'e str>) {
        let derived = &entry.derived;
        self.record_types.push(derived.record_type.as_deref());
        self.uids.push(derived.uid.as_deref());
        self.times.push(derived.time_recorded.as_deref());
        self.contents.push(&entry.row);
        self.shared
            .push(entry.shared.as_ref().map(|(digest, _)| digest.as_str()));
        self.digests.push(&entry.content_digest);
        self.ehr_ids.push(derived.ehr_id.as_deref());
        self.party_ids.push(derived.party_id.as_deref());
        self.retain_until.push(derived.retain_until.as_deref());
        self.action_terminologies
            .push(derived.action_terminology.as_deref());
        self.action_codes.push(derived.action_code.as_deref());
        self.result_terminologies
            .push(derived.result_terminology.as_deref());
        self.result_codes.push(derived.result_code.as_deref());
        let index = entry.index.as_ref();
        self.actions.push(index.map(|index| index.action));
        self.outcomes.push(index.map(|index| index.outcome));
        self.principals
            .push(index.and_then(|index| index.principal.as_deref()));
        self.patient_ids
            .push(index.and_then(|index| index.patient_id.as_deref()));
        self.resource_ids
            .push(index.and_then(|index| index.resource_id.as_deref()));
        self.request_ids
            .push(index.and_then(|index| index.request_id.as_deref()));
        self.client_ips
            .push(index.and_then(|index| index.client_ip.as_deref()));
        self.index_ehr_ids
            .push(index.and_then(|index| index.ehr_id.as_deref()));
        self.tokens
            .push(index.map(|index| ids.unwrap_or(index.search_tokens.as_str())));
        self.policies
            .push(index.map(|index| index.render_policy.as_slice()));
    }
}

/// One search token of an entry's index row, which the intake statement
/// stores or finds: its ITI-81 parameter, system and code, and whether it is
/// kept in the entry's shared set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntakeToken {
    /// The ITI-81 parameter.
    pub parameter: &'static str,
    /// The system of the coding or identifier.
    pub system: Option<String>,
    /// The code, the identifier value or the address.
    pub code: Option<String>,
    /// Whether the token belongs to the entry's shared set.
    pub shares: bool,
}

/// The intake statement: the search tokens and shared sets, the shared parts
/// and the rows of a batch (`audit.access_log_intake_take`).
const INTAKE: &str = "SELECT audit.access_log_intake_take($1, $2, $3, $4, $5, $6, $7, $8, $9, \
     $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, \
     $28, $29, $30, $31, $32, $33)";

/// The plain write of a batch whose every id the writer names from memory:
/// each entry's search tokens and shared set by id under the generation
/// `$24`, which the statement holds and checks with no function call while
/// it is the store's (the function that refuses it runs only when it is
/// not), its shared part one the store already holds or one the batch
/// carries (`$26`, `$27`), stored or raised in the same statement, and its
/// render policy by digest; held under `$25` when it names a reservation
/// (#6242). It is the intake statement's own insert, with no function around
/// it: a part an entry outlives is raised under its row lock, and a part or
/// policy the store does not hold, or a part removed while it was raised, is
/// refused as that statement refuses it.
const PLAIN: &str = "WITH own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken), \
     hold AS MATERIALIZED ( \
       SELECT pg_catalog.pg_advisory_xact_lock_shared( \
                  pg_catalog.hashtext('audit.audit_search_token'), 0)::text AS held \
        WHERE $24::bigint IS NOT NULL), \
     g AS MATERIALIZED ( \
       SELECT CASE WHEN $24::bigint IS NULL \
                     OR $24::bigint = (SELECT t.generation \
                                         FROM audit.audit_search_token_generation t \
                                        WHERE t.singleton) \
                   THEN true \
                   ELSE audit.access_log_token_generation_holds($24) END AS holds \
         FROM (SELECT count(*) FROM hold) AS taken), \
     u AS MATERIALIZED ( \
       SELECT * FROM unnest($1::text[], $2::text[], $3::text[], $4::text[], $5::text[], \
                            $6::text[], $7::text[], $8::text[], $9::text[], $10::text[], \
                            $11::text[], $12::text[], $13::text[], $14::text[], $15::smallint[], \
                            $16::text[], $17::text[], $18::text[], $19::text[], $20::text[], \
                            $21::text[], $22::text[], $23::bytea[]) WITH ORDINALITY \
         AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, \
              c19, c20, c21, c22, c23, n)), \
     held AS ( \
       SELECT s.digest, s.retain_until FROM audit.access_log_shared s \
        WHERE s.digest IN (SELECT DISTINCT decode(u.c5, 'hex') FROM u WHERE u.c5 IS NOT NULL)), \
     needed AS ( \
       SELECT decode(u.c5, 'hex') AS digest, \
              greatest(max(coalesce(u.c9::date, 'infinity'::date)), current_date) AS until \
         FROM u WHERE u.c5 IS NOT NULL GROUP BY u.c5), \
     batch AS (SELECT decode(b.digest, 'hex') AS digest, b.content \
                 FROM unnest($26::text[], $27::text[]) AS b(digest, content)), \
     carried AS ( \
       INSERT INTO audit.access_log_shared (digest, content, retain_until) \
       SELECT b.digest, b.content, n.until FROM batch b JOIN needed n ON n.digest = b.digest \
        ORDER BY b.digest \
       ON CONFLICT (digest) DO UPDATE SET retain_until = excluded.retain_until \
        WHERE audit.access_log_shared.retain_until < excluded.retain_until \
       RETURNING digest), \
     kept AS ( \
       UPDATE audit.access_log_shared s SET retain_until = n.until \
         FROM needed n JOIN held h ON h.digest = n.digest \
        WHERE s.digest = n.digest AND s.retain_until < n.until \
          AND n.digest NOT IN (SELECT b.digest FROM batch b) \
       RETURNING s.digest), \
     written AS ( \
       INSERT INTO audit.access_log_entry (record_type, uid, time_recorded, content, shared, \
         content_digest, ehr_id, party_id, retain_until, action_terminology, action_code, \
         result_terminology, result_code, event_action, event_outcome, principal, patient_id, \
         resource_id, request_id, client_ip, classified_ehr_id, search_tokens, render_policy_id) \
       SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4, \
              CASE WHEN u.c5 IS NOT NULL \
                        AND inbatch.digest IS NULL \
                        AND (held.digest IS NULL \
                             OR (held.retain_until < need.until AND raised.digest IS NULL \
                                 AND audit.access_log_intake_part_gone(u.c5))) \
                   THEN audit.access_log_intake_unknown('shared part', u.c5, \
                            held.digest IS NOT NULL) \
                   ELSE decode(u.c5, 'hex') END, \
              decode(u.c6, 'hex'), audit.access_log_canonical_uuid(u.c7), u.c8, u.c9::date, \
              u.c10, u.c11, u.c12, u.c13, u.c14, u.c15, u.c16, u.c17, u.c18, u.c19, u.c20, u.c21, \
              CASE WHEN u.c22 = '' THEN '{}'::integer[] ELSE u.c22::integer[] END, \
              CASE WHEN u.c23 IS NOT NULL AND policy.digest IS NULL \
                   THEN octet_length(audit.access_log_intake_unknown( \
                            'render policy', encode(u.c23, 'hex'), false)) \
                   ELSE policy.id END \
         FROM own_xid CROSS JOIN g CROSS JOIN u \
         LEFT JOIN held ON held.digest = decode(u.c5, 'hex') \
         LEFT JOIN batch inbatch ON inbatch.digest = decode(u.c5, 'hex') \
         LEFT JOIN needed need ON need.digest = decode(u.c5, 'hex') \
         LEFT JOIN kept raised ON raised.digest = decode(u.c5, 'hex') \
         LEFT JOIN audit.audit_render_policy policy ON policy.digest = u.c23 \
        WHERE g.holds \
        ORDER BY u.n \
       RETURNING id) \
     INSERT INTO audit.access_log_hold (entry_id, reserved_by) \
     SELECT w.id, $25::uuid FROM written w WHERE $25::uuid IS NOT NULL ORDER BY w.id";

/// Write `entries` into the intake on `conn`, in their order, in one
/// statement.
///
/// The shared parts they name go in with them, but for those whose digest
/// `stored` names, which the store already keeps for at least as long as the
/// entries need: the statement still raises a part's `retain_until` when an
/// entry outlives it, and refuses an entry whose part the store does not hold
/// or whose part it had to raise and found removed. `tokens` holds, by the
/// position of its entry, the search tokens of each entry whose index row
/// leaves them to the statement (an empty `search_tokens`); the statement
/// stores or finds them and their shared set. A batch that leaves the
/// statement no token to resolve goes in by one plain insert with no
/// function around it, which stores the parts it carries too. They are held
/// under `reserved_by` when it names a reservation (#5762). An entry whose search
/// tokens its index row names by id holds them under `generation`, which the
/// statement checks against the store's (#6242). On a connection
/// outside a transaction the statement commits on its own.
///
/// # Errors
/// The store errors when the statement fails.
pub async fn take_in(
    conn: &mut PgConnection,
    entries: &[IntakeEntry],
    tokens: &[Vec<IntakeToken>],
    reserved_by: Option<uuid::Uuid>,
    stored: &BTreeSet<&str>,
    named: &Named,
) -> Result<(), AccessLogError> {
    if entries.is_empty() {
        return Ok(());
    }
    // NOTE: our own design (#5881), the shared parts go in ordered by digest, so
    // concurrent statements that share some take their row locks in one order.
    let mut shared = BTreeMap::new();
    let mut columns = Columns::default();
    for (position, entry) in entries.iter().enumerate() {
        if let Some((digest, content)) = &entry.shared
            && !stored.contains(digest.as_str())
        {
            shared.insert(digest.as_str(), content.as_str());
        }
        columns.push(entry, named.of(position));
    }
    let (keys, texts): (Vec<&str>, Vec<&str>) = shared.into_iter().unzip();
    let mut token_entries = Vec::new();
    let mut parameters = Vec::new();
    let mut systems = Vec::new();
    let mut codes = Vec::new();
    let mut in_sets = Vec::new();
    let mut token_digests = Vec::new();
    for ((position, entry_tokens), at) in (1_i32..).zip(tokens).zip(0_usize..) {
        if named.of(at).is_some() {
            continue;
        }
        for token in entry_tokens {
            token_entries.push(position);
            parameters.push(token.parameter);
            systems.push(token.system.as_deref());
            codes.push(token.code.as_deref());
            in_sets.push(token.shares);
            token_digests.push(token_digest(
                token.parameter,
                token.system.as_deref(),
                token.code.as_deref(),
            ));
        }
    }
    let c = columns;
    if token_entries.is_empty() {
        return write_plain(conn, c, (&keys, &texts), named, reserved_by).await;
    }
    c.bound(sqlx::query(INTAKE))
        .bind(keys)
        .bind(texts)
        .bind(token_entries)
        .bind(parameters)
        .bind(systems)
        .bind(codes)
        .bind(in_sets)
        .bind(reserved_by)
        .bind(
            token_digests
                .iter()
                .map(<[u8; 16]>::as_slice)
                .collect::<Vec<_>>(),
        )
        .bind(named.generation)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Write the batch `c` by the plain insert ([`PLAIN`]), with the shared parts
/// it carries as digests and contents, its token ids named under `named`'s
/// generation and held under `reserved_by`.
async fn write_plain(
    conn: &mut PgConnection,
    c: Columns<'_>,
    (keys, texts): (&[&str], &[&str]),
    named: &Named,
    reserved_by: Option<uuid::Uuid>,
) -> Result<(), AccessLogError> {
    c.bound(sqlx::query(PLAIN))
        .bind(named.generation)
        .bind(reserved_by)
        .bind(keys)
        .bind(texts)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The search tokens of a batch's entries named by id.
///
/// It holds one text of an `integer[]` per entry the writer named, and the
/// generation the ids hold under; an entry it did not name leaves its tokens
/// to the statement.
#[derive(Debug, Default)]
pub struct Named {
    generation: Option<i64>,
    search_tokens: Vec<Option<String>>,
}

impl Named {
    /// The text naming entry `position`'s tokens, when the writer named them.
    fn of(&self, position: usize) -> Option<&str> {
        self.search_tokens.get(position).and_then(Option::as_deref)
    }
}

/// The longest gather window a deployment may set
/// (`[audit.store] gather_window`): each request that writes an entry waits
/// up to it, against a read target of 15 ms (#6242).
pub const GATHER_WINDOW_MAX: std::time::Duration = std::time::Duration::from_millis(1);

/// The gathered intake write (#5770).
///
/// Requests hand their entries to one writer, which writes every entry
/// waiting when its previous write returned in one statement that commits on
/// its own, and answers each request once that commit is durable. Under load one commit, and one flush of the
/// write-ahead log, serves many requests; alone, a request waits for its own
/// statement only. No specification governs it: our own design.
#[derive(Debug, Clone)]
pub struct GroupWriter {
    jobs: tokio::sync::mpsc::Sender<Job>,
    known: std::sync::Arc<Known>,
}

impl GroupWriter {
    /// Spawn the writer over `pool` on the running runtime.
    #[must_use]
    pub fn spawn(pool: &sqlx::PgPool) -> Self {
        Self::spawn_gathering(pool, std::time::Duration::ZERO)
    }

    /// The writers over `pool`, each waiting up to `window` after the first
    /// waiting entry for more before it writes, at most
    /// [`GATHER_WINDOW_MAX`].
    #[must_use]
    pub fn spawn_gathering(pool: &sqlx::PgPool, window: std::time::Duration) -> Self {
        let window = window.min(GATHER_WINDOW_MAX);
        let (jobs, queue) = tokio::sync::mpsc::channel(QUEUE);
        let queue = std::sync::Arc::new(tokio::sync::Mutex::new(queue));
        let known = std::sync::Arc::new(Known::default());
        for _ in 0..WRITERS {
            tokio::spawn(gather(
                pool.clone(),
                std::sync::Arc::clone(&queue),
                std::sync::Arc::clone(&known),
                window,
            ));
        }
        Self { jobs, known }
    }

    /// Write `entries`, with their search `tokens` by position, held under
    /// `reserved_by`, on `conn`, which holds no transaction, in one statement
    /// that commits on its own (#6244).
    ///
    /// It knows what the gathered writers learned the store holds, so it
    /// sends no shared part the store keeps and names the search tokens it
    /// knows by id, as they do, and teaches them what its committed statement
    /// stored (`gather::write_known`).
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn write_on(
        &self,
        conn: &mut PgConnection,
        entries: &[IntakeEntry],
        tokens: &[Vec<IntakeToken>],
        reserved_by: Option<uuid::Uuid>,
    ) -> Result<(), AccessLogError> {
        if entries.is_empty() {
            return Ok(());
        }
        gather::write_known(conn, &self.known, entries, tokens, reserved_by).await
    }

    /// Write `entries`, with their search `tokens` by position, durably in
    /// the next gathered statement. The current request gives back the
    /// connections it keeps before it waits for the writers.
    ///
    /// # Errors
    /// [`AccessLogError::Gathered`] when the gathered write fails or the
    /// writer stopped.
    pub async fn write(
        &self,
        entries: Vec<IntakeEntry>,
        tokens: Vec<Vec<IntakeToken>>,
    ) -> Result<(), AccessLogError> {
        if entries.is_empty() {
            return Ok(());
        }
        let (done, answer) = tokio::sync::oneshot::channel();
        // NOTE: our own design (#6008), the writers take connections of the pool this request may
        // keep one of, so the request gives back what it keeps before it waits for them.
        crate::db::acquire::handoff(async {
            self.jobs
                .send(Job {
                    entries,
                    tokens,
                    done,
                })
                .await
                .map_err(|_stopped| AccessLogError::Gathered(None))?;
            match answer.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(failed)) => Err(AccessLogError::Gathered(Some(failed))),
                Err(_) => Err(AccessLogError::Gathered(None)),
            }
        })
        .await
    }
}

/// The gathered writer of a store, started on first use, waiting up to its
/// window after the first entry for more.
#[derive(Debug, Default)]
pub struct Gathered {
    writer: std::sync::OnceLock<GroupWriter>,
    window: std::time::Duration,
}

impl Gathered {
    /// A writer not yet started, gathering for up to `window`.
    #[must_use]
    pub fn new(window: std::time::Duration) -> Self {
        Self {
            writer: std::sync::OnceLock::new(),
            window,
        }
    }

    /// The writer over `pool`, started on the running runtime on first use.
    pub fn writer(&self, pool: &sqlx::PgPool) -> &GroupWriter {
        self.writer
            .get_or_init(|| GroupWriter::spawn_gathering(pool, self.window))
    }
}

mod gather;
mod ids;

#[cfg(test)]
mod tests;
