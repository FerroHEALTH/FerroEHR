// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `stored_query`, the registered queries keyed by their
//! `(reverse_domain_name, semantic_id)` name and their SEMVER.
//!
//! The name compares case-insensitively and the version exactly (BASE
//! `master05-identification_package.adoc` §Composite Identifiers and Case).
//! Versions order numerically segment by segment
//! (`string_to_array(semver, '.')::int[]`). No openEHR spec governs the SQL:
//! our own design.

use sqlx::Connection as _;
use sqlx::{PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// The two-part name a stored query is keyed by.
#[derive(Debug, Clone, Copy)]
pub struct QueryName<'a> {
    /// The namespace segment.
    pub reverse_domain_name: &'a str,
    /// The query-name segment.
    pub semantic_id: &'a str,
}

/// Which stored version of a name to read.
#[derive(Debug, Clone, Copy)]
pub enum VersionMatch<'a> {
    /// Exactly this version.
    Exact(&'a str),
    /// The highest version equal to this prefix or extending it on a dot
    /// boundary (`1` matches `1.x.y`, `1.0` matches `1.0.x`).
    Prefix(&'a str),
    /// The highest stored version.
    Latest,
}

/// One `stored_query` row.
#[derive(Debug, Clone)]
pub struct StoredQueryRow {
    /// The namespace segment, case-preserved.
    pub reverse_domain_name: String,
    /// The query-name segment, case-preserved.
    pub semantic_id: String,
    /// The SEMVER the query is stored at.
    pub semver: String,
    /// The query formalism.
    pub query_type: String,
    /// The query text.
    pub query_text: String,
    /// When the query was stored.
    pub created_at: jiff::Timestamp,
}

fn stored_query_row(row: &sqlx::postgres::PgRow) -> Result<StoredQueryRow, StorageError> {
    Ok(StoredQueryRow {
        reverse_domain_name: row.try_get("reverse_domain_name")?,
        semantic_id: row.try_get("semantic_id")?,
        semver: row.try_get("semver")?,
        query_type: row.try_get("query_type")?,
        query_text: row.try_get("query_text")?,
        created_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
            .to_jiff(),
    })
}

/// Store the AQL `query_text` under `name` at `version`, replacing the text a
/// case-variant of the name holds at that version, in one transaction.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace(
    pool: &PgPool,
    name: QueryName<'_>,
    version: &str,
    query_text: &str,
) -> Result<(), StorageError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(
        "DELETE FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
         AND lower(semantic_id) = lower($2) AND semver = $3",
    )
    .bind(name.reverse_domain_name)
    .bind(name.semantic_id)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO stored_query \
         (reverse_domain_name, semantic_id, semver, query_type, query_text) \
         VALUES ($1, $2, $3, 'AQL', $4)",
    )
    .bind(name.reverse_domain_name)
    .bind(name.semantic_id)
    .bind(version)
    .bind(query_text)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Store the AQL `query_text` under `name` at `version` unless any
/// case-variant of the name is stored at that version; `false`, with nothing
/// written, when one is.
///
/// The exact-case insert is race-safe through `ON CONFLICT DO NOTHING` on the
/// `(reverse_domain_name, semantic_id, semver)` key.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert(
    pool: &PgPool,
    name: QueryName<'_>,
    version: &str,
    query_text: &str,
) -> Result<bool, StorageError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
         AND lower(semantic_id) = lower($2) AND semver = $3)",
    )
    .bind(name.reverse_domain_name)
    .bind(name.semantic_id)
    .bind(version)
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    if exists {
        return Ok(false);
    }
    let inserted = sqlx::query(
        "INSERT INTO stored_query \
         (reverse_domain_name, semantic_id, semver, query_type, query_text) \
         VALUES ($1, $2, $3, 'AQL', $4) \
         ON CONFLICT (reverse_domain_name, semantic_id, semver) DO NOTHING",
    )
    .bind(name.reverse_domain_name)
    .bind(name.semantic_id)
    .bind(version)
    .bind(query_text)
    .execute(&mut *acquire(pool).await?)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}

/// The stored query of `name` at the version `version` selects, or `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn find(
    pool: &PgPool,
    name: QueryName<'_>,
    version: VersionMatch<'_>,
) -> Result<Option<StoredQueryRow>, StorageError> {
    let query = match version {
        // `left(...)` rather than LIKE, so no pattern metacharacter needs escaping.
        VersionMatch::Prefix(v) => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             AND (semver = $3 OR left(semver, length($3) + 1) = $3 || '.') \
             ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
        )
        .bind(name.reverse_domain_name)
        .bind(name.semantic_id)
        .bind(v),
        VersionMatch::Exact(v) => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             AND semver = $3",
        )
        .bind(name.reverse_domain_name)
        .bind(name.semantic_id)
        .bind(v),
        VersionMatch::Latest => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
        )
        .bind(name.reverse_domain_name)
        .bind(name.semantic_id),
    };
    query
        .fetch_optional(&mut *acquire(pool).await?)
        .await?
        .as_ref()
        .map(stored_query_row)
        .transpose()
}

/// Every stored version of every query whose qualified name starts with
/// `name_prefix`, case-insensitively.
///
/// A prefix with no `::` also matches under the `misc` namespace. The rows are
/// ordered by name, then numerically by version.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn list(pool: &PgPool, name_prefix: &str) -> Result<Vec<StoredQueryRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
         FROM stored_query \
         WHERE left(lower(CASE WHEN reverse_domain_name = '' THEN semantic_id \
                         ELSE reverse_domain_name || '::' || semantic_id END), \
                    length($1)) = lower($1) \
            OR ($1 NOT LIKE '%::%' AND \
                left(lower(reverse_domain_name || '::' || semantic_id), \
                     length('misc::' || $1)) = lower('misc::' || $1)) \
         ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[]",
    )
    .bind(name_prefix)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter().map(stored_query_row).collect()
}

/// Delete the stored query of `name` at exactly `version`; `false` when none
/// was stored.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete(
    pool: &PgPool,
    name: QueryName<'_>,
    version: &str,
) -> Result<bool, StorageError> {
    let deleted = sqlx::query(
        "DELETE FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
           AND lower(semantic_id) = lower($2) AND semver = $3",
    )
    .bind(name.reverse_domain_name)
    .bind(name.semantic_id)
    .bind(version)
    .execute(&mut *acquire(pool).await?)
    .await?
    .rows_affected();
    Ok(deleted > 0)
}
