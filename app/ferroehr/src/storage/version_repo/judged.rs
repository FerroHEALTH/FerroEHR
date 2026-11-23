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
/// revision is the version's (the head's EHR is the version's). The head's
/// ordinal and the version's content digest are what a copy of the version
/// is confirmed against later ([`read_marks`], #5778).
macro_rules! judged_select {
    ($tail:literal) => {
        judged_select!(""; $tail)
    };
    ($head:expr; $tail:literal) => {
        concat!(
            "SELECT ",
            $head,
            head_object_facts!(),
            "h.head_sys_version AS held_head, ",
            "(SELECT d.content_digest FROM version d WHERE d.tier = h.tier AND d.vo_id = r.vo_id ",
            "AND d.sys_version = r.sys_version) AS held_digest, ",
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

/// [`JUDGED_CURRENT_SQL`] with the party store's revisions, for a ride.
pub const JUDGED_CURRENT_RIDE_SQL: &str = judged_select!(party_revisions!("$2");
    "WHERE v.vo_id = $1 AND h.trunk_head_sys_version = v.sys_version");

/// [`JUDGED_TREE_SQL`] with the party store's revisions, for a ride.
pub const JUDGED_TREE_RIDE_SQL: &str = judged_select!(party_revisions!("$5");
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

/// What a judged read found: the object's head facts, the version it
/// addresses, and what a copy of that version is confirmed against.
#[derive(Debug, Clone)]
pub struct Judged {
    /// The facts the evaluator judges the object by.
    pub facts: HeadFacts,
    /// The version, its body as the stored text; `None` when the object
    /// holds no such version.
    pub stored: Option<StoredVersion>,
    /// The ordinal of the object's head, which every new version moves.
    pub head: i32,
    /// The version's content digest, which an erasure that rewrites the body
    /// changes; `None` beside no version.
    pub digest: Option<Vec<u8>>,
}

/// The head facts of the object `at` names and the version it addresses.
///
/// Unjudged, with the body as the stored text; `None` when the store holds no
/// such object, and no version beside the facts when the object holds no
/// such version.
///
/// # Errors
/// [`StorageError`] on a driver failure or a column that does not decode.
pub async fn read_judged(pool: &PgPool, at: JudgedAt) -> Result<Option<Judged>, StorageError> {
    let ride = crate::party_binding::request::reads_door();
    let door = crate::party_binding::request::door();
    let door = door.as_deref().map(crate::db::snapshot::Snapshot::text);
    let (vo_id, query) = match at {
        JudgedAt::Current(vo_id) => {
            let query = if ride {
                sqlx::query(JUDGED_CURRENT_RIDE_SQL).bind(vo_id).bind(door)
            } else {
                sqlx::query(JUDGED_CURRENT_SQL).bind(vo_id)
            };
            (vo_id, query)
        }
        JudgedAt::Tree(vo_id, tree) => {
            let (trunk, branch, branch_version) = tree.columns();
            let sql = if ride {
                JUDGED_TREE_RIDE_SQL
            } else {
                JUDGED_TREE_SQL
            };
            let query = sqlx::query(sql)
                .bind(vo_id)
                .bind(trunk)
                .bind(branch)
                .bind(branch_version);
            (vo_id, if ride { query.bind(door) } else { query })
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
    Ok(Some(Judged {
        facts,
        stored,
        head: row.try_get("held_head")?,
        digest: row.try_get("held_digest")?,
    }))
}

/// The statement [`read_marks`] runs, `$head` leading its select list.
macro_rules! marks_select {
    ($head:expr) => {
        concat!(
            "SELECT ",
            $head,
            "h.head_sys_version AS held_head, h.restricted_at, ",
            "(h.person_withheld_until > now()) IS TRUE AS person_withheld, ",
            "(SELECT d.content_digest FROM version d WHERE d.tier = h.tier AND d.vo_id = h.vo_id ",
            "AND d.sys_version = $2) AS held_digest, ",
            access_revision!("h.ehr_id"),
            " FROM vo_head h WHERE h.vo_id = $1"
        )
    };
}

/// The statement [`read_marks`] runs.
pub const MARKS_SQL: &str = marks_select!("");

/// [`MARKS_SQL`] with the party store's revisions, for a ride.
pub const MARKS_RIDE_SQL: &str = marks_select!(party_revisions!("$3"));

/// What may have changed about a version since it was read: everything a
/// read decides by that a later commit can change while the version itself
/// stays (RM common master06 §Version Identification).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marks {
    /// The ordinal of the object's head.
    pub head: i32,
    /// The restriction-of-processing mark (GDPR Art. 18(2)).
    pub restricted_at: Option<jiff::Timestamp>,
    /// Whether an EHDS Art. 3(3) delay withholds the object from the person.
    pub person_withheld: bool,
    /// The version's content digest; `None` when it was erased.
    pub digest: Option<Vec<u8>>,
    /// The revision of the owning EHR's settings row.
    pub revision: crate::storage::access_projection::Revision,
}

/// The marks of the object `vo_id` and of its version `sys_version`, read by
/// the head's and the version's primary keys in one statement; `None` when
/// the store holds no such object (#5778).
///
/// # Errors
/// [`StorageError`] on a driver failure or a column that does not decode.
pub async fn read_marks(
    pool: &PgPool,
    vo_id: VoId,
    sys_version: i32,
) -> Result<Option<Marks>, StorageError> {
    let door = crate::party_binding::request::door();
    let query = if crate::party_binding::request::reads_door() {
        sqlx::query(MARKS_RIDE_SQL)
            .bind(vo_id)
            .bind(sys_version)
            .bind(door.as_deref().map(crate::db::snapshot::Snapshot::text))
    } else {
        sqlx::query(MARKS_SQL).bind(vo_id).bind(sys_version)
    };
    let Some(row) = query.fetch_optional(&mut *acquire(pool).await?).await? else {
        return Ok(None);
    };
    Ok(Some(Marks {
        head: row.try_get("held_head")?,
        restricted_at: row
            .try_get::<Option<jiff_sqlx::Timestamp>, _>("restricted_at")?
            .map(jiff_sqlx::Timestamp::to_jiff),
        person_withheld: row.try_get("person_withheld")?,
        digest: row.try_get("held_digest")?,
        revision: crate::storage::access_projection::Revision::read(&row)?,
    }))
}
