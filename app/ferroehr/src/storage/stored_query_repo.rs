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

use openehr_its::rest::stored_query::{
    QualifiedQueryName, QueryNamePattern, QueryVersion, VersionPattern,
};
use sqlx::Connection as _;
use sqlx::{PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// The namespace a name without one is stored under.
///
/// NOTE: ITS-REST `Qualified_query_name.md` makes the namespace optional and names no default:
/// our own design, one key per query so `q` and `misc::q` address the same row.
const DEFAULT_NAMESPACE: &str = "misc";

/// The `(reverse_domain_name, semantic_id)` key `name` is stored under.
fn key(name: &QualifiedQueryName) -> (&str, &str) {
    (name.namespace().unwrap_or(DEFAULT_NAMESPACE), name.name())
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
    name: &QualifiedQueryName,
    version: QueryVersion,
    query_text: &str,
) -> Result<(), StorageError> {
    let (reverse_domain_name, semantic_id) = key(name);
    let version = version.to_string();
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    sqlx::query(
        "DELETE FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
         AND lower(semantic_id) = lower($2) AND semver = $3",
    )
    .bind(reverse_domain_name)
    .bind(semantic_id)
    .bind(&version)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO stored_query \
         (reverse_domain_name, semantic_id, semver, query_type, query_text) \
         VALUES ($1, $2, $3, 'AQL', $4)",
    )
    .bind(reverse_domain_name)
    .bind(semantic_id)
    .bind(&version)
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
    name: &QualifiedQueryName,
    version: QueryVersion,
    query_text: &str,
) -> Result<bool, StorageError> {
    let (reverse_domain_name, semantic_id) = key(name);
    let version = version.to_string();
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
         AND lower(semantic_id) = lower($2) AND semver = $3)",
    )
    .bind(reverse_domain_name)
    .bind(semantic_id)
    .bind(&version)
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
    .bind(reverse_domain_name)
    .bind(semantic_id)
    .bind(&version)
    .bind(query_text)
    .execute(&mut *acquire(pool).await?)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}

/// The stored query of `name` at the highest version `version` selects, or
/// at the highest stored version when `version` is `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn find(
    pool: &PgPool,
    name: &QualifiedQueryName,
    version: Option<&VersionPattern>,
) -> Result<Option<StoredQueryRow>, StorageError> {
    let (reverse_domain_name, semantic_id) = key(name);
    let query = match version {
        Some(VersionPattern::Exact(exact)) => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             AND semver = $3",
        )
        .bind(reverse_domain_name)
        .bind(semantic_id)
        .bind(exact.to_string()),
        // `left(...)` rather than LIKE, so no pattern metacharacter needs escaping.
        Some(prefix) => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             AND left(semver, length($3)) = $3 \
             ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
        )
        .bind(reverse_domain_name)
        .bind(semantic_id)
        .bind(format!("{prefix}.")),
        None => sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
             created_at FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
        )
        .bind(reverse_domain_name)
        .bind(semantic_id),
    };
    query
        .fetch_optional(&mut *acquire(pool).await?)
        .await?
        .as_ref()
        .map(stored_query_row)
        .transpose()
}

/// Every stored version of every query `pattern` matches, case-insensitively,
/// or of every query when it is `None`.
///
/// A pattern without `::` is a prefix of the qualified name, and also matches
/// under the default namespace; one with `::` matches the namespace it names
/// (any, when empty) and the query names its name starts with. The rows are
/// ordered by name, then numerically by version.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn list(
    pool: &PgPool,
    pattern: Option<&QueryNamePattern>,
) -> Result<Vec<StoredQueryRow>, StorageError> {
    // NOTE: PostgreSQL 18 §9.4 `left`, each part is compared as a bound value, so `%` and `_`
    // in a pattern match themselves, as no `LIKE` reads them.
    let mut connection = acquire(pool).await?;
    let rows = if let Some(pattern) = pattern.filter(|pattern| pattern.is_qualified()) {
        sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             WHERE ($1::text IS NULL OR lower(reverse_domain_name) = lower($1)) \
               AND left(lower(semantic_id), length($2)) = lower($2) \
             ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[]",
        )
        .bind(pattern.namespace())
        .bind(pattern.name())
        .fetch_all(&mut *connection)
        .await?
    } else {
        let prefix = pattern.map(QueryNamePattern::name).unwrap_or_default();
        let defaulted = pattern.map(|pattern| format!("{DEFAULT_NAMESPACE}::{}", pattern.name()));
        sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             WHERE left(lower(CASE WHEN reverse_domain_name = '' THEN semantic_id \
                         ELSE reverse_domain_name || '::' || semantic_id END), \
                    length($1)) = lower($1) \
                OR left(lower(reverse_domain_name || '::' || semantic_id), \
                    length($2)) = lower($2) \
             ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[]",
        )
        .bind(prefix)
        .bind(defaulted)
        .fetch_all(&mut *connection)
        .await?
    };
    rows.iter().map(stored_query_row).collect()
}

/// Delete the stored query of `name` at exactly `version`; `false` when none
/// was stored.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete(
    pool: &PgPool,
    name: &QualifiedQueryName,
    version: QueryVersion,
) -> Result<bool, StorageError> {
    let (reverse_domain_name, semantic_id) = key(name);
    let version = version.to_string();
    let deleted = sqlx::query(
        "DELETE FROM stored_query \
         WHERE lower(reverse_domain_name) = lower($1) \
           AND lower(semantic_id) = lower($2) AND semver = $3",
    )
    .bind(reverse_domain_name)
    .bind(semantic_id)
    .bind(&version)
    .execute(&mut *acquire(pool).await?)
    .await?
    .rows_affected();
    Ok(deleted > 0)
}
