// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A version read that carries what the access evaluator judges its object
//! by, so a read whose item the evaluator decides is one statement (#5773).
//!
//! The statement reads the object's head row and, beside it, the addressed
//! version as [`super::read::read_one`] reads it, so the facts arrive even
//! when the object holds no such version and the evaluator judges the object
//! the request names exactly as it would before any read. Nothing here judges:
//! the caller judges the facts, then decides what of the version leaves the
//! server. No openEHR spec governs this: our own design.

use sqlx::{PgPool, Row};

use crate::db::acquire::acquire;
use crate::ids::VoId;
use crate::storage::access_facts::HeadFacts;
use crate::storage::error::StorageError;
use crate::storage::version_repo::read::{StoredVersion, stored_version_raw};
use crate::versioning::object_version_id::TreeId;

/// The head facts of the object `$1` names beside the version `$tail`
/// selects, `NULL` columns of the version when it holds none; the settings
/// revision is the version's (the head's EHR is the version's).
macro_rules! judged_select {
    ($tail:literal) => {
        concat!(
            "SELECT ",
            head_object_facts!(),
            "r.* FROM vo_head h LEFT JOIN LATERAL (",
            version_select!($tail),
            ") r ON true WHERE h.vo_id = $1"
        )
    };
}

/// The statement [`read_judged`] runs for [`JudgedAt::Current`].
pub const JUDGED_CURRENT_SQL: &str =
    judged_select!("WHERE v.vo_id = $1 AND h.trunk_head_sys_version = v.sys_version");

/// The statement [`read_judged`] runs for [`JudgedAt::Tree`].
pub const JUDGED_TREE_SQL: &str = judged_select!(
    "WHERE v.vo_id = $1 AND v.trunk_version = $2 \
     AND v.branch_number = $3 AND v.branch_version = $4"
);

/// The version of an object a judged read addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgedAt {
    /// The current trunk version.
    Current(VoId),
    /// The version at a `VERSION_TREE_ID`.
    Tree(VoId, TreeId),
}

/// The head facts of the object `at` names and the version it addresses.
///
/// Unjudged, with the body as the stored text; `None` when the store holds no
/// such object, and no version beside the facts when the object holds no
/// such version.
///
/// # Errors
/// [`StorageError`] on a driver failure or a column that does not decode.
pub async fn read_judged(
    pool: &PgPool,
    at: JudgedAt,
) -> Result<Option<(HeadFacts, Option<StoredVersion>)>, StorageError> {
    let (vo_id, query) = match at {
        JudgedAt::Current(vo_id) => (vo_id, sqlx::query(JUDGED_CURRENT_SQL).bind(vo_id)),
        JudgedAt::Tree(vo_id, tree) => {
            let (trunk, branch, branch_version) = tree.columns();
            let query = sqlx::query(JUDGED_TREE_SQL)
                .bind(vo_id)
                .bind(trunk)
                .bind(branch)
                .bind(branch_version);
            (vo_id, query)
        }
    };
    let Some(row) = query.fetch_optional(&mut *acquire(pool).await?).await? else {
        return Ok(None);
    };
    let facts = HeadFacts::read(&row, vo_id)?;
    let stored = match row.try_get::<Option<VoId>, _>("vo_id")? {
        Some(held) => Some(stored_version_raw(held, &row)?),
        None => None,
    };
    Ok(Some((facts, stored)))
}
