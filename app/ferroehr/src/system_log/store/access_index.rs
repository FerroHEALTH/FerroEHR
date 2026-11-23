// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log's search index, filled from the stored entries (#6319,
//! #6322): each frontier round copies the search columns of the entries up to
//! the frontier into `audit.access_log_index`, with the ITI-81 search tokens
//! of each entry's projection rendered from the stored entry, and a search on
//! a token parameter first moves the frontier and the index past every entry
//! committed before it ([`AuditStore::catch_up`]). The SQL is
//! `crate::access_log::index`'s. No specification governs audit storage: our
//! own design.

use std::collections::BTreeMap;
#[cfg(feature = "fhir")]
use std::collections::BTreeSet;
#[cfg(feature = "fhir")]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sqlx::{Connection, PgConnection, Row};

use super::AuditStore;
use crate::access_log::index::{BATCH, BOUNDS, COPY, CURSOR, LOCK, POLICIES, ROWS};
use crate::db::acquire::acquire;
use crate::system_log::AuditError;
#[cfg(feature = "fhir")]
use crate::system_log::render::CompiledPolicy;

/// How long a search on a token parameter waits for the frontier and the
/// index to pass the entries committed before it, below the server's request
/// timeout, before it is refused as unavailable (#6322).
pub const CATCH_UP_WAIT: Duration = Duration::from_secs(2);

/// What the searches share of the catch-up: one runs it at a time, and how
/// many ran a round, so a search that waited behind one that passed its
/// entries runs none.
#[derive(Debug, Default)]
pub(crate) struct CatchUp {
    running: tokio::sync::Mutex<()>,
    rounds: AtomicU64,
}

/// The policies of a batch's entries by digest, compiled before the batch
/// takes its connection's transaction.
#[cfg(feature = "fhir")]
type Policies = BTreeMap<Vec<u8>, Arc<CompiledPolicy>>;

/// A slim build renders no projection, so it keeps no policy.
#[cfg(not(feature = "fhir"))]
type Policies = BTreeMap<Vec<u8>, ()>;

impl AuditStore {
    /// Copy the entries past the index's cursor up to `upto`, the access
    /// log's frontier, and below every entry a write holds into the index,
    /// each indexed entry with the search tokens of its projection, batch
    /// after batch; returns the cursor.
    ///
    /// # Errors
    /// The store errors when a statement fails, and the rendering and token
    /// errors of an entry; the cursor then stays where the last committed
    /// batch left it.
    pub async fn index_access_log(&self, upto: i64) -> Result<i64, AuditError> {
        let mut cursor = i64::MIN;
        loop {
            let after: i64 = sqlx::query_scalar(CURSOR)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            if after >= upto {
                return Ok(after);
            }
            let policies = self.batch_policies(after, upto).await?;
            let mut conn = acquire(&self.pool).await?;
            let reached = self.index_batch(&mut conn, upto, &policies).await?;
            if reached >= upto || reached <= cursor {
                return Ok(reached);
            }
            cursor = reached;
        }
    }

    /// The policies the entries of `(after, upto]` name, compiled, each read
    /// on a connection of its own before the batch takes one (#5879).
    async fn batch_policies(&self, after: i64, upto: i64) -> Result<Policies, AuditError> {
        let digests: Vec<Vec<u8>> = sqlx::query_scalar(POLICIES)
            .bind(after)
            .bind(upto.min(after.saturating_add(BATCH)))
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        let mut policies = Policies::new();
        for digest in digests {
            #[cfg(feature = "fhir")]
            let policy = self.policy(after, &digest).await?;
            #[cfg(not(feature = "fhir"))]
            let policy = ();
            policies.insert(digest, policy);
        }
        Ok(policies)
    }

    /// One batch on `conn`, in a transaction of its own that commits without
    /// waiting for its flush: the entries past the cursor up to `upto`, each
    /// rendered under its policy in `policies`, copied with their tokens;
    /// returns the cursor.
    async fn index_batch(
        &self,
        conn: &mut PgConnection,
        upto: i64,
        policies: &Policies,
    ) -> Result<i64, AuditError> {
        let mut tx = conn.begin().await?;
        // NOTE: our own design (#6319), the index is copied again from the cursor after a crash,
        // so its batch does not wait for the flush.
        sqlx::query("SET LOCAL synchronous_commit = off")
            .execute(&mut *tx)
            .await?;
        sqlx::query(LOCK).execute(&mut *tx).await?;
        let (after, last): (i64, i64) = sqlx::query_as(BOUNDS)
            .bind(upto)
            .bind(BATCH)
            .fetch_one(&mut *tx)
            .await?;
        if last <= after {
            return Ok(after);
        }
        let rows = sqlx::query(ROWS)
            .bind(after)
            .bind(last)
            .fetch_all(&mut *tx)
            .await?;
        // NOTE: our own design (#6322), an entry whose policy the batch did not compile is never
        // indexed without its tokens: the batch is left uncommitted and read again.
        let Some((ids, tokens)) = self.batch_tokens_of(&mut tx, &rows, policies).await? else {
            return Ok(after);
        };
        sqlx::query(COPY)
            .bind(after)
            .bind(last)
            .bind(ids)
            .bind(tokens)
            .execute(&mut *tx)
            .await?;
        // NOTE: our own design (#5762), the chainer's own round, which no request makes.
        tx.commit().await?;
        Ok(last)
    }

    /// The ids of the indexed entries of `rows` and the text of the
    /// `integer[]` of each one's search tokens, stored or found on `conn`;
    /// `None` when an entry names a policy `policies` does not hold.
    #[cfg(feature = "fhir")]
    async fn batch_tokens_of(
        &self,
        conn: &mut PgConnection,
        rows: &[sqlx::postgres::PgRow],
        policies: &Policies,
    ) -> Result<Option<(Vec<i64>, Vec<String>)>, AuditError> {
        let mut ids = Vec::with_capacity(rows.len());
        let mut tokens = Vec::with_capacity(rows.len());
        for row in rows {
            let digest: Option<Vec<u8>> = row.try_get("render_policy")?;
            let Some(digest) = digest else {
                continue;
            };
            let Some(policy) = policies.get(&digest) else {
                return Ok(None);
            };
            if row.try_get::<&str, _>("entry_type")? != "ACCESS_LOG_ENTRY" {
                continue;
            }
            let chain_seq: i64 = row.try_get("chain_seq")?;
            let entry = crate::access_log::store::stored_with(row, row.try_get("shared_part")?)?;
            ids.push(chain_seq);
            tokens.push(entry_tokens(&entry, chain_seq, policy));
        }
        let (values, learned) = self.tokens.record_tokens(conn, &tokens).await?;
        self.tokens.remember(learned);
        Ok(Some((ids, super::entries::token_texts(&values))))
    }

    /// A slim build renders no projection, so it indexes no search token.
    #[cfg(not(feature = "fhir"))]
    async fn batch_tokens_of(
        &self,
        _conn: &mut PgConnection,
        _rows: &[sqlx::postgres::PgRow],
        _policies: &Policies,
    ) -> Result<Option<(Vec<i64>, Vec<String>)>, AuditError> {
        Ok(Some((Vec::new(), Vec::new())))
    }

    /// Moves the frontier and the search index past every access-log entry
    /// committed before the call, within [`CATCH_UP_WAIT`], so a search on a
    /// token parameter finds each one through the index (#6322).
    ///
    /// The catch-ups run one at a time, and one that finds the index already
    /// past its entries does nothing, so concurrent searches run at most one
    /// round between them.
    ///
    /// # Errors
    /// [`AuditError::IndexBehind`] when the wait runs out, an open
    /// transaction holding the frontier back, and the store errors.
    pub async fn catch_up(&self) -> Result<(), AuditError> {
        let behind = AuditError::IndexBehind(CATCH_UP_WAIT);
        tokio::time::timeout(CATCH_UP_WAIT, self.catch_up_within())
            .await
            .map_err(|_elapsed| behind)?
    }

    /// The body of [`Self::catch_up`], which bounds it.
    async fn catch_up_within(&self) -> Result<(), AuditError> {
        let (target, _): (Option<i64>, Option<i64>) =
            sqlx::query_as(crate::access_log::frontier::LAST_VALUES)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
        let target = target.unwrap_or(0);
        let _running = self.catch_up.running.lock().await;
        if self.cursor().await? >= target {
            return Ok(());
        }
        let mut frontier = crate::access_log::frontier::Frontier::default();
        let reached = loop {
            let reached = frontier.round(&mut *acquire(&self.pool).await?).await?;
            if reached.access_log >= target {
                break reached.access_log;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        self.catch_up.rounds.fetch_add(1, Ordering::Relaxed);
        self.index_access_log(reached).await?;
        Ok(())
    }

    /// The index's cursor.
    async fn cursor(&self) -> Result<i64, AuditError> {
        Ok(sqlx::query_scalar(CURSOR)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?)
    }

    /// How many catch-up rounds the searches of this store ran.
    #[must_use]
    pub fn catch_up_rounds(&self) -> u64 {
        self.catch_up.rounds.load(Ordering::Relaxed)
    }
}

/// The ITI-81 search tokens of the projection of `entry`, at `chain_seq`,
/// rendered under `policy`: what the index keeps of it. An entry that does
/// not render has none, metered and logged, since it cannot be served either.
#[cfg(feature = "fhir")]
pub(crate) fn entry_tokens(
    entry: &openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry,
    chain_seq: i64,
    policy: &CompiledPolicy,
) -> BTreeSet<crate::system_log::search::SearchToken> {
    let projection = crate::access_log::projection::Projection {
        context: &policy.context,
        categories: &policy.categories,
    };
    match projection.record(entry, chain_seq) {
        Ok((event, subject)) => super::entries::tokens_of(
            crate::system_log::fhir::to_resource(&event, &policy.context, subject.as_deref()),
            &event,
        ),
        Err(error) => {
            crate::telemetry::metrics::metrics()
                .atna_audit_serialize_failed
                .add(1, &[]);
            tracing::warn!(chain_seq, "access-log entry indexed without search tokens: {error}");
            BTreeSet::new()
        }
    }
}
