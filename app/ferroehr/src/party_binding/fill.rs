// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The first fill and the refill of the binding side table, and the replica
//! leases that keep one mapping in force.
//!
//! The selectors are configuration, which a SQL migration cannot read, so the
//! rows are filled from the configured mapping and the party versions. The
//! first boot with no stored digest fills them under an advisory lock and
//! stores the mapping's digest; after that a replica whose mapping digest
//! differs from the stored one refuses to start, naming both. A mapping change
//! has one path, [`refill`]: it takes the lock, refuses while any replica's
//! lease is live, refills from the versions and stores the new digest. A fill
//! refuses, naming the parties, a store in which two `PERSON`s carry one
//! selector value. No openEHR spec governs the side table: our own design.

use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::party_binding::BindingMapping;
use crate::party_binding::store::{self, HeldSelector};
use crate::storage::error::StorageError;

/// The advisory lock key the fill and the refill take, so two replicas
/// booting together fill the table once.
const LOCK: i64 = 0x5013_b1d0;

/// How long a replica's lease lives without a renewal.
pub const LEASE_TTL: Duration = Duration::from_secs(60);

/// How often a running replica renews its lease.
pub const LEASE_RENEWAL: Duration = Duration::from_secs(20);

/// Why the side table cannot be filled, or a replica cannot run against it.
#[derive(Debug, thiserror::Error)]
pub enum FillError {
    /// The rows were filled under another mapping.
    #[error(
        "the party binding side table was filled under the mapping digest {stored}, and this \
         replica runs the mapping digest {running}: stop every replica and run \
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
    /// No party mapping is configured, so there is nothing to fill.
    #[error(
        "no [[authz.party_binding.person.template]] or [authz.party_binding.group] is \
         configured, so the party binding side table holds no row"
    )]
    Unmapped,
    /// The party store failed.
    #[error("the party binding side table could not be filled")]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for FillError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(e))
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

/// A replica's lease on the side table: while it is renewed, no refill runs.
#[derive(Debug)]
pub struct Lease {
    /// The party pool.
    pool: PgPool,
    /// The replica holding the lease.
    replica: Uuid,
}

impl Lease {
    /// Prepares the side table for the replica `replica` running `mapping`,
    /// and takes its lease.
    ///
    /// Under the advisory lock: with no stored digest the table is filled
    /// and the digest stored; with a stored digest it must be this mapping's.
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
        let mut tx = pool.begin().await?;
        lock(&mut tx).await?;
        match stored_digest(&mut tx).await? {
            Some(stored) if stored == running => {}
            Some(stored) => return Err(FillError::Digest { stored, running }),
            None => {
                fill(&mut tx, mapping).await?;
                store_digest(&mut tx, &running).await?;
            }
        }
        sqlx::query(
            "INSERT INTO binding_lease (replica, digest) VALUES ($1, $2) \
             ON CONFLICT (replica) DO UPDATE SET digest = EXCLUDED.digest, renewed_at = now()",
        )
        .bind(replica)
        .bind(&running)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Self {
            pool: pool.clone(),
            replica,
        })
    }

    /// Renews the lease.
    ///
    /// # Errors
    /// [`FillError::Storage`] when the party store fails.
    pub async fn renew(&self) -> Result<(), FillError> {
        sqlx::query("UPDATE binding_lease SET renewed_at = now() WHERE replica = $1")
            .bind(self.replica)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Releases the lease, as a replica does when it stops.
    ///
    /// # Errors
    /// [`FillError::Storage`] when the party store fails.
    pub async fn release(self) -> Result<(), FillError> {
        sqlx::query("DELETE FROM binding_lease WHERE replica = $1")
            .bind(self.replica)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
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
    let mut tx = pool.begin().await?;
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
    tx.commit().await?;
    Ok(())
}

/// The digest the table was last filled under, when it was filled.
///
/// # Errors
/// [`FillError::Storage`] when the party store fails.
pub async fn digest(pool: &PgPool) -> Result<Option<String>, FillError> {
    let mut conn = pool.acquire().await?;
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
    let duplicates = duplicates(store::held_selectors(tx, mapping).await?);
    if !duplicates.is_empty() {
        return Err(FillError::Duplicate { duplicates });
    }
    store::refill(tx, mapping).await?;
    Ok(())
}

/// The values of `held` that two or more parties carry.
fn duplicates(held: Vec<HeldSelector>) -> Vec<Duplicate> {
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
