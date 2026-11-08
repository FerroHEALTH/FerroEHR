// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the binding side table that find the Users of one SCIM
//! client.
//!
//! A User of the client whose `binding_issuer` is `$1` is a `PERSON` with a
//! live key row of that issuer whose professional `ROLE` is valid on the
//! request's date: each User belongs to the client whose `binding_issuer`
//! issues its key, every `PERSON` without a live professional `ROLE` is no
//! User, and a deleted `PERSON` has no live row. Its `userName` is its live
//! `user_name` row and its `externalId` its live `external_id` row of the
//! same issuer, so an `externalId` is unique within one client. A filter is a
//! conjunction of equality tests (RFC 7644 §3.4.2.2 `eq` and `and`): a
//! `userName` compared by its fold (RFC 7643 §4.1.1, `caseExact` false), an
//! `externalId` and an `id` exactly (RFC 7643 §3.1). No openEHR spec governs
//! the side table or its SQL: our own design.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The condition that the key row `k` is a live User of the client of `$1`
/// on `$5` matching the ids `$2`, the `userName`s `$3` and the `externalId`s
/// `$4`.
macro_rules! user_of_client {
    () => {
        "k.selector = 'key' AND k.issuer = $1 AND k.retired_at IS NULL \
         AND k.professional @> $5::date \
         AND k.party_id = ALL($2::uuid[]) \
         AND NOT EXISTS (SELECT 1 FROM unnest($3::text[]) AS n(v) WHERE NOT EXISTS ( \
             SELECT 1 FROM binding_identifier u WHERE u.party_id = k.party_id \
               AND u.selector = 'user_name' AND u.retired_at IS NULL \
               AND u.folded = party.binding_fold(n.v))) \
         AND NOT EXISTS (SELECT 1 FROM unnest($4::text[]) AS n(v) WHERE NOT EXISTS ( \
             SELECT 1 FROM binding_identifier e WHERE e.party_id = k.party_id \
               AND e.selector = 'external_id' AND e.issuer = $1 AND e.retired_at IS NULL \
               AND e.value = n.v))"
    };
}

/// One User of a client, as the side table holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRow {
    /// The `PERSON`'s container: the User's `id`.
    pub party: VoId,
    /// The binding key: the User's `loginIdentifier`.
    pub login_identifier: String,
    /// The head's `version_lifecycle_state` code.
    pub lifecycle_state: String,
    /// The `userName`.
    pub user_name: Option<String>,
    /// The `externalId`.
    pub external_id: Option<String>,
}

/// The equality tests a filter makes, each a list a User must match in
/// full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFilter {
    /// The `id`s the User's container equals.
    pub ids: Vec<Uuid>,
    /// The `userName`s the User's `userName` equals.
    pub user_names: Vec<String>,
    /// The `externalId`s the User's `externalId` equals.
    pub external_ids: Vec<String>,
}

/// The Users of the client of `binding_issuer` matching `filter` on `today`
/// (`YYYY-MM-DD`): ordered by container, from the 0-based `offset`, at most
/// `limit`; with the total match count.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn users(
    conn: &mut PgConnection,
    binding_issuer: &str,
    filter: &UserFilter,
    today: &str,
    page: (i64, i64),
) -> Result<(i64, Vec<UserRow>), StorageError> {
    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT count(DISTINCT k.party_id) FROM binding_identifier k WHERE ",
        user_of_client!()
    ))
    .bind(binding_issuer)
    .bind(&filter.ids)
    .bind(&filter.user_names)
    .bind(&filter.external_ids)
    .bind(today)
    .fetch_one(&mut *conn)
    .await?;
    let rows = sqlx::query(concat!(
        "SELECT DISTINCT ON (k.party_id) k.party_id, k.value, k.lifecycle_state, \
                (SELECT u.value FROM binding_identifier u \
                 WHERE u.party_id = k.party_id AND u.selector = 'user_name' \
                   AND u.retired_at IS NULL ORDER BY u.value LIMIT 1) AS user_name, \
                (SELECT e.value FROM binding_identifier e \
                 WHERE e.party_id = k.party_id AND e.selector = 'external_id' \
                   AND e.issuer = $1 AND e.retired_at IS NULL ORDER BY e.value LIMIT 1) \
                    AS external_id \
         FROM binding_identifier k WHERE ",
        user_of_client!(),
        " ORDER BY k.party_id, k.value OFFSET $6 LIMIT $7"
    ))
    .bind(binding_issuer)
    .bind(&filter.ids)
    .bind(&filter.user_names)
    .bind(&filter.external_ids)
    .bind(today)
    .bind(page.0)
    .bind(page.1)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(UserRow {
            party: VoId(row.try_get("party_id")?),
            login_identifier: row.try_get("value")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            user_name: row.try_get("user_name")?,
            external_id: row.try_get("external_id")?,
        });
    }
    Ok((total, out))
}
