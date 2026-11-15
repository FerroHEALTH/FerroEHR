// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The key spaces of the binding side table: one per binding-key issuer, each
//! added and removed while the replicas serve.
//!
//! A key row under an issuer depends only on whether the mapping lists the
//! issuer and whether it is a `patients` issuer, so each issuer's key rows
//! are a space of their own. A replica that names a new issuer records it
//! `pending`; once every live lease lists it with one portal flag, its rows
//! are filled online and it turns `active`. A replica that drops an issuer,
//! or names it with the other flag, turns it `retiring`, and its rows are
//! deleted once no live lease lists it so. A key row binds only while its
//! issuer's key space is `active`, so no replica reads a key space some live
//! replica does not maintain. No openEHR spec governs the side table: our
//! own design.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::party_binding::BindingMapping;
use crate::storage::error::StorageError;

/// The advisory lock a party commit holds shared and the online fill holds
/// exclusive, so no commit derives rows while a page of a key space is
/// written.
const GATE: i64 = 0x5013_b1d1;

/// Where a key space stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySpaceState {
    /// A live replica names it and another does not yet: its rows do not
    /// bind.
    Pending,
    /// Every live replica names it and its rows are being filled: they do
    /// not bind yet.
    Filling,
    /// Every live replica maintains its rows, which bind.
    Active,
    /// A live replica no longer names it: its rows bind nowhere, and they
    /// go once no live replica names it.
    Retiring,
}

impl KeySpaceState {
    /// The stored name of the state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Filling => "filling",
            Self::Active => "active",
            Self::Retiring => "retiring",
        }
    }

    /// The state stored as `name`, `None` for another name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "pending" => Some(Self::Pending),
            "filling" => Some(Self::Filling),
            "active" => Some(Self::Active),
            "retiring" => Some(Self::Retiring),
            _ => None,
        }
    }
}

/// A login looked up under an issuer whose key space is not `active`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the key space of the issuer {issuer:?} is {}, so its binding keys bind no login",
    state.as_str()
)]
pub struct KeySpaceClosed {
    /// The issuer.
    pub issuer: String,
    /// Its key space's state.
    pub state: KeySpaceState,
}

/// One key space as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpace {
    /// The issuer.
    pub issuer: String,
    /// Whether it is a `patients` issuer.
    pub portal: bool,
    /// Where it stands.
    pub state: KeySpaceState,
}

/// Every stored key space, by issuer.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn list(pool: &PgPool) -> Result<Vec<KeySpace>, StorageError> {
    let rows = sqlx::query("SELECT issuer, portal, state FROM binding_key_space ORDER BY issuer")
        .fetch_all(pool)
        .await?;
    rows.iter().map(stored).collect()
}

/// The key space of one stored row.
fn stored(row: &sqlx::postgres::PgRow) -> Result<KeySpace, StorageError> {
    let state: String = row.try_get("state")?;
    Ok(KeySpace {
        issuer: row.try_get("issuer")?,
        portal: row.try_get("portal")?,
        state: KeySpaceState::parse(&state).ok_or_else(|| {
            StorageError::InvalidRows(format!("the key space state {state:?} is not one of four"))
        })?,
    })
}

/// Takes the commit side of the fill gate on `tx`, so a page of the online
/// fill never interleaves with the rows a party commit derives.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn gate_shared(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(GATE)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Takes the fill side of the gate on `tx`.
async fn gate_exclusive(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Records every key space of `mapping` as `active` and no other, on `tx`:
/// what a full fill of the side table leaves.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn reset(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_key_space")
        .execute(&mut *tx)
        .await?;
    let (issuers, portal): (Vec<String>, Vec<bool>) = mapping.key_spaces().into_iter().unzip();
    sqlx::query(
        "INSERT INTO binding_key_space (issuer, portal, state) \
         SELECT i, p, 'active' FROM unnest($1::text[], $2::boolean[]) AS t(i, p)",
    )
    .bind(&issuers)
    .bind(&portal)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The key issuers one live lease maintains.
#[derive(Debug)]
struct Maintained {
    /// Every key issuer of the replica.
    issuers: BTreeSet<String>,
    /// The `patients` issuers among them.
    portal: BTreeSet<String>,
}

impl Maintained {
    /// Whether the replica derives the key rows of `issuer` with the portal
    /// flag `portal`.
    fn lists(&self, issuer: &str, portal: bool) -> bool {
        self.issuers.contains(issuer) && self.portal.contains(issuer) == portal
    }
}

/// The leases renewed within `ttl`, in replica order.
async fn live(tx: &mut PgConnection, ttl: Duration) -> Result<Vec<Maintained>, StorageError> {
    let rows = sqlx::query(
        "SELECT issuers, portal FROM binding_lease \
         WHERE renewed_at > now() - make_interval(secs => $1) ORDER BY replica",
    )
    .bind(ttl.as_secs_f64())
    .fetch_all(&mut *tx)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let issuers: Vec<String> = row.try_get("issuers")?;
        let portal: Vec<String> = row.try_get("portal")?;
        out.push(Maintained {
            issuers: issuers.into_iter().collect(),
            portal: portal.into_iter().collect(),
        });
    }
    Ok(out)
}

/// Brings the key spaces to the leases renewed within `ttl`, on `tx`, which
/// holds the fill lock, returning the `pending` issuers every live lease now
/// lists, ready to fill.
///
/// An `active` or `filling` key space some live lease does not list with its
/// flag turns `retiring`; a `pending` or `retiring` one no live lease lists
/// with its flag goes, its rows with it; a `retiring` one every live lease
/// lists again turns `pending`, since a replica that did not maintain its
/// rows ran meanwhile, so it fills anew; an issuer a live lease lists and no
/// key space records is recorded `pending`, with that lease's flag.
///
/// # Errors
/// [`StorageError`] on a driver failure or a stored state of no known name.
pub(crate) async fn reconcile(
    tx: &mut PgConnection,
    ttl: Duration,
) -> Result<Vec<String>, StorageError> {
    let leases = live(tx, ttl).await?;
    if leases.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT issuer, portal, state FROM binding_key_space ORDER BY issuer FOR UPDATE",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut known = BTreeSet::new();
    let mut ready = Vec::new();
    for space in rows.iter().map(stored) {
        let space = space?;
        let all = leases.iter().all(|l| l.lists(&space.issuer, space.portal));
        let any = leases.iter().any(|l| l.lists(&space.issuer, space.portal));
        match space.state {
            KeySpaceState::Active | KeySpaceState::Filling if !all => {
                set_state(tx, &space.issuer, KeySpaceState::Retiring).await?;
            }
            KeySpaceState::Pending | KeySpaceState::Retiring if !any => {
                prune(tx, &space.issuer).await?;
                continue;
            }
            KeySpaceState::Pending if all => ready.push(space.issuer.clone()),
            KeySpaceState::Retiring if all => {
                set_state(tx, &space.issuer, KeySpaceState::Pending).await?;
                ready.push(space.issuer.clone());
            }
            _ => {}
        }
        known.insert(space.issuer);
    }
    let mut added: BTreeMap<&str, bool> = BTreeMap::new();
    for lease in &leases {
        for issuer in &lease.issuers {
            if !known.contains(issuer) {
                added
                    .entry(issuer)
                    .or_insert_with(|| lease.portal.contains(issuer));
            }
        }
    }
    for (issuer, portal) in added {
        sqlx::query(
            "INSERT INTO binding_key_space (issuer, portal, state) VALUES ($1, $2, 'pending') \
             ON CONFLICT (issuer) DO NOTHING",
        )
        .bind(issuer)
        .bind(portal)
        .execute(&mut *tx)
        .await?;
        tracing::info!(issuer, portal, "a binding key space is pending");
        if leases.iter().all(|l| l.lists(issuer, portal)) {
            ready.push(issuer.to_owned());
        }
    }
    Ok(ready)
}

/// Moves the key space of `issuer` to `state`.
async fn set_state(
    tx: &mut PgConnection,
    issuer: &str,
    state: KeySpaceState,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE binding_key_space SET state = $2, fill_after = NULL, changed_at = now() \
         WHERE issuer = $1",
    )
    .bind(issuer)
    .bind(state.as_str())
    .execute(&mut *tx)
    .await?;
    tracing::info!(
        issuer,
        state = state.as_str(),
        "a binding key space changed state"
    );
    Ok(())
}

/// Deletes the key space of `issuer` and every key row under it.
async fn prune(tx: &mut PgConnection, issuer: &str) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_identifier WHERE selector = 'key' AND issuer = $1")
        .bind(issuer)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM binding_key_space WHERE issuer = $1")
        .bind(issuer)
        .execute(&mut *tx)
        .await?;
    tracing::info!(
        issuer,
        "a binding key space no live replica names went, its key rows with it"
    );
    Ok(())
}

/// Starts the online fill of the `pending` key space of `issuer` on `tx`,
/// which holds the fill lock, once every lease renewed within `ttl` lists it
/// with the flag `mapping` gives it: its key rows go, so the fill writes
/// each from the party versions, and it turns `filling`. Returns whether it
/// started.
///
/// # Errors
/// [`StorageError`] on a driver failure.
pub(crate) async fn start_fill(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    issuer: &str,
    ttl: Duration,
) -> Result<bool, StorageError> {
    let Some(&portal) = mapping.key_spaces().get(issuer) else {
        return Ok(false);
    };
    let leases = live(tx, ttl).await?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM binding_key_space WHERE issuer = $1 AND portal = $2 FOR UPDATE",
    )
    .bind(issuer)
    .bind(portal)
    .fetch_optional(&mut *tx)
    .await?;
    if state.as_deref() != Some(KeySpaceState::Pending.as_str())
        || leases.is_empty()
        || !leases.iter().all(|l| l.lists(issuer, portal))
    {
        return Ok(false);
    }
    gate_exclusive(tx).await?;
    sqlx::query("DELETE FROM binding_identifier WHERE selector = 'key' AND issuer = $1")
        .bind(issuer)
        .execute(&mut *tx)
        .await?;
    set_state(tx, issuer, KeySpaceState::Filling).await?;
    Ok(true)
}

/// What one page of the online fill came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    /// A page was written and more may follow.
    More,
    /// The last page was written and the key space is `active`.
    Done,
    /// The key space is no longer `filling` under `mapping`'s flag, so this
    /// replica writes no page of it.
    Stopped,
}

/// Writes the next page of the online fill of the `filling` key space of
/// `issuer` on `tx`, which holds the fill lock.
///
/// # Errors
/// [`StorageError`] on a driver failure, the unique violation of two
/// `PERSON`s carrying one key under `issuer` included.
pub(crate) async fn fill_page(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    issuer: &str,
) -> Result<Progress, StorageError> {
    let Some(&portal) = mapping.key_spaces().get(issuer) else {
        return Ok(Progress::Stopped);
    };
    let row = sqlx::query(
        "SELECT state, fill_after FROM binding_key_space \
         WHERE issuer = $1 AND portal = $2 FOR UPDATE",
    )
    .bind(issuer)
    .bind(portal)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Ok(Progress::Stopped);
    };
    let state: String = row.try_get("state")?;
    if state != KeySpaceState::Filling.as_str() {
        return Ok(Progress::Stopped);
    }
    let after: Option<Uuid> = row.try_get("fill_after")?;
    gate_exclusive(tx).await?;
    let Some(last) =
        crate::party_binding::store::fill_key_space(tx, mapping, issuer, after).await?
    else {
        set_state(tx, issuer, KeySpaceState::Active).await?;
        return Ok(Progress::Done);
    };
    sqlx::query("UPDATE binding_key_space SET fill_after = $2 WHERE issuer = $1")
        .bind(issuer)
        .bind(last)
        .execute(&mut *tx)
        .await?;
    Ok(Progress::More)
}

/// Moves the `filling` key space of `issuer` back to `pending` on `tx`, as a
/// page that found two `PERSON`s carrying one key under it does.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn stall(tx: &mut PgConnection, issuer: &str) -> Result<(), StorageError> {
    let filling: Option<String> = sqlx::query_scalar(
        "SELECT issuer FROM binding_key_space WHERE issuer = $1 AND state = 'filling' FOR UPDATE",
    )
    .bind(issuer)
    .fetch_optional(&mut *tx)
    .await?;
    if filling.is_some() {
        set_state(tx, issuer, KeySpaceState::Pending).await?;
    }
    Ok(())
}

/// The `filling` key spaces `mapping` names with their stored flag.
///
/// # Errors
/// [`StorageError`] on a driver failure.
pub(crate) async fn filling(
    pool: &PgPool,
    mapping: &BindingMapping,
) -> Result<Vec<String>, StorageError> {
    let spaces = mapping.key_spaces();
    Ok(list(pool)
        .await?
        .into_iter()
        .filter(|space| {
            space.state == KeySpaceState::Filling
                && spaces.get(&space.issuer) == Some(&space.portal)
        })
        .map(|space| space.issuer)
        .collect())
}

/// Whether `e` is the unique violation of a binding-key index.
pub(crate) fn key_conflict(e: &StorageError) -> bool {
    let StorageError::Database(source) = e else {
        return false;
    };
    source
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
}
