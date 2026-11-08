// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `template_store`, the OPT 1.4 operational templates keyed by
//! their wire `template_id`, and for `template_ref`, the registry of every
//! template address a committed version may name.
//!
//! Identity is case-insensitive while storage is case-preserving (BASE
//! `master05-identification_package.adoc` §Composite Identifiers and Case), so
//! every lookup compares `lower(template_id) = lower($1)`. No openEHR spec
//! governs the SQL: our own design.

use sqlx::Connection as _;
use sqlx::{PgConnection, PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// One `template_store` row, without its XML content.
#[derive(Debug, Clone)]
pub struct TemplateRow {
    /// The wire `template_id`, case-preserved.
    pub template_id: String,
    /// The template's `concept`, when one was stored.
    pub concept: Option<String>,
    /// The root archetype id, when one was stored.
    pub root_archetype: Option<String>,
    /// When the template was stored.
    pub created_at: jiff::Timestamp,
}

/// One operational template to store.
#[derive(Debug, Clone, Copy)]
pub struct NewTemplate<'a> {
    /// The wire `template_id`.
    pub template_id: &'a str,
    /// The template's `concept`.
    pub concept: Option<&'a str>,
    /// The root archetype id.
    pub root_archetype: Option<&'a str>,
    /// The canonical OPT 1.4 XML, stored verbatim.
    pub content: &'a str,
}

fn template_row(row: &sqlx::postgres::PgRow) -> Result<TemplateRow, StorageError> {
    Ok(TemplateRow {
        template_id: row.try_get("template_id")?,
        concept: row.try_get("concept")?,
        root_archetype: row.try_get("root_archetype")?,
        created_at: row
            .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
            .to_jiff(),
    })
}

/// Store an operational template and register its wire address in
/// `template_ref`, in one transaction.
///
/// The insert is arbitrated on the case-insensitive unique index
/// (`lower(template_id)`), so an exact or case-variant duplicate inserts
/// nothing and returns `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn insert_template(
    pool: &PgPool,
    template: NewTemplate<'_>,
) -> Result<Option<TemplateRow>, StorageError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    let Some(row) = sqlx::query(
        "INSERT INTO template_store (template_id, concept, root_archetype, content) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (lower(template_id)) DO NOTHING \
         RETURNING template_id, concept, root_archetype, created_at",
    )
    .bind(template.template_id)
    .bind(template.concept)
    .bind(template.root_archetype)
    .bind(template.content)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    register_template_ref(&mut tx, template.template_id).await?;
    tx.commit().await?;
    template_row(&row).map(Some)
}

/// The stored OPT 1.4 XML of `template_id`, or `None` when none is stored.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn template_xml(
    pool: &PgPool,
    template_id: &str,
) -> Result<Option<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT content FROM template_store WHERE lower(template_id) = lower($1)",
    )
    .bind(template_id)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?)
}

/// Whether an OPT 1.4 template with this id is stored, without moving its XML.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn template_exists(pool: &PgPool, template_id: &str) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM template_store WHERE lower(template_id) = lower($1))",
    )
    .bind(template_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?)
}

/// Whether `template_id` is registered in `template_ref`, compared
/// case-insensitively: an address a committed version may name.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn template_ref_exists(pool: &PgPool, template_id: &str) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM template_ref WHERE lower(template_id) = lower($1))",
    )
    .bind(template_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?)
}

/// Every stored template's row, ordered by `template_id`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure or a column that does not
/// decode.
pub async fn list_templates(pool: &PgPool) -> Result<Vec<TemplateRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT template_id, concept, root_archetype, created_at \
         FROM template_store ORDER BY template_id",
    )
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter().map(template_row).collect()
}

/// Register `template_id` in `template_ref`, the target of the
/// `version.template_id` foreign key; an address already registered by
/// either template dialect stays as it is.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn register_template_ref(
    tx: &mut PgConnection,
    template_id: &str,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO template_ref (template_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(template_id)
        .execute(tx)
        .await?;
    Ok(())
}

/// Deregister `template_id` from `template_ref` unless an OPT 1.4 template or
/// a template-kind ADL2 artefact still claims it, so the registry stays the
/// union of both dialects' addresses.
///
/// The `version.template_id` foreign key refuses the delete when a commit
/// referenced the address after its caller counted the references.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn release_template_ref(
    tx: &mut PgConnection,
    template_id: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "DELETE FROM template_ref WHERE template_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM template_store WHERE lower(template_id) = lower($1)) \
         AND NOT EXISTS (SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1) \
                         AND kind IN ('template', 'operational_template'))",
    )
    .bind(template_id)
    .execute(tx)
    .await?;
    Ok(())
}

/// How many committed versions name exactly `template_id`, the stored
/// case-preserved address, over both storage tiers (`version` is one relation
/// partitioned by tier).
///
/// A version names its template through the `template_ref` foreign key, so it
/// carries the address exactly as the store registered it, and the exact
/// comparison reads the `template_id` index.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn template_use_count(
    tx: &mut PgConnection,
    template_id: &str,
) -> Result<i64, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM version WHERE template_id = $1")
            .bind(template_id)
            .fetch_one(tx)
            .await?,
    )
}

/// The stored, case-preserved `template_id` of the OPT 1.4 template whose id
/// equals `template_id` case-insensitively, or `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn stored_template_id(
    tx: &mut PgConnection,
    template_id: &str,
) -> Result<Option<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT template_id FROM template_store WHERE lower(template_id) = lower($1)",
    )
    .bind(template_id)
    .fetch_optional(tx)
    .await?)
}

/// Delete the OPT 1.4 template stored under exactly `template_id`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_template(tx: &mut PgConnection, template_id: &str) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM template_store WHERE template_id = $1")
        .bind(template_id)
        .execute(tx)
        .await?;
    Ok(())
}
