// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first fill and the refill of the binding side table, and the replica
//! leases that keep one mapping in force.
//!
//! The selectors are configuration, which a SQL migration cannot read, so the
//! rows are filled from the configured mapping and the party versions. The
//! first boot with no stored digest fills them under an advisory lock and
//! stores the mapping's digest, which leaves out the key issuers; after that a
//! replica whose mapping digest differs from the stored one refuses to start,
//! naming both, and [`refill`] is the one path for such a change: it takes the
//! lock, refuses while any replica's lease is live, refills from the versions
//! and stores the new digest. A key issuer is a key space of its own
//! ([`key_space`]), added and removed while the replicas serve: each lease
//! lists the key issuers its replica maintains, and [`Lease::tend`] brings the
//! key spaces to the live leases and fills a new one online. A fill refuses,
//! naming the parties, a store in which two `PERSON`s carry one selector
//! value. No openEHR spec governs the side table: our own design.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::{Connection, PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::db::acquire::{AcquireError, acquire};
use crate::party_binding::BindingMapping;
use crate::party_binding::key_space::{self, Progress};
use crate::party_binding::store::{
    self,
    held::{self, HeldSelector},
};
use crate::storage::error::StorageError;

/// The advisory lock key the fill and the refill take, so two replicas
/// booting together fill the table once.
const LOCK: i64 = 0x5013_b1d0;

/// How long a replica's lease lives without a renewal.
pub const LEASE_TTL: Duration = Duration::from_secs(60);

/// How often a running replica renews its lease and tends the key spaces.
pub const LEASE_RENEWAL: Duration = Duration::from_secs(20);

/// How long a key space whose fill found a value two parties carry waits
/// before the next look.
const DUPLICATE_RETRY: Duration = Duration::from_secs(300);

/// Why the side table cannot be filled, or a replica cannot run against it.
#[derive(Debug, thiserror::Error)]
pub enum FillError {
    /// The rows were filled under another mapping: another template,
    /// selector, role, group or membership. A key issuer added or removed
    /// changes no digest ([`key_space`]).
    #[error(
        "the party binding side table was filled under the mapping digest {stored}, and this \
         replica runs the mapping digest {running}: the templates, selectors, role, group or \
         membership of [authz.party_binding] changed, so stop every replica and run \
         `ferroehr party-binding refill` with the new mapping, or start this replica with the \
         mapping the table was filled under"
    )]
    Digest {
        /// The digest the table was filled under.
        stored: String,
        /// The digest of the mapping this replica runs.
        running: String,
    },
    /// A replica still runs against the table, so it cannot be refilled.
    #[error(
        "{} replica(s) hold a live lease on the party binding side table ({}): stop every \
         replica before `ferroehr party-binding refill`",
        replicas.len(),
        replicas.iter().map(Uuid::to_string).collect::<Vec<_>>().join(", ")
    )]
    Leased {
        /// The replicas whose lease is live.
        replicas: Vec<Uuid>,
    },
    /// Two parties carry one selector value.
    #[error(
        "the party binding cannot be filled: {}",
        duplicates.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    )]
    Duplicate {
        /// Each value two or more parties carry.
        duplicates: Vec<Duplicate>,
    },
    /// The party store failed.
    #[error("the party binding side table could not be filled")]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for FillError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(e))
    }
}

impl From<AcquireError> for FillError {
    fn from(e: AcquireError) -> Self {
        Self::Storage(StorageError::Acquire(e))
    }
}

/// One selector value two or more `PERSON`s carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// `key`, `user_name` or `external_id`.
    pub selector: &'static str,
    /// The issuer; empty for a `userName`.
    pub issuer: String,
    /// The `PERSON`s carrying it.
    pub parties: Vec<Uuid>,
}

impl std::fmt::Display for Duplicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parties: Vec<String> = self.parties.iter().map(Uuid::to_string).collect();
        let selector = match self.selector {
            "user_name" => "one userName",
            "external_id" => "one externalId",
            _ => "one binding key",
        };
        if self.issuer.is_empty() {
            write!(f, "the PERSONs {} carry {selector}", parties.join(" and "))
        } else {
            write!(
                f,
                "the PERSONs {} carry {selector} of the issuer {:?}",
                parties.join(" and "),
                self.issuer
            )
        }
    }
}

/// A replica's lease on the side table: while it is renewed, no refill runs,
/// and the key issuers it lists are maintained.
#[derive(Debug)]
pub struct Lease {
    /// The party pool.
    pool: PgPool,
    /// The replica holding the lease.
    replica: Uuid,
    /// The mapping the replica derives rows under.
    mapping: BindingMapping,
    /// When the fill of each key space last found a value two parties carry.
    stalled: Mutex<BTreeMap<String, Instant>>,
}

impl Lease {
    /// Prepares the side table for the replica `replica` running `mapping`,
    /// and takes its lease.
    ///
    /// Under the advisory lock: with no stored digest the table is filled,
    /// the digest stored and every key space of `mapping` recorded `active`;
    /// with a stored digest it must be this mapping's, and the key spaces are
    /// brought to the live leases, this one included ([`key_space`]).
    ///
    /// # Errors
    /// [`FillError::Digest`] when the table was filled under another mapping,
    /// [`FillError::Duplicate`] when a first fill finds a value two parties
    /// carry, [`FillError::Storage`] when the party store fails.
    pub async fn take(
        pool: &PgPool,
        mapping: &BindingMapping,
        replica: Uuid,
    ) -> Result<Self, FillError> {
        let running = mapping.digest();
        let spaces = mapping.key_spaces();
        let issuers: Vec<&String> = spaces.keys().collect();
        let portal: Vec<&String> = spaces.iter().filter(|(_, p)| **p).map(|(i, _)| i).collect();
        let mut conn = acquire(pool).await?;
        let mut tx = conn.begin().await?;
        lock(&mut tx).await?;
        let first = match stored_digest(&mut tx).await? {
            Some(stored) if stored == running => false,
            Some(stored) => return Err(FillError::Digest { stored, running }),
            None => {
                fill(&mut tx, mapping).await?;
                store_digest(&mut tx, &running).await?;
                key_space::reset(&mut tx, mapping).await?;
                true
            }
        };
        sqlx::query(
            "INSERT INTO binding_lease (replica, digest, issuers, portal) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (replica) DO UPDATE SET digest = EXCLUDED.digest, \
                 issuers = EXCLUDED.issuers, portal = EXCLUDED.portal, renewed_at = now()",
        )
        .bind(replica)
        .bind(&running)
        .bind(&issuers)
        .bind(&portal)
        .execute(&mut *tx)
        .await?;
        if !first {
            key_space::reconcile(&mut tx, LEASE_TTL).await?;
        }
        tx.commit().await?;
        drop(conn);
        Ok(Self {
            pool: pool.clone(),
            replica,
            mapping: mapping.clone(),
            stalled: Mutex::default(),
        })
    }

    /// Renews the lease.
    ///
    /// # Errors
    /// [`FillError::Storage`] when the party store fails.
    pub async fn renew(&self) -> Result<(), FillError> {
        let mut conn = acquire(&self.pool).await?;
        sqlx::query("UPDATE binding_lease SET renewed_at = now() WHERE replica = $1")
            .bind(self.replica)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    /// Renews the lease and tends the key spaces: brings them to the live
    /// leases, starts the fill of each `pending` one every live lease lists,
    /// and writes pages of each `filling` one this replica lists for at most
    /// half a renewal period, the next call resuming where it stopped.
    ///
    /// # Errors
    /// [`FillError::Duplicate`] naming the parties when a fill finds a value
    /// two parties carry under its issuer, which leaves that key space
    /// `pending` and looks again after five minutes; [`FillError::Storage`]
    /// when the party store fails.
    pub async fn tend(&self) -> Result<(), FillError> {
        self.renew().await?;
        let ready = {
            let mut conn = acquire(&self.pool).await?;
            let mut tx = conn.begin().await?;
            lock(&mut tx).await?;
            let ready = key_space::reconcile(&mut tx, LEASE_TTL).await?;
            tx.commit().await?;
            ready
        };
        for issuer in ready {
            if self.stalled_recently(&issuer) {
                continue;
            }
            let held = {
                let mut conn = acquire(&self.pool).await?;
                held::held_selectors(&mut conn, &self.mapping).await?
            };
            let found = issuer_duplicates(held, &issuer);
            if !found.is_empty() {
                self.stall(&issuer);
                return Err(FillError::Duplicate { duplicates: found });
            }
            let mut conn = acquire(&self.pool).await?;
            let mut tx = conn.begin().await?;
            lock(&mut tx).await?;
            key_space::start_fill(&mut tx, &self.mapping, &issuer, LEASE_TTL).await?;
            tx.commit().await?;
        }
        let started = Instant::now();
        for issuer in key_space::filling(&self.pool, &self.mapping).await? {
            loop {
                match self.fill_page(&issuer).await? {
                    Progress::More if started.elapsed() < LEASE_RENEWAL / 2 => {}
                    Progress::More => return Ok(()),
                    Progress::Done | Progress::Stopped => break,
                }
            }
        }
        Ok(())
    }

    /// Writes one page of the fill of `issuer`'s key space; a page that
    /// meets a key two parties carry moves it back to `pending`.
    async fn fill_page(&self, issuer: &str) -> Result<Progress, FillError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        lock(&mut tx).await?;
        match key_space::fill_page(&mut tx, &self.mapping, issuer).await {
            Ok(progress) => {
                tx.commit().await?;
                Ok(progress)
            }
            Err(e) if key_space::key_conflict(&e) => {
                drop(tx);
                let mut tx = conn.begin().await?;
                lock(&mut tx).await?;
                key_space::stall(&mut tx, issuer).await?;
                tx.commit().await?;
                self.stall(issuer);
                let held = held::held_selectors(&mut conn, &self.mapping).await?;
                Err(FillError::Duplicate {
                    duplicates: issuer_duplicates(held, issuer),
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Whether the fill of `issuer` found a duplicate within the retry
    /// period.
    fn stalled_recently(&self, issuer: &str) -> bool {
        self.stalled
            .lock()
            .ok()
            .and_then(|stalled| stalled.get(issuer).copied())
            .is_some_and(|at| at.elapsed() < DUPLICATE_RETRY)
    }

    /// Records that the fill of `issuer` found a duplicate now.
    fn stall(&self, issuer: &str) {
        if let Ok(mut stalled) = self.stalled.lock() {
            stalled.insert(issuer.to_owned(), Instant::now());
        }
    }

    /// Releases the lease, as a replica does when it stops.
    ///
    /// # Errors
    /// [`FillError::Storage`] when the party store fails.
    pub async fn release(self) -> Result<(), FillError> {
        let mut conn = acquire(&self.pool).await?;
        sqlx::query("DELETE FROM binding_lease WHERE replica = $1")
            .bind(self.replica)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }
}

/// The binding keys under `issuer` among `held` that two or more parties
/// carry.
fn issuer_duplicates(held: Vec<HeldSelector>, issuer: &str) -> Vec<Duplicate> {
    duplicates(
        held.into_iter()
            .filter(|h| h.selector == "key" && h.issuer == issuer)
            .collect(),
    )
}

/// Rebuilds the side table from the versions under `mapping` and stores its
/// digest: the one path by which the mapping of a deployment changes.
///
/// It takes the advisory lock, and refuses while any replica's lease was
/// renewed within `ttl`.
///
/// # Errors
/// [`FillError::Leased`] while a replica runs, [`FillError::Duplicate`] when a
/// value two parties carry, [`FillError::Storage`] when the party store
/// fails.
pub async fn refill(
    pool: &PgPool,
    mapping: &BindingMapping,
    ttl: Duration,
) -> Result<(), FillError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    lock(&mut tx).await?;
    let replicas: Vec<Uuid> = sqlx::query_scalar(
        "SELECT replica FROM binding_lease \
         WHERE renewed_at > now() - make_interval(secs => $1) ORDER BY replica",
    )
    .bind(ttl.as_secs_f64())
    .fetch_all(&mut *tx)
    .await?;
    if !replicas.is_empty() {
        return Err(FillError::Leased { replicas });
    }
    sqlx::query("DELETE FROM binding_lease")
        .execute(&mut *tx)
        .await?;
    fill(&mut tx, mapping).await?;
    store_digest(&mut tx, &mapping.digest()).await?;
    key_space::reset(&mut tx, mapping).await?;
    tx.commit().await?;
    Ok(())
}

/// Rebuilds the grant rows from the `AGENT` and `ROLE` versions under the
/// advisory lock: the refill of a deployment with no party mapping, which
/// takes no lease and stores no digest.
///
/// # Errors
/// [`FillError::Storage`] when the party store fails, two `AGENT`s naming
/// one client included.
pub async fn refill_grants(pool: &PgPool) -> Result<(), FillError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    lock(&mut tx).await?;
    store::refill_grants(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Fills the grant rows when they were never filled: a store whose grant
/// revision has not moved since the migration that made them, which a boot
/// runs so the policy parties committed before it are read.
///
/// # Errors
/// As [`refill_grants`].
pub async fn fill_grants_once(pool: &PgPool) -> Result<(), FillError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    lock(&mut tx).await?;
    if crate::party_binding::grants::store::revision(&mut tx).await? == 0 {
        store::refill_grants(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The digest the table was last filled under, when it was filled.
///
/// # Errors
/// [`FillError::Storage`] when the party store fails.
pub async fn digest(pool: &PgPool) -> Result<Option<String>, FillError> {
    let mut conn = acquire(pool).await?;
    stored_digest(&mut conn).await
}

/// Takes the transaction-scoped advisory lock of the fill.
async fn lock(tx: &mut PgConnection) -> Result<(), FillError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(LOCK)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// The stored digest.
async fn stored_digest(conn: &mut PgConnection) -> Result<Option<String>, FillError> {
    let row = sqlx::query("SELECT digest FROM binding_state")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(match row {
        Some(row) => Some(row.try_get("digest")?),
        None => None,
    })
}

/// Stores `digest` as the one the table was filled under.
async fn store_digest(tx: &mut PgConnection, digest: &str) -> Result<(), FillError> {
    sqlx::query(
        "INSERT INTO binding_state (digest) VALUES ($1) \
         ON CONFLICT (singleton) DO UPDATE SET digest = EXCLUDED.digest, filled_at = now()",
    )
    .bind(digest)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Fills the table from the versions, refusing a value two parties carry.
async fn fill(tx: &mut PgConnection, mapping: &BindingMapping) -> Result<(), FillError> {
    let duplicates = duplicates(held::held_selectors(tx, mapping).await?);
    if !duplicates.is_empty() {
        return Err(FillError::Duplicate { duplicates });
    }
    store::refill(tx, mapping).await?;
    Ok(())
}

/// The values of `held` that two or more parties carry.
pub(crate) fn duplicates(held: Vec<HeldSelector>) -> Vec<Duplicate> {
    let mut by_value: BTreeMap<(&'static str, String, String), Vec<Uuid>> = BTreeMap::new();
    for row in held {
        by_value
            .entry((row.selector, row.issuer, row.value))
            .or_default()
            .push(row.party_id);
    }
    by_value
        .into_iter()
        .filter_map(|((selector, issuer, _), mut parties)| {
            parties.sort();
            parties.dedup();
            (parties.len() > 1).then_some(Duplicate {
                selector,
                issuer,
                parties,
            })
        })
        .collect()
}
