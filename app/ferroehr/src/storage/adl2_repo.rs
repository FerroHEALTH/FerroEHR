// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `adl2_artefact`, the ADL2 archetypes, templates and
//! operational templates keyed by their `ARCHETYPE_HRID`.
//!
//! Identity is case-insensitive while storage is case-preserving (BASE
//! `master05-identification_package.adoc` §Composite Identifiers and Case), so
//! every lookup compares `lower(hrid) = lower($1)`. The `kind` column holds
//! `archetype`, `template` or `operational_template`. No openEHR spec governs
//! the SQL: our own design.

use sqlx::{PgConnection, PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// The stored `kind` of a source archetype.
pub const ARCHETYPE_KIND: &str = "archetype";

/// The stored `kind` of an operational template, which inlines every
/// constituent it was compiled from.
pub const OPERATIONAL_TEMPLATE_KIND: &str = "operational_template";

/// Whether a stored `kind` is a template or an operational template, the kinds
/// whose HRID is an address a committed version may name.
#[must_use]
pub fn is_template_kind(kind: &str) -> bool {
    matches!(kind, "template" | OPERATIONAL_TEMPLATE_KIND)
}

/// One stored artefact's identity and kind.
#[derive(Debug, Clone)]
pub struct ArtefactKey {
    /// The stored `ARCHETYPE_HRID`, case-preserved.
    pub hrid: String,
    /// The stored `kind`.
    pub kind: String,
}

/// One stored artefact with its source.
#[derive(Debug, Clone)]
pub struct ArtefactRow {
    /// The stored `ARCHETYPE_HRID`.
    pub hrid: String,
    /// The stored `kind`.
    pub kind: String,
    /// The ADL2 source, stored verbatim.
    pub adl: String,
}

/// One validated artefact to store.
#[derive(Debug, Clone, Copy)]
pub struct NewArtefact<'a> {
    /// The artefact's `ARCHETYPE_HRID`.
    pub hrid: &'a str,
    /// The storage `kind`.
    pub kind: &'a str,
    /// The ADL2 source.
    pub adl: &'a str,
    /// The HRID of the archetype it specialises, when it specialises one.
    pub parent_hrid: Option<&'a str>,
}

/// Take the transaction-scoped advisory lock every ADL2 upload and delete
/// holds from its repository read to its commit.
///
/// The delete refuses while a stored artefact depends on its target, and an
/// upload is refused while a constituent it needs is not stored; each decides
/// on a read the other's write changes, so they run one after the other, the
/// second deciding on what the first committed. PostgreSQL releases the lock at
/// the end of the transaction (`pg_advisory_xact_lock`, PostgreSQL
/// documentation §13.3.5 Advisory Locks).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn write_lock(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('ferroehr.adl2_artefact', 0))")
        .execute(tx)
        .await?;
    Ok(())
}

/// Every stored ADL2 source, in HRID order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn sources(conn: &mut PgConnection) -> Result<Vec<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT adl FROM adl2_artefact ORDER BY hrid")
            .fetch_all(conn)
            .await?,
    )
}

/// Every stored artefact with its kind and source, in HRID order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn artefacts(conn: &mut PgConnection) -> Result<Vec<ArtefactRow>, StorageError> {
    let rows = sqlx::query("SELECT hrid, kind, adl FROM adl2_artefact ORDER BY hrid")
        .fetch_all(conn)
        .await?;
    rows.iter()
        .map(|row| {
            Ok(ArtefactRow {
                hrid: row.try_get("hrid")?,
                kind: row.try_get("kind")?,
                adl: row.try_get("adl")?,
            })
        })
        .collect()
}

/// The stored artefact whose HRID equals `hrid` case-insensitively, or
/// `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn find(
    conn: &mut PgConnection,
    hrid: &str,
) -> Result<Option<ArtefactKey>, StorageError> {
    let row = sqlx::query("SELECT hrid, kind FROM adl2_artefact WHERE lower(hrid) = lower($1)")
        .bind(hrid)
        .fetch_optional(conn)
        .await?;
    row.map(|row| {
        Ok(ArtefactKey {
            hrid: row.try_get("hrid")?,
            kind: row.try_get("kind")?,
        })
    })
    .transpose()
}

/// Store a validated artefact; `false`, with nothing written, when an artefact
/// is already stored under any case-variant of its HRID.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert(
    tx: &mut PgConnection,
    artefact: NewArtefact<'_>,
) -> Result<bool, StorageError> {
    let stored: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1))",
    )
    .bind(artefact.hrid)
    .fetch_one(&mut *tx)
    .await?;
    if stored {
        return Ok(false);
    }
    sqlx::query("INSERT INTO adl2_artefact (hrid, kind, adl, parent_hrid) VALUES ($1, $2, $3, $4)")
        .bind(artefact.hrid)
        .bind(artefact.kind)
        .bind(artefact.adl)
        .bind(artefact.parent_hrid)
        .execute(&mut *tx)
        .await?;
    Ok(true)
}

/// Delete the artefact stored under exactly `hrid`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete(tx: &mut PgConnection, hrid: &str) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM adl2_artefact WHERE hrid = $1")
        .bind(hrid)
        .execute(tx)
        .await?;
    Ok(())
}

/// Every stored HRID, in order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn hrids(pool: &PgPool) -> Result<Vec<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT hrid FROM adl2_artefact ORDER BY hrid")
            .fetch_all(&mut *acquire(pool).await?)
            .await?,
    )
}

/// The ADL2 source stored under `hrid`, or `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn adl(pool: &PgPool, hrid: &str) -> Result<Option<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT adl FROM adl2_artefact WHERE lower(hrid) = lower($1)")
            .bind(hrid)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?,
    )
}

/// The stored HRIDs of `kind` (every kind for `None`) in order, `offset` rows
/// skipped and at most `limit` returned (`None` returns every row).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn list_hrids(
    pool: &PgPool,
    kind: Option<&str>,
    offset: i64,
    limit: Option<i64>,
) -> Result<Vec<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT hrid FROM adl2_artefact WHERE ($1::text IS NULL OR kind = $1) \
         ORDER BY hrid OFFSET $2 LIMIT $3",
    )
    .bind(kind)
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// How many artefacts of `kind` (every kind for `None`) are stored.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn count(pool: &PgPool, kind: Option<&str>) -> Result<i64, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM adl2_artefact WHERE ($1::text IS NULL OR kind = $1)",
    )
    .bind(kind)
    .fetch_one(&mut *acquire(pool).await?)
    .await?)
}

/// The HRID and storage instant of every stored template and operational
/// template, in HRID order, `offset` rows skipped and at most `limit` returned.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn templates(
    pool: &PgPool,
    offset: i64,
    limit: Option<i64>,
) -> Result<Vec<(String, jiff::Timestamp)>, StorageError> {
    let rows = sqlx::query(
        "SELECT hrid, created_at FROM adl2_artefact \
         WHERE kind IN ('template', 'operational_template') \
         ORDER BY hrid OFFSET $1 LIMIT $2",
    )
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("hrid")?,
                row.try_get::<jiff_sqlx::Timestamp, _>("created_at")?
                    .to_jiff(),
            ))
        })
        .collect()
}

/// Every stored `(hrid, parent_hrid)` specialisation edge.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn parent_edges(pool: &PgPool) -> Result<Vec<(String, String)>, StorageError> {
    Ok(
        sqlx::query_as("SELECT hrid, parent_hrid FROM adl2_artefact WHERE parent_hrid IS NOT NULL")
            .fetch_all(&mut *acquire(pool).await?)
            .await?,
    )
}
