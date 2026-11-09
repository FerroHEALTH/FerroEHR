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
//!
//! The Groups of a client are the `GROUP`s it created (`scim_group`) whose
//! `displayName` row is live, so a deleted `GROUP` is no Group. A Group
//! filter compares a `displayName` by its fold, since Microsoft Entra ID
//! needs it unique, and an `externalId` under the client's group issuer
//! exactly.

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

/// The ids among `ids` that are Users of the client of `binding_issuer` on
/// `today` (`YYYY-MM-DD`): each a `PERSON` with a live key row of that issuer
/// and a professional `ROLE` valid on that date.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn users_among(
    conn: &mut PgConnection,
    binding_issuer: &str,
    ids: &[Uuid],
    today: &str,
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT k.party_id FROM binding_identifier k \
         WHERE k.selector = 'key' AND k.issuer = $1 AND k.retired_at IS NULL \
           AND k.professional @> $3::date AND k.party_id = ANY($2::uuid[]) \
         ORDER BY k.party_id",
    )
    .bind(binding_issuer)
    .bind(ids)
    .bind(today)
    .fetch_all(&mut *conn)
    .await?)
}

/// The condition that the `GROUP` `o.group_id`, whose live `displayName` row
/// is `d`, is a Group of the client of `$1` matching the ids `$3`, the
/// `displayName`s `$4` and the `externalId`s `$5` under the group issuer `$2`.
macro_rules! group_of_client {
    () => {
        "o.binding_issuer = $1 AND o.group_id = ALL($3::uuid[]) \
         AND NOT EXISTS (SELECT 1 FROM unnest($4::text[]) AS n(v) \
                         WHERE d.folded <> party.binding_fold(n.v)) \
         AND NOT EXISTS (SELECT 1 FROM unnest($5::text[]) AS n(v) WHERE NOT EXISTS ( \
             SELECT 1 FROM binding_identifier e WHERE e.party_id = o.group_id \
               AND e.selector = 'group_external_id' AND e.issuer = $2 \
               AND e.retired_at IS NULL AND e.value = n.v))"
    };
}

/// One Group of a client, as the side table holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    /// The `GROUP`'s container: the Group's `id`.
    pub party: VoId,
    /// The `displayName`.
    pub display_name: String,
    /// The head's `version_lifecycle_state` code.
    pub lifecycle_state: String,
    /// The `externalId` under the client's group issuer.
    pub external_id: Option<String>,
}

/// The equality tests a Group filter makes, each a list a Group must match
/// in full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupFilter {
    /// The `id`s the Group's container equals.
    pub ids: Vec<Uuid>,
    /// The `displayName`s the Group's `displayName` equals, by its fold.
    pub display_names: Vec<String>,
    /// The `externalId`s the Group's `externalId` equals.
    pub external_ids: Vec<String>,
}

/// Who a Group query asks for: the client by its `binding_issuer`, and the
/// issuer its Groups' `externalId`s are under.
#[derive(Debug, Clone, Copy)]
pub struct GroupClient<'a> {
    /// The client's `binding_issuer`.
    pub binding_issuer: &'a str,
    /// The `group_identifier_issuer` its Groups' `externalId`s are under.
    pub group_issuer: &'a str,
}

/// The live Groups the client `client` created matching `filter`: ordered
/// by container, from the 0-based `offset`, at most `limit`; with the total
/// match count. A Group is live while its `displayName` row is.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn groups(
    conn: &mut PgConnection,
    client: GroupClient<'_>,
    filter: &GroupFilter,
    page: (i64, i64),
) -> Result<(i64, Vec<GroupRow>), StorageError> {
    let rows = sqlx::query(concat!(
        "WITH matched AS ( \
             SELECT o.group_id, d.value, d.lifecycle_state FROM scim_group o \
             JOIN binding_identifier d ON d.party_id = o.group_id \
                  AND d.selector = 'group_display_name' AND d.retired_at IS NULL WHERE ",
        group_of_client!(),
        ") SELECT t.total, m.group_id, m.value, m.lifecycle_state, \
                  (SELECT e.value FROM binding_identifier e \
                   WHERE e.party_id = m.group_id AND e.selector = 'group_external_id' \
                     AND e.issuer = $2 AND e.retired_at IS NULL ORDER BY e.value LIMIT 1) \
                      AS external_id \
           FROM (SELECT count(*) AS total FROM matched) AS t \
           LEFT JOIN LATERAL (SELECT * FROM matched ORDER BY group_id OFFSET $6 LIMIT $7) AS m \
                ON true \
           ORDER BY m.group_id"
    ))
    .bind(client.binding_issuer)
    .bind(client.group_issuer)
    .bind(&filter.ids)
    .bind(&filter.display_names)
    .bind(&filter.external_ids)
    .bind(page.0)
    .bind(page.1)
    .fetch_all(&mut *conn)
    .await?;
    let mut total = 0;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        total = row.try_get("total")?;
        // NOTE: a page past the last match joins no Group, so its one row carries the total alone.
        let Some(party) = row.try_get::<Option<Uuid>, _>("group_id")? else {
            continue;
        };
        out.push(GroupRow {
            party: VoId(party),
            display_name: row.try_get("value")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            external_id: row.try_get("external_id")?,
        });
    }
    Ok((total, out))
}

/// Records on `tx` that the client of `binding_issuer` created the `GROUP`
/// `group`, before the commit that creates it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn record_group_owner(
    tx: &mut PgConnection,
    binding_issuer: &str,
    group: VoId,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO scim_group (group_id, binding_issuer) VALUES ($1, $2)")
        .bind(group)
        .bind(binding_issuer)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Whether the client of `binding_issuer` created the `GROUP` `group`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn owns_group(
    conn: &mut PgConnection,
    binding_issuer: &str,
    group: VoId,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM scim_group WHERE group_id = $1 AND binding_issuer = $2)",
    )
    .bind(group)
    .bind(binding_issuer)
    .fetch_one(&mut *conn)
    .await?)
}

/// The live members of the `GROUP` `group`: the `PERSON`s whose memberships
/// target it, in the order they joined.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn group_members(
    conn: &mut PgConnection,
    group: VoId,
) -> Result<Vec<VoId>, StorageError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT member_id FROM binding_membership \
         WHERE group_id = $1 AND retired_at IS NULL ORDER BY written_at, member_id",
    )
    .bind(group)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids.into_iter().map(VoId).collect())
}

/// The membership revision of the `GROUP` `group` and when it last moved;
/// `None` before its first membership change.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn group_revision(
    conn: &mut PgConnection,
    group: VoId,
) -> Result<Option<(i64, jiff::Timestamp)>, StorageError> {
    let row =
        sqlx::query("SELECT revision, revised_at FROM binding_group_revision WHERE group_id = $1")
            .bind(group)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let revised_at: jiff_sqlx::Timestamp = row.try_get("revised_at")?;
    Ok(Some((row.try_get("revision")?, revised_at.to_jiff())))
}

/// Counts one deactivation or deletion of the client of `binding_issuer` on
/// the write transaction `tx`.
///
/// The bound is `max` per window of `window_minutes`; `false` when the
/// window already holds `max`, and nothing is counted.
/// The client's row is read under `FOR UPDATE`, so two replicas counting at
/// once queue on it and share one bound.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn count_deactivation(
    tx: &mut PgConnection,
    binding_issuer: &str,
    max: u32,
    window_minutes: u32,
) -> Result<bool, StorageError> {
    sqlx::query(
        "INSERT INTO scim_deactivation (binding_issuer, window_start, counted) \
         VALUES ($1, now(), 0) ON CONFLICT (binding_issuer) DO NOTHING",
    )
    .bind(binding_issuer)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(
        "SELECT window_start + make_interval(mins => $2) <= now() AS expired, counted \
         FROM scim_deactivation WHERE binding_issuer = $1 FOR UPDATE",
    )
    .bind(binding_issuer)
    .bind(i32::try_from(window_minutes).unwrap_or(i32::MAX))
    .fetch_one(&mut *tx)
    .await?;
    let expired: bool = row.try_get("expired")?;
    let counted: i32 = if expired { 0 } else { row.try_get("counted")? };
    if i64::from(counted) >= i64::from(max) {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE scim_deactivation \
         SET window_start = CASE WHEN $2 THEN now() ELSE window_start END, counted = $3 \
         WHERE binding_issuer = $1",
    )
    .bind(binding_issuer)
    .bind(expired)
    .bind(counted.saturating_add(1))
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

/// At most `limit` deleted `PERSON`s of the clients of `binding_issuers`
/// whose contacts are due for erasure: a deprovisioned key row of one of
/// those issuers at least `delay_minutes` old, and no erasure recorded.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn contacts_due(
    conn: &mut PgConnection,
    binding_issuers: &[String],
    delay_minutes: u32,
    limit: i64,
) -> Result<Vec<VoId>, StorageError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT t.party_id FROM binding_identifier t \
         WHERE t.deprovisioned_at IS NOT NULL AND t.issuer = ANY($1) \
           AND t.deprovisioned_at + make_interval(mins => $2) <= now() \
           AND NOT EXISTS (SELECT 1 FROM scim_contact_erasure e WHERE e.party_id = t.party_id) \
         ORDER BY t.party_id LIMIT $3",
    )
    .bind(binding_issuers)
    .bind(i32::try_from(delay_minutes).unwrap_or(i32::MAX))
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids.into_iter().map(VoId).collect())
}

/// Locks the head row of `party` on the erasure transaction `tx`, so two
/// replicas erasing at once queue on it; `false` when the party has no head
/// or its contacts are already erased.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock_contact_erasure(
    tx: &mut PgConnection,
    party: VoId,
) -> Result<bool, StorageError> {
    let head: Option<Uuid> =
        sqlx::query_scalar("SELECT vo_id FROM vo_head WHERE vo_id = $1 FOR UPDATE")
            .bind(party)
            .fetch_optional(&mut *tx)
            .await?;
    if head.is_none() {
        return Ok(false);
    }
    let erased: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM scim_contact_erasure WHERE party_id = $1)",
    )
    .bind(party)
    .fetch_one(&mut *tx)
    .await?;
    Ok(!erased)
}

/// Records on `tx` that the contacts of `party` are erased.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn mark_contacts_erased(tx: &mut PgConnection, party: VoId) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO scim_contact_erasure (party_id, erased_at) VALUES ($1, now())")
        .bind(party)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// The provisional `PERSON`s whose first version was committed before
/// `before`, with that instant, oldest first: each `PERSON` with a live key
/// row whose head is `553|incomplete|`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn provisional_before(
    conn: &mut PgConnection,
    before: jiff::Timestamp,
) -> Result<Vec<(VoId, jiff::Timestamp)>, StorageError> {
    let rows = sqlx::query(
        "SELECT p.party_id, min(a.time_committed) AS created \
         FROM (SELECT DISTINCT k.party_id FROM binding_identifier k \
               WHERE k.selector = 'key' AND k.retired_at IS NULL \
                 AND k.lifecycle_state = '553') p \
         JOIN version v ON v.vo_id = p.party_id \
         JOIN commit_audit a ON a.id = v.commit_audit_id \
         GROUP BY p.party_id \
         HAVING min(a.time_committed) < $1 \
         ORDER BY created, p.party_id",
    )
    .bind(jiff_sqlx::Timestamp::from(before))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let created: jiff_sqlx::Timestamp = row.try_get("created")?;
        out.push((VoId(row.try_get("party_id")?), created.to_jiff()));
    }
    Ok(out)
}
