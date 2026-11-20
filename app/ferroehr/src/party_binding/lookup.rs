// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the binding side table the lookup makes on every request.
//!
//! They are the key rows of one login, the live memberships of the parties
//! found and the live representation relationships of the representatives
//! among them. A key row binds only while its issuer's key space is
//! `active` ([`crate::party_binding::key_space`]), read in the same
//! statement; an issuer no key space records binds nothing. No openEHR spec governs the side table or its SQL: our own
//! design.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::party_binding::grants::caller::care_of::ProviderKey;
use crate::party_binding::key_space::{KeySpaceClosed, KeySpaceState};
use crate::storage::error::StorageError;

/// Why the key rows of a login cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    /// The issuer's key space is not `active`, so its keys bind no login.
    #[error(transparent)]
    Closed(#[from] KeySpaceClosed),
    /// The party store failed.
    #[error("the binding side table could not be read")]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for LookupError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(e))
    }
}

impl LookupError {
    /// The closed key space, when that is why.
    #[must_use]
    pub fn closed(&self) -> Option<&KeySpaceClosed> {
        match self {
            Self::Closed(closed) => Some(closed),
            Self::Storage(_) => None,
        }
    }
}

/// The refusal of `issuer` unless `space`, the stored state of its key space
/// (`None` when no key space records it), is `active`.
fn open(issuer: &str, space: Option<&str>) -> Result<(), LookupError> {
    match space.map(|name| KeySpaceState::parse(name).ok_or(name)) {
        Some(Ok(KeySpaceState::Active)) => Ok(()),
        None => Err(KeySpaceClosed {
            issuer: issuer.to_owned(),
            state: None,
        }
        .into()),
        Some(Ok(state)) => Err(KeySpaceClosed {
            issuer: issuer.to_owned(),
            state: Some(state),
        }
        .into()),
        Some(Err(name)) => Err(StorageError::InvalidRows(format!(
            "the key space state {name:?} is not one of four"
        ))
        .into()),
    }
}

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
/// `issuer`, live and tombstoned, with the store's binding revision.
///
/// The revision is the one the statement read the rows at, so the rows a
/// later statement reads are never older than it.
///
/// Each carries the professional state on `today` (`YYYY-MM-DD`); answered
/// from `ix_binding_identifier_lookup` and the key space's primary key.
///
/// # Errors
/// [`LookupError::Closed`] when `issuer`'s key space is not `active` or no
/// key space records it, [`LookupError::Storage`] on a driver failure.
pub async fn key_rows(
    conn: &mut PgConnection,
    issuer: &str,
    identifier_type: &str,
    value: &str,
    today: &str,
) -> Result<(Vec<KeyRow>, i64), LookupError> {
    let rows = sqlx::query(LOOKUP)
        .bind(issuer)
        .bind(identifier_type)
        .bind(value)
        .bind(today)
        .fetch_all(&mut *conn)
        .await?;
    let (space, revision): (Option<String>, i64) = match rows.first() {
        Some(row) => (row.try_get("space")?, row.try_get("revision")?),
        None => {
            return Err(StorageError::InvalidRows(
                "the binding lookup answered no row, where it answers the key space's".to_owned(),
            )
            .into());
        }
    };
    open(issuer, space.as_deref())?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(party_id) = row.try_get::<Option<Uuid>, _>("party_id")? else {
            continue;
        };
        out.push(KeyRow {
            party_id,
            lifecycle_state: row.try_get("lifecycle_state")?,
            professional: row.try_get("professional")?,
            representative: row.try_get("representative")?,
            live: row.try_get("live")?,
        });
    }
    Ok((out, revision))
}

/// The lookup statement of [`key_rows`], public so a test can `EXPLAIN` it.
///
/// It answers one row with the key space's state, NULL when none records the
/// issuer, and no key row when no row matches or the key space is not
/// `active`; every row carries the binding revision.
pub const LOOKUP: &str = "SELECT s.state AS space, b.party_id, b.lifecycle_state, \
            b.professional @> $4::date AS professional, b.representative, \
            b.retired_at IS NULL AS live, \
            (SELECT r.binding_revision FROM binding_grant_revision r) AS revision \
     FROM (SELECT $1::text AS issuer) q \
     LEFT JOIN binding_key_space s ON s.issuer = q.issuer \
     LEFT JOIN binding_identifier b ON s.state = 'active' \
       AND b.selector = 'key' AND b.issuer = q.issuer AND b.identifier_type = $2 \
       AND b.value = $3 AND (b.retired_at IS NULL OR b.deprovisioned_at IS NOT NULL) \
     ORDER BY b.party_id";

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

/// One live representation: the representative, the subject and the
/// openEHR `subject_relationship` code it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationRow {
    /// The representative `PERSON`'s container.
    pub representative_id: Uuid,
    /// The subject `PERSON`'s container.
    pub subject_id: Uuid,
    /// The openEHR `subject_relationship` code, when the relationship names
    /// one.
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
         ORDER BY representative_id, subject_id, relationship_id",
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

/// The live key rows of the `PERSON`s whose portal login is `value`.
///
/// The key is under the identifier issuer `issuer`, a `patients` issuer, of
/// any type, compared in the canonical form its type declares; each row is
/// `(party, lifecycle_state)`.
///
/// # Errors
/// [`LookupError::Closed`] when `issuer`'s key space is not `active` or no
/// key space records it, [`LookupError::Storage`] on a driver failure.
pub async fn representative_keys(
    conn: &mut PgConnection,
    issuer: &str,
    value: &str,
) -> Result<Vec<(Uuid, String)>, LookupError> {
    let rows = sqlx::query(
        "SELECT s.state AS space, b.party_id, b.lifecycle_state \
         FROM (SELECT $1::text AS issuer) q \
         LEFT JOIN binding_key_space s ON s.issuer = q.issuer \
         LEFT JOIN binding_identifier b ON s.state = 'active' \
           AND b.selector = 'key' AND b.issuer = q.issuer AND b.retired_at IS NULL \
           AND b.value = CASE WHEN lower(b.identifier_type) IN ('guid', 'uuid') \
                              THEN lower($2) ELSE $2 END \
         ORDER BY b.party_id",
    )
    .bind(issuer)
    .bind(value)
    .fetch_all(&mut *conn)
    .await?;
    let space: Option<String> = match rows.first() {
        Some(row) => row.try_get("space")?,
        None => None,
    };
    open(issuer, space.as_deref())?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(party) = row.try_get::<Option<Uuid>, _>("party_id")? {
            out.push((party, row.try_get("lifecycle_state")?));
        }
    }
    Ok(out)
}

/// The representation a representative is admitted on.
///
/// It is live on the day, recorded for a provider the caller acts for, and
/// not written by the representative itself (draft access control decision
/// procedure, step 0 Representation, and Writing Relationships).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRepresentation {
    /// The `PARTY_RELATIONSHIP` container of the representation.
    pub container: Uuid,
    /// The openEHR `subject_relationship` code it names, when it names one.
    pub relationship: Option<String>,
}

/// Whether `representative` has ever represented `subject`.
///
/// `None` when no representation row, live or retired, names the pair; else
/// the representation the representative is admitted on, on `today`
/// (`YYYY-MM-DD`) and for one of `providers`, `Some(None)` when none is.
/// Answered from `ix_binding_representation_history`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn representation_of(
    conn: &mut PgConnection,
    (representative, subject): (Uuid, Uuid),
    today: &str,
    providers: &[ProviderKey],
) -> Result<Option<Option<LiveRepresentation>>, StorageError> {
    let issuers: Vec<&str> = providers.iter().map(|p| p.issuer.as_str()).collect();
    let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
    let row: Option<(bool, Uuid, Option<String>)> = sqlx::query_as(
        "SELECT (r.retired_at IS NULL AND r.validity @> $3::date AND p.id IS NOT NULL \
                 AND r.writer_id IS DISTINCT FROM r.representative_id) AS admits, \
                r.relationship_id, r.relationship \
         FROM binding_representation r \
         LEFT JOIN unnest($4::text[], $5::text[]) AS p(issuer, id) \
           ON p.issuer = r.provider_issuer AND p.id = r.provider_id \
         WHERE r.representative_id = $1 AND r.subject_id = $2 \
         ORDER BY admits DESC, r.relationship_id LIMIT 1",
    )
    .bind(representative)
    .bind(subject)
    .bind(today)
    .bind(&issuers)
    .bind(&ids)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(admits, container, relationship)| {
        admits.then_some(LiveRepresentation {
            container,
            relationship,
        })
    }))
}
