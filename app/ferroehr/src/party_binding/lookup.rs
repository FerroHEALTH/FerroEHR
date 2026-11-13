// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the binding side table the lookup makes on every request.
//!
//! They are the key rows of one login, the live memberships of the parties
//! found and the live representation relationships of the representatives
//! among them. No openEHR spec governs the side table or its SQL: our own
//! design.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::storage::error::StorageError;

/// One key row the lookup reads: a live row, or the tombstone of a deleted
/// `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    /// The `PERSON`'s container.
    pub party_id: Uuid,
    /// The head's `version_lifecycle_state` code.
    pub lifecycle_state: String,
    /// Whether a professional `ROLE` is valid on the lookup's date.
    pub professional: bool,
    /// Whether the `PERSON` is the source of a representation relationship.
    pub representative: bool,
    /// Whether the row is live; a row that is not is a tombstone.
    pub live: bool,
}

/// The key rows of the identifier `value` of `identifier_type` under
/// `issuer`, live and tombstoned, with the professional state on `today`
/// (`YYYY-MM-DD`); answered from `ix_binding_identifier_lookup` alone.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn key_rows(
    conn: &mut PgConnection,
    issuer: &str,
    identifier_type: &str,
    value: &str,
    today: &str,
) -> Result<Vec<KeyRow>, StorageError> {
    let rows = sqlx::query(LOOKUP)
        .bind(issuer)
        .bind(identifier_type)
        .bind(value)
        .bind(today)
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(KeyRow {
            party_id: row.try_get("party_id")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            professional: row.try_get("professional")?,
            representative: row.try_get("representative")?,
            live: row.try_get("live")?,
        });
    }
    Ok(out)
}

/// The lookup statement of [`key_rows`], public so a test can `EXPLAIN` it.
pub const LOOKUP: &str = "SELECT party_id, lifecycle_state, professional @> $4::date AS professional, \
            representative, retired_at IS NULL AS live \
     FROM binding_identifier \
     WHERE selector = 'key' AND issuer = $1 AND identifier_type = $2 AND value = $3 \
       AND (retired_at IS NULL OR deprovisioned_at IS NOT NULL) \
     ORDER BY party_id";

/// One live membership: a member, its group and the group's `externalId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipRow {
    /// The member `PERSON`'s container.
    pub member: Uuid,
    /// The `GROUP`'s container.
    pub group: Uuid,
    /// The `GROUP`'s `externalId` value under the queried issuer.
    pub external_id: String,
}

/// The live memberships of `members` in a `GROUP` whose head is `complete` or
/// `incomplete` and whose `externalId` is under `issuer`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn memberships(
    conn: &mut PgConnection,
    members: &[Uuid],
    issuer: &str,
) -> Result<Vec<MembershipRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT m.member_id, g.party_id AS group_id, g.value AS external_id \
         FROM binding_membership m \
         JOIN binding_identifier g ON g.party_id = m.group_id AND g.retired_at IS NULL \
                                  AND g.selector = 'group_external_id' \
         WHERE m.member_id = ANY($1) AND m.retired_at IS NULL AND g.issuer = $2 \
           AND g.lifecycle_state IN ('532', '553') \
         ORDER BY m.member_id, g.party_id, g.value",
    )
    .bind(members)
    .bind(issuer)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(MembershipRow {
            member: row.try_get("member_id")?,
            group: row.try_get("group_id")?,
            external_id: row.try_get("external_id")?,
        });
    }
    Ok(out)
}

/// One live representation relationship: the representative, the subject and
/// the mapped `subject_relationship` code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationRow {
    /// The representative `PERSON`'s container.
    pub representative_id: Uuid,
    /// The subject `PERSON`'s container.
    pub subject_id: Uuid,
    /// The openEHR `subject_relationship` code, when the type maps to one.
    pub relationship: Option<String>,
}

/// The representation relationships of `representatives` valid on `today`
/// (`YYYY-MM-DD`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn representations(
    conn: &mut PgConnection,
    representatives: &[Uuid],
    today: &str,
) -> Result<Vec<RepresentationRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT representative_id, subject_id, relationship FROM binding_representation \
         WHERE representative_id = ANY($1) AND retired_at IS NULL AND validity @> $2::date \
         ORDER BY representative_id, subject_id, relationship",
    )
    .bind(representatives)
    .bind(today)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(RepresentationRow {
            representative_id: row.try_get("representative_id")?,
            subject_id: row.try_get("subject_id")?,
            relationship: row.try_get("relationship")?,
        });
    }
    Ok(out)
}

/// The live key rows of the representatives whose binding key is `value`
/// under the identifier issuer `issuer`, of any type, compared in the
/// canonical form its type declares, as `(party, lifecycle_state)`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn representative_keys(
    conn: &mut PgConnection,
    issuer: &str,
    value: &str,
) -> Result<Vec<(Uuid, String)>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT party_id, lifecycle_state FROM binding_identifier \
         WHERE selector = 'key' AND issuer = $1 AND retired_at IS NULL AND representative \
           AND value = CASE WHEN lower(identifier_type) IN ('guid', 'uuid') \
                            THEN lower($2) ELSE $2 END \
         ORDER BY party_id",
    )
    .bind(issuer)
    .bind(value)
    .fetch_all(&mut *conn)
    .await?)
}

/// Whether `representative` has ever represented `subject`.
///
/// `None` when no representation row, live or retired, names the pair, else
/// whether one is live on `today` (`YYYY-MM-DD`), with the openEHR
/// `subject_relationship` code of the row that decides it; answered from
/// `ix_binding_representation_history`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn representation_of(
    conn: &mut PgConnection,
    representative: Uuid,
    subject: Uuid,
    today: &str,
) -> Result<Option<(bool, Option<String>)>, StorageError> {
    Ok(sqlx::query_as(
        "SELECT retired_at IS NULL AND validity @> $3::date AS live, relationship \
         FROM binding_representation \
         WHERE representative_id = $1 AND subject_id = $2 \
         ORDER BY live DESC, relationship LIMIT 1",
    )
    .bind(representative)
    .bind(subject)
    .bind(today)
    .fetch_optional(&mut *conn)
    .await?)
}
