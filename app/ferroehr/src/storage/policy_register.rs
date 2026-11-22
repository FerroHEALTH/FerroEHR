// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers storage keeps: one append-only row
//! per version of each register, with its committer.
//!
//! A register with no row holds its default, which is version `0`. A write
//! names the version it replaces and appends the next one, so two writers that
//! read the same version cannot both win: the primary key refuses the second.
//! No openEHR spec governs operational policy: our own design/extension.

use sqlx::{PgConnection, PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// One stored version of a register.
#[derive(Debug, Clone)]
pub struct RegisterRow {
    /// The version, from `1`.
    pub version: i64,
    /// The register document, as JSON text.
    pub document: String,
    /// Who committed the version.
    pub committer: String,
    /// When it was committed.
    pub committed_at: jiff::Timestamp,
}

/// The revisions a request confirms its policy and definition copies with,
/// read in one statement of two one-row primary-key reads (#6251, #5948).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Revisions {
    /// The revision of the policy registers and the access policy,
    /// `policy_revision`.
    pub policy: i64,
    /// The revision of the definition stores, `definition_revision`.
    pub definitions: i64,
}

/// The policy and definition revisions in force in the store.
///
/// The one read a request confirms its copies with: a copy confirmed at the
/// policy revision read holds every change committed up to it (#6251).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn revisions(pool: &PgPool) -> Result<Revisions, StorageError> {
    let (policy, definitions): (i64, i64) = sqlx::query_as(
        "SELECT p.revision, d.revision FROM policy_revision p CROSS JOIN definition_revision d \
         WHERE p.singleton AND d.singleton",
    )
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    Ok(Revisions {
        policy,
        definitions,
    })
}

/// The newest stored policy versions and the policy revision they are at,
/// read at one snapshot (#6251).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyHeads {
    /// The newest version of every register that has one, as `(register,
    /// version)`.
    pub registers: Vec<(String, i64)>,
    /// The newest trunk version of the access policy, `0` when it has none.
    pub access_policy: i32,
    /// The `policy_revision` the versions are at.
    pub revision: i64,
}

/// The newest policy versions and the policy revision, in one statement.
///
/// Read only when a request found the policy revision past the one its copies
/// were confirmed at ([`revisions`]): one snapshot holds both, so copies
/// brought to these versions hold every change up to `revision`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn policy_heads(pool: &PgPool) -> Result<PolicyHeads, StorageError> {
    policy_heads_on(&mut *acquire(pool).await?).await
}

/// [`policy_heads`] on `conn`.
///
/// Inside a policy write's transaction, after its write moved
/// `policy_revision`, it reads the versions and the revision that transaction
/// produced, every earlier writer's change among them (#6265).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn policy_heads_on(conn: &mut PgConnection) -> Result<PolicyHeads, StorageError> {
    let rows = sqlx::query(
        "SELECT register, max(version) AS version, NULL::integer AS policy, \
                NULL::bigint AS revision \
           FROM policy_register GROUP BY register \
         UNION ALL \
         SELECT NULL, NULL, max(version), NULL FROM access_policy_version \
         UNION ALL \
         SELECT NULL, NULL, NULL, revision FROM policy_revision WHERE singleton",
    )
    .fetch_all(conn)
    .await?;
    let mut heads = PolicyHeads {
        registers: Vec::with_capacity(rows.len()),
        ..PolicyHeads::default()
    };
    for row in &rows {
        if let Some(register) = row.try_get::<Option<String>, _>("register")? {
            heads.registers.push((register, row.try_get("version")?));
        } else if let Some(revision) = row.try_get::<Option<i64>, _>("revision")? {
            heads.revision = revision;
        } else {
            heads.access_policy = row.try_get::<Option<i32>, _>("policy")?.unwrap_or(0);
        }
    }
    Ok(heads)
}

/// The stored version `version` of `register`, or `None` when it has none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn at(
    pool: &PgPool,
    register: &str,
    version: i64,
) -> Result<Option<RegisterRow>, StorageError> {
    let row = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 AND version = $2",
    )
    .bind(register)
    .bind(version)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    row.as_ref().map(read_row).transpose()
}

/// The newest stored version of `register`, or `None` when it has none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn newest(pool: &PgPool, register: &str) -> Result<Option<RegisterRow>, StorageError> {
    let row = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(register)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    row.as_ref().map(read_row).transpose()
}

/// Every stored version of `register`, newest first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn history(pool: &PgPool, register: &str) -> Result<Vec<RegisterRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 ORDER BY version DESC",
    )
    .bind(register)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter().map(read_row).collect()
}

/// Append version `replaces + 1` of `register` on `conn`, when `replaces` is
/// still the newest version; `None` when it is not.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn append(
    conn: &mut PgConnection,
    register: &str,
    replaces: i64,
    document: &str,
    committer: &str,
) -> Result<Option<i64>, StorageError> {
    // NOTE: the advisory lock serializes writers of one register, so the version check holds.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('policy_register:' || $1))")
        .bind(register)
        .execute(&mut *conn)
        .await?;
    let newest: Option<i64> =
        sqlx::query_scalar("SELECT max(version) FROM policy_register WHERE register = $1")
            .bind(register)
            .fetch_one(&mut *conn)
            .await?;
    if newest.unwrap_or(0) != replaces {
        return Ok(None);
    }
    let Some(version) = replaces.checked_add(1) else {
        return Ok(None);
    };
    sqlx::query(
        "INSERT INTO policy_register (register, version, document, committer) \
         VALUES ($1, $2, $3::jsonb, $4)",
    )
    .bind(register)
    .bind(version)
    .bind(document)
    .bind(committer)
    .execute(&mut *conn)
    .await?;
    Ok(Some(version))
}

/// One register row read off the database.
fn read_row(row: &sqlx::postgres::PgRow) -> Result<RegisterRow, StorageError> {
    let committed_at: jiff_sqlx::Timestamp = row.try_get("committed_at")?;
    Ok(RegisterRow {
        version: row.try_get("version")?,
        document: row.try_get("document")?,
        committer: row.try_get("committer")?,
        committed_at: committed_at.to_jiff(),
    })
}
