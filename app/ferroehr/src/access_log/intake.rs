// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The intake of the access log: each entry written before the response that
//! reports its access, unchained, with every column the chain keeps (#5747).
//!
//! The writer supplies the row and shared parts of the entry's canonical
//! form, the content digest over the whole form, the columns the log derives
//! from the content and, when the entry is indexed, its ITI-81 index row, so
//! the chainer only computes the links
//! (`migrations/audit/0042_audit_intake.sql`). The forms are written
//! straight from the entry, with no JSON value of it built (#5849). Each
//! derived column is read from the entry's ITS-JSON form at the path the
//! database's own derive reads (`audit.access_log_record_derive`), and the
//! text is cast by the database as that function casts it, so a chained
//! record compares equal to its derivation, which the verifier checks. No
//! openEHR spec governs audit storage: our own design.

use std::collections::BTreeMap;

use jiff_sqlx::Timestamp;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::invariants::check_entry;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_object_without;
use sqlx::PgConnection;

use crate::access_log::chain::ALGORITHM;
use crate::access_log::store::AccessLogError;
use crate::access_log::stored_form;
use crate::db::acquire::acquire;
use crate::digest::sha256_hex;
use crate::system_log::search::digest::token_digest;

/// The member of an entry's ITS-JSON form its canonical form leaves out.
const LINK: &str = "link";

/// The ITI-81 index row of an entry: what the search filters on and the
/// render policy its projection is written under.
#[derive(Debug, Clone)]
pub struct IndexRow {
    /// When the access happened.
    pub recorded_at: jiff::Timestamp,
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

/// The columns the log derives from an entry's content, as text the database
/// casts.
#[derive(Debug, Default, PartialEq, Eq)]
struct Derived {
    record_type: Option<String>,
    uid: Option<String>,
    time_recorded: Option<String>,
    ehr_id: Option<String>,
    action_terminology: Option<String>,
    action_code: Option<String>,
    result_terminology: Option<String>,
    result_code: Option<String>,
    party_id: Option<String>,
    retain_until: Option<String>,
}

impl IntakeEntry {
    /// `entry` as the intake keeps it, with `index` when it is indexed.
    ///
    /// The entry's link is the chain's to decide, so it is replaced by the
    /// prospective link over the entry's content digest, and the entry is
    /// checked against the draft's invariants with it: an entry that breaks
    /// one never reaches the intake.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when the entry has no canonical form,
    /// and [`AccessLogError::Invalid`] when it breaks an invariant.
    pub fn of(entry: &mut AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        let intake = Self::stored(entry, index)?;
        entry.link = AccessLogLink {
            sequence: 1,
            algorithm: ALGORITHM.to_owned(),
            previous_digest: intake.content_digest.clone(),
            content_digest: intake.content_digest.clone(),
            digest: intake.content_digest.clone(),
            signature: None,
        };
        check_entry(entry)?;
        Ok(intake)
    }

    /// The stored parts of `entry`, written straight from it (#5849).
    fn stored(entry: &AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        let whole = canonical_object_without(entry, &[LINK]).map_err(AccessLogError::Canonical)?;
        let cut = stored_form::parts::Cut::of(&whole).map_err(AccessLogError::Canonical)?;
        let (row, shared) = cut.parts().map_err(AccessLogError::Canonical)?;
        let derived = Derived::of(&cut)?;
        Ok(Self {
            content_digest: sha256_hex(&whole),
            row,
            shared: shared.map(|text| (sha256_hex(&text), text)),
            derived,
            index,
        })
    }
}

impl Derived {
    /// The derived columns of the entry whose canonical form `cut` read, at
    /// the paths `audit.access_log_record_derive` reads.
    fn of(cut: &stored_form::parts::Cut<'_>) -> Result<Self, AccessLogError> {
        let text = |path: &[&str]| cut.text_at(path).map_err(AccessLogError::Canonical);
        Ok(Self {
            record_type: text(&["_type"])?,
            uid: text(&["uid", "value"])?,
            time_recorded: text(&["time_recorded", "value"])?,
            ehr_id: text(&["ehr_id", "value"])?,
            action_terminology: text(&["action", "defining_code", "terminology_id", "value"])?,
            action_code: text(&["action", "defining_code", "code_string"])?,
            result_terminology: text(&[
                "outcome",
                "result",
                "defining_code",
                "terminology_id",
                "value",
            ])?,
            result_code: text(&["outcome", "result", "defining_code", "code_string"])?,
            party_id: text(&["accessor", "party", "external_ref", "id", "value"])?,
            retain_until: text(&["retention", "retain_until", "value"])?,
        })
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
    recorded_at: Vec<Option<Timestamp>>,
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
    /// Adds `entry`'s row.
    fn push(&mut self, entry: &'e IntakeEntry) {
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
        self.recorded_at
            .push(index.map(|index| Timestamp::from(index.recorded_at)));
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
            .push(index.map(|index| index.search_tokens.as_str()));
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

/// Write `entries` into the intake on `conn`, in their order, in one
/// statement.
///
/// The shared parts they name go in with them. `tokens` holds, by the
/// position of its entry, the search tokens of each entry whose index row
/// leaves them to the statement (an empty `search_tokens`); the statement
/// stores or finds them and their shared set. They are held under
/// `reserved_by` when it names a reservation (#5762). On a connection
/// outside a transaction the statement commits on its own.
///
/// # Errors
/// The store errors when the statement fails.
pub async fn take_in(
    conn: &mut PgConnection,
    entries: &[IntakeEntry],
    tokens: &[Vec<IntakeToken>],
    reserved_by: Option<uuid::Uuid>,
) -> Result<(), AccessLogError> {
    if entries.is_empty() {
        return Ok(());
    }
    // NOTE: our own design (#5881), the shared parts go in ordered by digest, so
    // concurrent statements that share some take their row locks in one order.
    let mut shared = BTreeMap::new();
    let mut columns = Columns::default();
    for entry in entries {
        if let Some((digest, content)) = &entry.shared {
            shared.insert(digest.as_str(), content.as_str());
        }
        columns.push(entry);
    }
    let (keys, texts): (Vec<&str>, Vec<&str>) = shared.into_iter().unzip();
    let mut token_entries = Vec::new();
    let mut parameters = Vec::new();
    let mut systems = Vec::new();
    let mut codes = Vec::new();
    let mut in_sets = Vec::new();
    let mut token_digests = Vec::new();
    for (position, entry_tokens) in (1_i32..).zip(tokens) {
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
    sqlx::query(INTAKE)
        .bind(c.record_types)
        .bind(c.uids)
        .bind(c.times)
        .bind(c.contents)
        .bind(c.shared)
        .bind(c.digests)
        .bind(c.ehr_ids)
        .bind(c.party_ids)
        .bind(c.retain_until)
        .bind(c.action_terminologies)
        .bind(c.action_codes)
        .bind(c.result_terminologies)
        .bind(c.result_codes)
        .bind(c.recorded_at)
        .bind(c.actions)
        .bind(c.outcomes)
        .bind(c.principals)
        .bind(c.patient_ids)
        .bind(c.resource_ids)
        .bind(c.request_ids)
        .bind(c.client_ips)
        .bind(c.index_ehr_ids)
        .bind(c.tokens)
        .bind(c.policies)
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
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// One request's entries waiting for the gathered intake write.
struct Job {
    entries: Vec<IntakeEntry>,
    tokens: Vec<Vec<IntakeToken>>,
    done: tokio::sync::oneshot::Sender<Result<(), std::sync::Arc<AccessLogError>>>,
}

/// The most entries one gathered write takes.
const GATHER_MAX: usize = 512;

/// How many jobs may wait for the writers before a request waits to queue.
const QUEUE: usize = 4096;

/// How many gathered writes may be in flight at once, each on its own
/// connection.
const WRITERS: usize = 4;

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
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl GroupWriter {
    /// Spawn the writer over `pool` on the running runtime.
    #[must_use]
    pub fn spawn(pool: &sqlx::PgPool) -> Self {
        let (jobs, queue) = tokio::sync::mpsc::channel(QUEUE);
        let queue = std::sync::Arc::new(tokio::sync::Mutex::new(queue));
        for _ in 0..WRITERS {
            tokio::spawn(gather(pool.clone(), std::sync::Arc::clone(&queue)));
        }
        Self { jobs }
    }

    /// Write `entries`, with their search `tokens` by position, durably in
    /// the next gathered statement.
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
    }
}

/// One writer's loop, until every sender is gone: it takes what waits, lets
/// the next writer take what arrives meanwhile, and writes.
async fn gather(
    pool: sqlx::PgPool,
    queue: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Job>>>,
) {
    loop {
        let mut waiting = queue.lock().await;
        let Some(first) = waiting.recv().await else {
            return;
        };
        let mut jobs = vec![first];
        let mut count = jobs.iter().map(|job| job.entries.len()).sum::<usize>();
        while count < GATHER_MAX {
            let Ok(job) = waiting.try_recv() else {
                break;
            };
            count = count.saturating_add(job.entries.len());
            jobs.push(job);
        }
        drop(waiting);
        write_jobs(&pool, jobs, count).await;
    }
}

/// Writes `jobs`, `count` entries in all, in one statement, and answers each
/// job by its own outcome.
///
/// When the database refuses the gathered statement, which then committed
/// nothing, each job is written again on its own, so an entry that fails
/// refuses only its own request. Any other failure leaves the outcome of the
/// statement unknown, or no statement ran, and every job is refused: none is
/// written twice and none is answered as written when it may not be.
async fn write_jobs(pool: &sqlx::PgPool, jobs: Vec<Job>, count: usize) {
    let mut entries = Vec::with_capacity(count);
    let mut tokens = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(jobs.len());
    let mut answers = Vec::with_capacity(jobs.len());
    for job in jobs {
        let start = entries.len();
        let mut job_tokens = job.tokens;
        job_tokens.resize(job.entries.len(), Vec::new());
        entries.extend(job.entries);
        tokens.extend(job_tokens);
        spans.push(start..entries.len());
        answers.push(job.done);
    }
    let written = write_retrying(pool, &entries, &tokens).await;
    match written {
        Err(refused) if answers.len() > 1 && refused_by_database(&refused) => {
            tracing::warn!(
                requests = answers.len(),
                error = %refused,
                "the gathered access-log intake write was refused; writing each request on its own"
            );
            for (answer, span) in answers.into_iter().zip(spans) {
                let own = match (entries.get(span.clone()), tokens.get(span)) {
                    (Some(own), Some(own_tokens)) => write_retrying(pool, own, own_tokens).await,
                    _ => Err(AccessLogError::Gathered(None)),
                };
                // NOTE: our own design (#5770), a request that stopped waiting needs no answer.
                let _gone = answer.send(own.map_err(std::sync::Arc::new));
            }
        }
        written => {
            let written = written.map_err(std::sync::Arc::new);
            for answer in answers {
                // NOTE: our own design (#5770), a request that stopped waiting needs no answer.
                let _gone = answer.send(written.clone());
            }
        }
    }
}

/// How many times a statement the database refused as a deadlock or a
/// serialization failure is run in all.
const ATTEMPTS: u32 = 3;

/// Writes `entries` in one statement on a connection of its own, running it
/// again while the database refuses it as a deadlock or a serialization
/// failure (SQLSTATE `40P01`, `40001`): the refused statement committed
/// nothing, so running it again writes each entry once.
async fn write_retrying(
    pool: &sqlx::PgPool,
    entries: &[IntakeEntry],
    tokens: &[Vec<IntakeToken>],
) -> Result<(), AccessLogError> {
    let mut attempt = 1;
    loop {
        let written = async {
            let mut conn = acquire(pool).await?;
            take_in(&mut conn, entries, tokens, None).await
        }
        .await;
        match written {
            Err(refused) if attempt < ATTEMPTS && transient(&refused) => {
                tracing::warn!(attempt, error = %refused, "the access-log intake write runs again");
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(attempt) * 5)).await;
                attempt += 1;
            }
            written => return written,
        }
    }
}

/// The SQLSTATE of the error the database answered `error` with, `None` when
/// the database did not answer one.
fn sqlstate(error: &AccessLogError) -> Option<String> {
    match error {
        AccessLogError::Store(sqlx::Error::Database(database)) => {
            database.code().map(std::borrow::Cow::into_owned)
        }
        _ => None,
    }
}

/// Whether the database answered `error`, so the statement it refused rolled
/// back whole (`PostgreSQL` 18 docs, "Transactions": a statement outside a
/// transaction block runs in one of its own).
fn refused_by_database(error: &AccessLogError) -> bool {
    matches!(error, AccessLogError::Store(sqlx::Error::Database(_)))
}

/// Whether `error` is a deadlock or a serialization failure, which a run of
/// the same statement may not meet again.
fn transient(error: &AccessLogError) -> bool {
    sqlstate(error).is_some_and(|code| code == "40P01" || code == "40001")
}

#[cfg(test)]
mod tests;
