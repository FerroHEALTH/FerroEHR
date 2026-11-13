// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the grant rows: `binding_client`, `binding_capability`,
//! `binding_provider` and the revision that moves with them.
//!
//! A commit of an `ACTOR` or a `ROLE` replaces the rows of that party with the
//! ones its new head holds, in its own transaction; a change moves the
//! revision and notifies [`CHANNEL`], which PostgreSQL delivers to every
//! listening replica when the transaction commits
//! (<https://www.postgresql.org/docs/18/sql-notify.html>). The refill
//! derives the rows of every head with the same function. No openEHR spec
//! governs the side table or its SQL: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the capability `time_validity` and \
              `credentials` fragments the grant rows hold"
)]

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::storage::error::StorageError;
use crate::versioning::Kind;

/// The channel a grant change notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_policy_grants";

/// The kinds whose heads hold grant rows: every `ACTOR` and the `ROLE`.
pub(crate) const HOLDERS: [Kind; 5] = [
    Kind::Agent,
    Kind::Person,
    Kind::Organisation,
    Kind::Group,
    Kind::Role,
];

/// The `version_lifecycle_state` code of a logically deleted head.
const DELETED: &str = "523";

/// The head of one party as the grant rows read it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GrantHead<'a> {
    /// The party's container.
    pub(crate) vo_id: Uuid,
    /// The head's `version_lifecycle_state` code.
    pub(crate) lifecycle_state: &'a str,
    /// What the head's body holds; `None` for a head without one.
    pub(crate) facts: Option<&'a Facts>,
}

impl GrantHead<'_> {
    /// What a live head holds; `None` for a deleted one.
    fn live(&self) -> Option<&Facts> {
        (self.lifecycle_state != DELETED)
            .then_some(self.facts)
            .flatten()
    }
}

/// Replaces the grant rows of `head`, a party of `kind`, on its commit
/// transaction `tx`; moves the revision and notifies when they changed.
/// An `ACTOR` holds client rows, an `ORGANISATION` its provider row too, and
/// a `ROLE` capability rows; every other kind holds none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, the unique violation of
/// `uq_binding_client` included.
pub(crate) async fn write(
    tx: &mut PgConnection,
    kind: Kind,
    head: GrantHead<'_>,
) -> Result<(), StorageError> {
    let changed = match kind {
        Kind::Organisation => write_clients(tx, head).await? | write_provider(tx, head).await?,
        Kind::Agent | Kind::Person | Kind::Group => write_clients(tx, head).await?,
        Kind::Role => write_role(tx, head).await?,
        _ => false,
    };
    if changed {
        revise(tx).await?;
        super::live::note_change();
    }
    Ok(())
}

/// Replaces the client rows of the `ACTOR` `head`; whether any row went or
/// came.
async fn write_clients(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let gone = sqlx::query("DELETE FROM binding_client WHERE party_id = $1")
        .bind(head.vo_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let Some(facts) = head.live().filter(|facts| !facts.grants.clients.is_empty()) else {
        return Ok(gone > 0);
    };
    let (issuers, clients): (Vec<&str>, Vec<&str>) = facts
        .grants
        .clients
        .iter()
        .map(|(issuer, client)| (issuer.as_str(), client.as_str()))
        .unzip();
    let came = sqlx::query(
        "INSERT INTO binding_client (party_id, issuer, client_id, lifecycle_state, provider_id) \
         SELECT $1, t.issuer, t.client_id, $4, $5 \
         FROM unnest($2::text[], $3::text[]) AS t(issuer, client_id)",
    )
    .bind(head.vo_id)
    .bind(&issuers)
    .bind(&clients)
    .bind(head.lifecycle_state)
    .bind(facts.grants.provider)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(gone > 0 || came > 0)
}

/// Replaces the provider row of the `ORGANISATION` `head`; whether the row
/// changed.
async fn write_provider(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let identifier = head
        .live()
        .and_then(|facts| facts.grants.identifier.as_ref());
    let Some((issuer, id)) = identifier else {
        let gone = sqlx::query("DELETE FROM binding_provider WHERE party_id = $1")
            .bind(head.vo_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        return Ok(gone > 0);
    };
    let written = sqlx::query(
        "INSERT INTO binding_provider (party_id, issuer, id) VALUES ($1, $2, $3) \
         ON CONFLICT (party_id) DO UPDATE SET issuer = EXCLUDED.issuer, id = EXCLUDED.id, \
             written_at = now() \
         WHERE (binding_provider.issuer, binding_provider.id) \
               IS DISTINCT FROM (EXCLUDED.issuer, EXCLUDED.id)",
    )
    .bind(head.vo_id)
    .bind(issuer)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(written > 0)
}

/// Replaces the capability rows of the `ROLE` `head`; whether any row went or
/// came.
async fn write_role(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let gone = sqlx::query("DELETE FROM binding_capability WHERE role_id = $1")
        .bind(head.vo_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let Some(facts) = head
        .live()
        .filter(|facts| !facts.grants.capabilities.is_empty())
    else {
        return Ok(gone > 0);
    };
    let Some(performer) = facts.performer.as_deref().and_then(container) else {
        return Ok(gone > 0);
    };
    let columns = CapabilityColumns::of(&facts.grants);
    let came = sqlx::query(
        "INSERT INTO binding_capability \
             (role_id, ordinal, operation, performer_id, lifecycle_state, role_validity, \
              validity, not_before, not_after, credentials) \
         SELECT $1, t.ordinal, t.operation, $2, $3, party.binding_validity($4::jsonb), \
                CASE WHEN t.unreadable THEN 'empty'::daterange \
                     ELSE party.binding_validity(t.validity) END, \
                t.not_before, t.not_after, t.credentials \
         FROM unnest($5::integer[], $6::text[], $7::jsonb[], $8::boolean[], \
                     $9::timestamptz[], $10::timestamptz[], $11::jsonb[]) \
              AS t(ordinal, operation, validity, unreadable, not_before, not_after, \
                   credentials)",
    )
    .bind(head.vo_id)
    .bind(performer)
    .bind(head.lifecycle_state)
    .bind(facts.validity.as_ref())
    .bind(&columns.ordinal)
    .bind(&columns.operation)
    .bind(&columns.validity)
    .bind(&columns.unreadable)
    .bind(&columns.not_before)
    .bind(&columns.not_after)
    .bind(&columns.credentials)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(gone > 0 || came > 0)
}

/// The capabilities of a `ROLE` as the arrays the insert binds.
struct CapabilityColumns<'a> {
    ordinal: Vec<i32>,
    operation: Vec<&'a str>,
    validity: Vec<Option<&'a Value>>,
    unreadable: Vec<bool>,
    not_before: Vec<Option<jiff_sqlx::Timestamp>>,
    not_after: Vec<Option<jiff_sqlx::Timestamp>>,
    credentials: Vec<Option<&'a Value>>,
}

impl<'a> CapabilityColumns<'a> {
    fn of(facts: &'a GrantFacts) -> Self {
        let instant = |bound: Bound| match bound {
            Bound::At(at) => Some(jiff_sqlx::Timestamp::from(at)),
            Bound::Open | Bound::Unreadable => None,
        };
        let capabilities = &facts.capabilities;
        Self {
            ordinal: capabilities.iter().map(|c| c.ordinal).collect(),
            operation: capabilities.iter().map(|c| c.code.as_str()).collect(),
            validity: capabilities.iter().map(|c| c.validity.as_ref()).collect(),
            unreadable: capabilities
                .iter()
                .map(|c| c.not_before == Bound::Unreadable || c.not_after == Bound::Unreadable)
                .collect(),
            not_before: capabilities.iter().map(|c| instant(c.not_before)).collect(),
            not_after: capabilities.iter().map(|c| instant(c.not_after)).collect(),
            credentials: capabilities
                .iter()
                .map(|c| c.credentials.as_ref())
                .collect(),
        }
    }
}

/// Moves the revision and notifies [`CHANNEL`] with it, on `tx`.
async fn revise(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "WITH revised AS ( \
             UPDATE binding_grant_revision SET revision = revision + 1, revised_at = now() \
             RETURNING revision) \
         SELECT pg_notify($1, revised.revision::text) FROM revised",
    )
    .bind(CHANNEL)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Deletes every grant row, on `tx`, before a refill.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn clear(tx: &mut PgConnection) -> Result<(), StorageError> {
    for statement in [
        "DELETE FROM binding_client",
        "DELETE FROM binding_capability",
        "DELETE FROM binding_provider",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    Ok(())
}

/// Moves the revision after a refill, on `tx`, so every replica reads the
/// rebuilt rows.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn refilled(tx: &mut PgConnection) -> Result<(), StorageError> {
    revise(tx).await?;
    super::live::note_change();
    Ok(())
}

/// The revision, on `tx`; `0` until the first fill or grant change.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn revision(conn: &mut PgConnection) -> Result<i64, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT revision FROM binding_grant_revision")
            .fetch_one(&mut *conn)
            .await?,
    )
}

/// Deletes every grant row naming any of `vo_ids`, on the erasure
/// transaction `tx`, and moves the revision when any went.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn erase(tx: &mut PgConnection, vo_ids: &[Uuid]) -> Result<(), StorageError> {
    let clients = sqlx::query("DELETE FROM binding_client WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let capabilities = sqlx::query(
        "DELETE FROM binding_capability WHERE role_id = ANY($1) OR performer_id = ANY($1)",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let providers = sqlx::query("DELETE FROM binding_provider WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if clients > 0 || capabilities > 0 || providers > 0 {
        revise(tx).await?;
        super::live::note_change();
    }
    Ok(())
}

/// One stored client identifier of an `ACTOR`.
#[derive(Debug, Clone)]
pub(crate) struct ClientRow {
    pub(crate) party_id: Uuid,
    pub(crate) issuer: String,
    pub(crate) client_id: String,
    pub(crate) lifecycle_state: String,
    /// The container the `ACTOR`'s acts-for relationship targets.
    pub(crate) provider_id: Option<Uuid>,
    /// That provider's identifier, `(issuer, id)`, when it carries one.
    pub(crate) provider: Option<(String, String)>,
}

/// One stored capability, with its dates as `[lower, upper)` bounds.
#[derive(Debug, Clone)]
pub(crate) struct CapabilityRow {
    pub(crate) role_id: Uuid,
    pub(crate) performer_id: Uuid,
    pub(crate) operation: String,
    pub(crate) lifecycle_state: String,
    /// The `ROLE`'s dates; `None` when they are empty.
    pub(crate) role_dates: Option<Dates>,
    /// The capability's dates; `None` when they are empty.
    pub(crate) dates: Option<Dates>,
    pub(crate) not_before: Option<jiff::Timestamp>,
    pub(crate) not_after: Option<jiff::Timestamp>,
    pub(crate) credentials: Option<Value>,
}

/// A non-empty date range `[lower, upper)`, each bound `None` when unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dates {
    pub(crate) lower: Option<jiff::civil::Date>,
    pub(crate) upper: Option<jiff::civil::Date>,
}

impl Dates {
    /// Whether the range holds `day`.
    pub(crate) fn holds(self, day: jiff::civil::Date) -> bool {
        self.lower.is_none_or(|lower| lower <= day) && self.upper.is_none_or(|upper| day < upper)
    }
}

/// Everything the grant rows hold, read in one snapshot: the revision, every
/// client identifier with its provider, and every capability.
#[derive(Debug, Clone, Default)]
pub(crate) struct Loaded {
    pub(crate) revision: i64,
    pub(crate) clients: Vec<ClientRow>,
    pub(crate) capabilities: Vec<CapabilityRow>,
}

/// The SCIM clients a binding key's provenance names, as token issuer and
/// `client_id`, sorted.
///
/// A SCIM client is a client whose `ACTOR` performs a `ROLE` that stores a
/// grant of `scim_provision`, in force or ended.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn provisioning_clients(
    pool: &sqlx::PgPool,
) -> Result<Vec<(String, String)>, StorageError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT c.issuer, c.client_id FROM binding_client c \
         JOIN binding_capability k ON k.performer_id = c.party_id \
         WHERE k.operation = $1 ORDER BY c.issuer, c.client_id",
    )
    .bind(crate::party_binding::grants::operation::Operation::ScimProvision.code())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Reads every grant row in one repeatable-read snapshot on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn load(conn: &mut PgConnection) -> Result<Loaded, StorageError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let revision = revision(&mut tx).await?;
    let mut clients = Vec::new();
    for row in sqlx::query(
        "SELECT c.party_id, c.issuer, c.client_id, c.lifecycle_state, c.provider_id, \
                p.issuer AS provider_issuer, p.id AS provider_ident \
         FROM binding_client c LEFT JOIN binding_provider p ON p.party_id = c.provider_id \
         ORDER BY c.issuer, c.client_id",
    )
    .fetch_all(&mut *tx)
    .await?
    {
        let provider_issuer: Option<String> = row.try_get("provider_issuer")?;
        let provider_ident: Option<String> = row.try_get("provider_ident")?;
        clients.push(ClientRow {
            party_id: row.try_get("party_id")?,
            issuer: row.try_get("issuer")?,
            client_id: row.try_get("client_id")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            provider_id: row.try_get("provider_id")?,
            provider: provider_issuer.zip(provider_ident),
        });
    }
    let mut capabilities = Vec::new();
    for row in sqlx::query(
        "SELECT role_id, performer_id, operation, lifecycle_state, \
                isempty(role_validity) AS role_empty, \
                lower(role_validity) AS role_lower, upper(role_validity) AS role_upper, \
                isempty(validity) AS empty, lower(validity) AS lower, upper(validity) AS upper, \
                not_before, not_after, credentials \
         FROM binding_capability ORDER BY performer_id, role_id, ordinal",
    )
    .fetch_all(&mut *tx)
    .await?
    {
        let dates = |empty: &str, lower: &str, upper: &str| -> Result<_, StorageError> {
            if row.try_get::<bool, _>(empty)? {
                return Ok(None);
            }
            let bound = |column: &str| -> Result<_, StorageError> {
                Ok(row
                    .try_get::<Option<jiff_sqlx::Date>, _>(column)?
                    .map(jiff_sqlx::Date::to_jiff))
            };
            Ok(Some(Dates {
                lower: bound(lower)?,
                upper: bound(upper)?,
            }))
        };
        let instant = |column: &str| -> Result<_, StorageError> {
            Ok(row
                .try_get::<Option<jiff_sqlx::Timestamp>, _>(column)?
                .map(jiff_sqlx::Timestamp::to_jiff))
        };
        capabilities.push(CapabilityRow {
            role_id: row.try_get("role_id")?,
            performer_id: row.try_get("performer_id")?,
            operation: row.try_get("operation")?,
            lifecycle_state: row.try_get("lifecycle_state")?,
            role_dates: dates("role_empty", "role_lower", "role_upper")?,
            dates: dates("empty", "lower", "upper")?,
            not_before: instant("not_before")?,
            not_after: instant("not_after")?,
            credentials: row.try_get("credentials")?,
        });
    }
    tx.commit().await?;
    Ok(Loaded {
        revision,
        clients,
        capabilities,
    })
}
