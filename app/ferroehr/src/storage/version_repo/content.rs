// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content identities of stored versions, for the access record: the
//! `(EHR, kind, template id, root archetype id)` tuples the category
//! classification reads (#3621).
//!
//! No openEHR spec governs the SQL: our own design.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// One distinct identity tuple of a served version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedContent {
    /// The owning EHR, `None` for EHR-less content.
    pub ehr_id: Option<EhrId>,
    /// The RM kind of the versioned object.
    pub kind: String,
    /// The `template_id` the version was committed against.
    pub template_id: Option<String>,
    /// The archetype id of the version's root node, as the composition wrote it.
    pub root_archetype: Option<String>,
}

/// The content identities of the latest version of each object in `vo_ids`
/// that carries content, distinct.
///
/// A logically deleted head (NULL `body`, RM common master06 §Logical
/// Deletion) is read through to the newest version before it that has a body,
/// so a delete and a container read are classified by the data they address.
///
/// # Errors
/// [`StorageError::Database`] when the read fails.
pub async fn content_of_objects(
    conn: &mut sqlx::PgConnection,
    vo_ids: &[VoId],
) -> Result<Vec<ServedContent>, StorageError> {
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = vo_ids.iter().map(|vo_id| vo_id.0).collect();
    let rows = sqlx::query(
        "SELECT DISTINCT c.ehr_id, c.kind, c.template_id, c.archetype \
         FROM unnest($1::uuid[]) AS a(vo_id) \
         CROSS JOIN LATERAL ( \
             SELECT v.ehr_id, v.kind, v.template_id, COALESCE(n.data->>'archetype_node_id', ext.node_archetype_node_id(n.archetype, n.rm_version)) AS archetype \
             FROM version v \
             LEFT JOIN node n ON n.tier = v.tier AND n.vo_id = v.vo_id \
                  AND n.sys_version = v.content_key AND n.num = 0 \
             WHERE v.vo_id = a.vo_id AND v.body IS NOT NULL \
             ORDER BY v.sys_version DESC LIMIT 1 \
         ) c \
         ORDER BY c.ehr_id, c.kind, c.template_id, c.archetype",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter().map(served_content_row).collect()
}

/// The content identities of the versions one CONTRIBUTION committed,
/// distinct, each logical delete read through to the version it deleted.
///
/// # Errors
/// [`StorageError::Database`] when the read fails.
pub async fn content_of_contribution(
    pool: &PgPool,
    contribution_id: Uuid,
) -> Result<Vec<ServedContent>, StorageError> {
    let rows = sqlx::query(
        "SELECT DISTINCT v.ehr_id, v.kind, c.template_id, c.archetype \
         FROM version v \
         LEFT JOIN LATERAL ( \
             SELECT p.template_id, COALESCE(n.data->>'archetype_node_id', ext.node_archetype_node_id(n.archetype, n.rm_version)) AS archetype \
             FROM version p \
             LEFT JOIN node n ON n.tier = p.tier AND n.vo_id = p.vo_id \
                  AND n.sys_version = p.content_key AND n.num = 0 \
             WHERE p.vo_id = v.vo_id AND p.sys_version <= v.sys_version \
               AND p.body IS NOT NULL \
             ORDER BY p.sys_version DESC LIMIT 1 \
         ) c ON true \
         WHERE v.contribution_id = $1 \
         ORDER BY v.ehr_id, v.kind, c.template_id, c.archetype",
    )
    .bind(contribution_id)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter().map(served_content_row).collect()
}

/// Decodes one `(ehr_id, kind, template_id, archetype)` row.
fn served_content_row(row: &PgRow) -> Result<ServedContent, StorageError> {
    Ok(ServedContent {
        ehr_id: row.try_get::<Option<Uuid>, _>("ehr_id")?.map(EhrId),
        kind: row.try_get("kind")?,
        template_id: row.try_get("template_id")?,
        root_archetype: row.try_get("archetype")?,
    })
}
