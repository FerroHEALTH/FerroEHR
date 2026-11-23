// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gathered intake write: the writers' loop, what they learned the store
//! holds, and the retries of a refused statement (#5770, #6005, #6242). No
//! specification governs it: our own design.

use std::collections::{BTreeMap, BTreeSet};

use super::{IntakeEntry, take_in};
use crate::access_log::store::AccessLogError;
use crate::db::acquire::acquire;
use crate::telemetry::stage::{self, Stage};

/// One request's entries waiting for the gathered intake write.
pub(super) struct Job {
    pub(super) entries: Vec<IntakeEntry>,
    pub(super) done: tokio::sync::oneshot::Sender<Result<(), std::sync::Arc<AccessLogError>>>,
}

/// The most entries one gathered write takes.
pub(super) const GATHER_MAX: usize = 512;

/// How many jobs may wait for the writers before a request waits to queue.
pub(super) const QUEUE: usize = 4096;

/// How many gathered writes may be in flight at once, each on its own
/// connection.
///
/// Two, so one slow flush of the write-ahead log never holds every request.
/// More writers send more statements of fewer entries, which costs the store
/// more for each entry and puts more commits behind each flush (#6058).
pub(super) const WRITERS: usize = 2;

/// The shared parts the gathered writes stored, each with the `retain_until`
/// the store keeps it until at least (`None` for no end), so a later write
/// sends only the parts it does not know stored (#6005).
///
/// A part is learned only from a statement that committed on its own, and the
/// store only ever raises a part's `retain_until`, so the store keeps every
/// part learned until at least its learned date. The retention sweep removes
/// only parts whose `retain_until` passed more than a day ago
/// (`audit.access_log_shared_collect`), so a part is skipped only while its
/// learned date is past tomorrow. The intake statement still refuses an entry
/// whose part is gone; such a refusal forgets the skipped parts and writes
/// the statement again with them. No specification governs it: our own
/// design.
#[derive(Debug, Default)]
pub(super) struct StoredParts {
    known: std::sync::Mutex<BTreeMap<String, Option<jiff::civil::Date>>>,
}

/// The most parts [`StoredParts`] remembers before it starts again.
pub(super) const KNOWN_PARTS: usize = 4096;

impl StoredParts {
    /// The guard of the known parts, recovered from a poisoned lock, which
    /// leaves the map whole: each change is one insert or removal.
    pub(super) fn known(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, Option<jiff::civil::Date>>> {
        self.known
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The digests of the parts `entries` name that the store keeps at least
    /// as long as every entry naming them needs, past `today`'s next day.
    pub(super) fn stored<'e>(
        &self,
        entries: &'e [IntakeEntry],
        today: jiff::civil::Date,
    ) -> BTreeSet<&'e str> {
        let Ok(tomorrow) = today.tomorrow() else {
            return BTreeSet::new();
        };
        let known = self.known();
        needed(entries)
            .into_iter()
            .filter(|(digest, need)| match (known.get(*digest), need) {
                (Some(None), Need::Until(_) | Need::Forever) => true,
                (Some(Some(kept)), Need::Until(until)) => *kept > tomorrow && until <= kept,
                _ => false,
            })
            .map(|(digest, _)| digest)
            .collect()
    }

    /// Learns the parts `entries` name, which a committed statement stored
    /// or kept for at least as long as each entry naming it needs; a part
    /// whose need does not read is not learned.
    pub(super) fn learn(&self, entries: &[IntakeEntry]) {
        let mut known = self.known();
        if known.len() > KNOWN_PARTS {
            known.clear();
        }
        for (digest, need) in needed(entries) {
            let until = match need {
                Need::Until(until) => Some(until),
                Need::Forever => None,
                Need::Unread => continue,
            };
            let kept = known.entry(digest.to_owned()).or_insert(until);
            *kept = match (*kept, until) {
                (None, _) | (_, None) => None,
                (Some(kept), Some(until)) => Some(kept.max(until)),
            };
        }
    }

    /// Forgets the parts `digests` names.
    pub(super) fn forget(&self, digests: &BTreeSet<&str>) {
        let mut known = self.known();
        for digest in digests {
            known.remove(*digest);
        }
    }
}

/// How long the entries naming a shared part need it kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Need {
    /// Until this date.
    Until(jiff::civil::Date),
    /// Without end: an entry states no `retain_until`.
    Forever,
    /// An entry's `retain_until` does not read as a date here, so how long
    /// the store keeps the part is not known.
    Unread,
}

impl Need {
    /// The need of both `self` and `other`.
    pub(super) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unread, _) | (_, Self::Unread) => Self::Unread,
            (Self::Forever, _) | (_, Self::Forever) => Self::Forever,
            (Self::Until(left), Self::Until(right)) => Self::Until(left.max(right)),
        }
    }
}

/// Each shared part `entries` name, with how long the entries naming it need
/// it kept.
pub(super) fn needed(entries: &[IntakeEntry]) -> BTreeMap<&str, Need> {
    let mut needed: BTreeMap<&str, Need> = BTreeMap::new();
    for entry in entries {
        let Some((digest, _)) = &entry.shared else {
            continue;
        };
        // NOTE: our own design (#6005), an end that does not read here is never assumed.
        let need = match entry.derived.retain_until.as_deref() {
            None => Need::Forever,
            Some(text) => text.parse().map_or(Need::Unread, Need::Until),
        };
        needed
            .entry(digest.as_str())
            .and_modify(|kept| *kept = kept.and(need))
            .or_insert(need);
    }
    needed
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// What the gathered writers learned the store holds: the shared parts.
#[derive(Debug, Default)]
pub(super) struct Known {
    parts: StoredParts,
}

/// One writer's loop, until every sender is gone: it takes what waits, and
/// what arrives within `window` of the first, lets the next writer take what
/// arrives meanwhile, and writes.
pub(super) async fn gather(
    pool: sqlx::PgPool,
    queue: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Job>>>,
    known: std::sync::Arc<Known>,
    window: std::time::Duration,
) {
    loop {
        let mut waiting = queue.lock().await;
        let Some(first) = waiting.recv().await else {
            return;
        };
        let until = tokio::time::Instant::now() + window;
        let mut jobs = vec![first];
        let mut count = jobs.iter().map(|job| job.entries.len()).sum::<usize>();
        while count < GATHER_MAX {
            let job = match waiting.try_recv() {
                Ok(job) => job,
                Err(_) if window.is_zero() => break,
                Err(_) => match tokio::time::timeout_at(until, waiting.recv()).await {
                    Ok(Some(job)) => job,
                    Ok(None) | Err(_) => break,
                },
            };
            count = count.saturating_add(job.entries.len());
            jobs.push(job);
        }
        drop(waiting);
        write_jobs(&pool, &known, jobs, count).await;
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
pub(super) async fn write_jobs(pool: &sqlx::PgPool, known: &Known, jobs: Vec<Job>, count: usize) {
    let mut entries = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(jobs.len());
    let mut answers = Vec::with_capacity(jobs.len());
    for job in jobs {
        let start = entries.len();
        entries.extend(job.entries);
        spans.push(start..entries.len());
        answers.push(job.done);
    }
    let written = write_retrying(pool, known, &entries).await;
    match written {
        Err(refused) if answers.len() > 1 && refused_by_database(&refused) => {
            tracing::warn!(
                requests = answers.len(),
                error = %refused,
                "the gathered access-log intake write was refused; writing each request on its own"
            );
            for (answer, span) in answers.into_iter().zip(spans) {
                let own = match entries.get(span) {
                    Some(own) => write_retrying(pool, known, own).await,
                    None => Err(AccessLogError::Gathered(None)),
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
pub(super) const ATTEMPTS: u32 = 3;

/// Writes `entries` in one statement on a connection of its own, as
/// [`write_known`] writes them.
pub(super) async fn write_retrying(
    pool: &sqlx::PgPool,
    known: &Known,
    entries: &[IntakeEntry],
) -> Result<(), AccessLogError> {
    let mut conn = acquire(pool).await?;
    write_known(&mut conn, known, entries, None).await
}

/// Writes `entries` on `conn`, which holds no transaction, in one statement
/// that commits on its own, held under `reserved_by` when it names a
/// reservation, running it again while the database refuses it as a
/// deadlock or a serialization failure (SQLSTATE `40P01`, `40001`): the
/// refused statement committed nothing, so running it again writes each
/// entry once.
///
/// The parts `known` knows stored are left out of the statement; when the
/// store no longer holds such a part (SQLSTATE `23503`), they are forgotten
/// and the statement runs again with every part. A committed statement
/// teaches `known` the parts it stored or kept.
pub(super) async fn write_known(
    conn: &mut sqlx::PgConnection,
    known: &Known,
    entries: &[IntakeEntry],
    reserved_by: Option<uuid::Uuid>,
) -> Result<(), AccessLogError> {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let mut stored = known.parts.stored(entries, today);
    let mut attempt = 1;
    loop {
        let statement = take_in(&mut *conn, entries, reserved_by, &stored);
        match stage::instrument(Stage::Gather, statement).await {
            Err(refused) if !stored.is_empty() && unknown_part(&refused) => {
                tracing::warn!(error = %refused, "a shared part the intake knew stored is gone; it is written again");
                known.parts.forget(&stored);
                stored.clear();
            }
            Err(refused) if attempt < ATTEMPTS && transient(&refused) => {
                tracing::warn!(attempt, error = %refused, "the access-log intake write runs again");
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(attempt) * 5)).await;
                attempt += 1;
            }
            Ok(()) => {
                known.parts.learn(entries);
                return Ok(());
            }
            written => return written,
        }
    }
}

/// Whether the database refused `error` because an entry names a shared part
/// it does not hold (`audit.access_log_intake_unknown`).
pub(super) fn unknown_part(error: &AccessLogError) -> bool {
    sqlstate(error).is_some_and(|code| code == "23503")
}

/// The SQLSTATE of the error the database answered `error` with, `None` when
/// the database did not answer one.
pub(super) fn sqlstate(error: &AccessLogError) -> Option<String> {
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
pub(super) fn refused_by_database(error: &AccessLogError) -> bool {
    matches!(error, AccessLogError::Store(sqlx::Error::Database(_)))
}

/// Whether `error` is a deadlock or a serialization failure, which a run of
/// the same statement may not meet again.
pub(super) fn transient(error: &AccessLogError) -> bool {
    sqlstate(error).is_some_and(|code| code == "40P01" || code == "40001")
}
