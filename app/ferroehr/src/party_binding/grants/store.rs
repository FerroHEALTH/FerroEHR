// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the grant rows: `binding_client`, `binding_capability`,
//! `binding_provider` and the revision that moves with them, beside the
//! caller facts of [`super::caller::store`].
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
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::grants::live::revisions::{FactsRead, HeldSnapshot, Revisions};
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::step::{Step, Tag};
use crate::storage::error::StorageError;
use crate::versioning::Kind;

/// The channel a grant change notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_policy_grants";

/// The kinds whose heads hold grant rows or caller facts: every `ACTOR`, the
/// `ROLE` and the `PARTY_RELATIONSHIP` of its own a care relationship is.
pub(crate) const HOLDERS: [Kind; 6] = [
    Kind::Agent,
    Kind::Person,
    Kind::Organisation,
    Kind::Group,
    Kind::Role,
    Kind::PartyRelationship,
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

impl<'a> GrantHead<'a> {
    /// What a live head holds; `None` for a deleted one.
    pub(crate) fn live(&self) -> Option<&'a Facts> {
        (self.lifecycle_state != DELETED)
            .then_some(self.facts)
            .flatten()
    }
}

/// What a head of `kind` holds of the caller facts: the party they are about
/// unless a fact names its own (the performer of a `ROLE`, the source of a
/// care relationship, else the party itself) and the `ROLE`'s
/// `time_validity`; `None` for a kind whose heads hold none.
fn facts_about(kind: Kind, head: GrantHead<'_>) -> Option<(Option<Uuid>, Option<&Value>)> {
    match kind {
        Kind::Organisation | Kind::Person | Kind::Group => Some((Some(head.vo_id), None)),
        Kind::PartyRelationship => Some((head.live().and_then(|f| f.grants.care_source), None)),
        Kind::Role => Some((
            head.live()
                .and_then(|facts| facts.performer.as_deref())
                .and_then(container),
            head.live().and_then(|facts| facts.validity.as_ref()),
        )),
        _ => None,
    }
}

/// Replaces the grant rows and the caller facts of `head`, a party of `kind`,
/// on its commit transaction `tx`; moves the revision and notifies when they
/// changed. An `ACTOR` holds client rows, an `ORGANISATION` its provider row
/// and its `GROUP` memberships too, a `PERSON` its memberships, a `GROUP`
/// its membership of the `ORGANISATION` it belongs to, a `ROLE`
/// capability rows and its clearances and purposes, and a care relationship
/// of its own the fact about its source; every other kind holds none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, the unique violation of
/// `uq_binding_client` included.
pub(crate) async fn write(
    tx: &mut PgConnection,
    kind: Kind,
    head: GrantHead<'_>,
) -> Result<(), StorageError> {
    match facts_about(kind, head) {
        Some(_) if kind == Kind::Person => super::caller::store::write_person(tx, head).await?,
        Some((party, validity)) => super::caller::store::write(tx, head, party, validity).await?,
        None => {}
    }
    let changed = match kind {
        Kind::Organisation => write_clients(tx, head).await? | write_provider(tx, head).await?,
        Kind::Person | Kind::Group | Kind::Agent => write_clients(tx, head).await?,
        Kind::Role => write_role(tx, head).await?,
        _ => false,
    };
    if changed {
        revise(tx).await?;
        super::live::note_change();
    }
    Ok(())
}

/// The steps of [`write()`] for `head`, a party of `kind`, as a settle folds
/// them: the deletes, then the inserts. A row they return tagged
/// [`Tag::Grants`] says a grant row changed; the caller facts' and the record
/// subjects' rows say whether those changed.
pub(crate) fn steps(kind: Kind, head: GrantHead<'_>) -> (Vec<Step<'_>>, Vec<Step<'_>>) {
    let mut clear = Vec::new();
    let mut fill = Vec::new();
    if let Some((party, validity)) = facts_about(kind, head) {
        let (gone, came) = super::caller::store::steps(head, party, validity);
        clear.push(gone);
        fill.push(came);
        if kind == Kind::Person {
            let (gone, came) = super::caller::subject::steps(head);
            clear.push(gone);
            fill.push(came);
            let (gone, came) = super::caller::patient::steps(head);
            clear.push(gone);
            fill.push(came);
        }
    }
    match kind {
        Kind::Organisation => {
            clear.extend([clients_gone(head), provider_gone(head)]);
            fill.extend(clients_came(head).into_iter().chain(provider_came(head)));
        }
        Kind::Person | Kind::Group | Kind::Agent => {
            clear.push(clients_gone(head));
            fill.extend(clients_came(head));
        }
        Kind::Role => {
            clear.push(capabilities_gone(head));
            fill.extend(capabilities_came(head));
        }
        _ => {}
    }
    (clear, fill)
}

/// Replaces the client rows of the `ACTOR` `head`; whether any row went or
/// came.
async fn write_clients(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let gone = clients_gone(head).run(tx).await?;
    let Some(came) = clients_came(head) else {
        return Ok(!gone.is_empty());
    };
    let came = came.run(tx).await?;
    Ok(!gone.is_empty() || !came.is_empty())
}

/// The step that deletes the client rows of `head`.
fn clients_gone<'q>(head: GrantHead<'_>) -> Step<'q> {
    let party = head.vo_id;
    Step::new(
        "cd",
        "DELETE FROM binding_client WHERE party_id = $1 RETURNING party_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| query.bind(party),
    )
}

/// The step that inserts the client rows the live `head` holds; `None` when
/// it holds none.
fn clients_came(head: GrantHead<'_>) -> Option<Step<'_>> {
    let facts = head
        .live()
        .filter(|facts| !facts.grants.clients.is_empty())?;
    let (issuers, clients): (Vec<&str>, Vec<&str>) = facts
        .grants
        .clients
        .iter()
        .map(|(issuer, client)| (issuer.as_str(), client.as_str()))
        .unzip();
    let provider = facts.grants.provider;
    Some(Step::new(
        "ci",
        "INSERT INTO binding_client (party_id, issuer, client_id, lifecycle_state, provider_id) \
         SELECT $1, t.issuer, t.client_id, $4, $5 \
         FROM unnest($2::text[], $3::text[]) AS t(issuer, client_id) \
         RETURNING party_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| {
            query
                .bind(head.vo_id)
                .bind(issuers)
                .bind(clients)
                .bind(head.lifecycle_state)
                .bind(provider)
        },
    ))
}

/// Replaces the provider row of the `ORGANISATION` `head`; whether the row
/// changed.
async fn write_provider(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let step = provider_came(head).unwrap_or_else(|| provider_gone(head));
    Ok(!step.run(tx).await?.is_empty())
}

/// The provider identifier the live `head` carries, `(issuer, id)`.
fn provider_of(head: GrantHead<'_>) -> Option<(&str, &str)> {
    head.live()
        .and_then(|facts| facts.grants.identifier.as_ref())
        .map(|(issuer, id)| (issuer.as_str(), id.as_str()))
}

/// The step that deletes the provider row of `head` when it carries no
/// identifier.
fn provider_gone(head: GrantHead<'_>) -> Step<'_> {
    let issuer = provider_of(head).map(|(issuer, _)| issuer);
    Step::new(
        "vd",
        "DELETE FROM binding_provider WHERE party_id = $1 AND $2::text IS NULL \
         RETURNING party_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| query.bind(head.vo_id).bind(issuer),
    )
}

/// The step that writes the provider row of `head`, leaving a row that holds
/// its identifier as it is; `None` when it carries none.
fn provider_came(head: GrantHead<'_>) -> Option<Step<'_>> {
    let (issuer, id) = provider_of(head)?;
    Some(Step::new(
        "vi",
        "INSERT INTO binding_provider (party_id, issuer, id) \
         SELECT $1, $2, $3 \
         ON CONFLICT (party_id) DO UPDATE SET issuer = EXCLUDED.issuer, id = EXCLUDED.id, \
             written_at = now() \
         WHERE (binding_provider.issuer, binding_provider.id) \
               IS DISTINCT FROM (EXCLUDED.issuer, EXCLUDED.id) \
         RETURNING party_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| query.bind(head.vo_id).bind(issuer).bind(id),
    ))
}

/// Replaces the capability rows of the `ROLE` `head`; whether any row went or
/// came.
async fn write_role(tx: &mut PgConnection, head: GrantHead<'_>) -> Result<bool, StorageError> {
    let gone = capabilities_gone(head).run(tx).await?;
    let Some(came) = capabilities_came(head) else {
        return Ok(!gone.is_empty());
    };
    let came = came.run(tx).await?;
    Ok(!gone.is_empty() || !came.is_empty())
}

/// What the live `ROLE` `head` grants and the container its `performer`
/// names, when it grants any capability to one.
fn capability_holder(head: GrantHead<'_>) -> Option<(&Facts, Uuid)> {
    let facts = head
        .live()
        .filter(|facts| !facts.grants.capabilities.is_empty())?;
    Some((facts, facts.performer.as_deref().and_then(container)?))
}

/// The step that deletes the capability rows of `head`.
fn capabilities_gone<'q>(head: GrantHead<'_>) -> Step<'q> {
    let role = head.vo_id;
    Step::new(
        "kd",
        "DELETE FROM binding_capability WHERE role_id = $1 \
         RETURNING role_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| query.bind(role),
    )
}

/// The step that inserts the capability rows the live `head` grants its
/// performer; `None` when it grants none to one.
fn capabilities_came(head: GrantHead<'_>) -> Option<Step<'_>> {
    let (facts, performer) = capability_holder(head)?;
    let columns = CapabilityColumns::of(&facts.grants);
    let validity = facts.validity.as_ref();
    Some(Step::new(
        "ki",
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
                   credentials) \
         RETURNING role_id AS p, NULL::text AS t",
        Some(Tag::Grants),
        move |query| {
            query
                .bind(head.vo_id)
                .bind(performer)
                .bind(head.lifecycle_state)
                .bind(validity)
                .bind(columns.ordinal)
                .bind(columns.operation)
                .bind(columns.validity)
                .bind(columns.unreadable)
                .bind(columns.not_before)
                .bind(columns.not_after)
                .bind(columns.credentials)
        },
    ))
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
pub(crate) async fn revise(tx: &mut PgConnection) -> Result<(), StorageError> {
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
        "DELETE FROM binding_caller_fact",
        "DELETE FROM binding_record_subject",
        "DELETE FROM binding_patient_identifier",
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

/// The grant revision and the binding revision, and whether the store holds
/// a caller facts stamp the snapshot of `held` did not see
/// (`party.facts_behind`), with the read's own snapshot when it does, on
/// `conn`, in one read.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and
/// [`StorageError::InvalidRows`] for a snapshot that does not read.
pub(crate) async fn revisions(
    conn: &mut PgConnection,
    held: Option<&HeldSnapshot>,
) -> Result<Revisions, StorageError> {
    let (grants, bindings, store): (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT r.revision, r.binding_revision, \
                CASE WHEN b.behind THEN pg_current_snapshot()::text END \
         FROM binding_grant_revision r, \
              LATERAL (SELECT party.facts_behind($1::pg_snapshot) AS behind) b",
    )
    .bind(held.map(|held| held.snapshot.text()))
    .fetch_one(&mut *conn)
    .await?;
    let facts = match (held, store) {
        (Some(held), None) => FactsRead::Seen {
            generation: held.generation,
        },
        (_, store) => FactsRead::Behind {
            store: store
                .as_deref()
                .map(crate::db::snapshot::read)
                .transpose()?
                .map(std::sync::Arc::new),
        },
    };
    Ok(Revisions {
        grants,
        facts,
        bindings,
    })
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
    let facts = super::caller::store::erase(tx, vo_ids).await?;
    if clients > 0 || capabilities > 0 || providers > 0 || facts {
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

/// One stored capability.
#[derive(Debug, Clone)]
pub(crate) struct CapabilityRow {
    pub(crate) role_id: Uuid,
    pub(crate) performer_id: Uuid,
    pub(crate) operation: String,
    /// When it holds.
    pub(crate) term: Term,
    /// Shared with every [`super::capabilities::Grant`] built from it, so a
    /// request takes a reference count, never a copy (#5754).
    pub(crate) credentials: Option<std::sync::Arc<Value>>,
    /// What the credentials give a Basic user ([`super::capabilities::Grant::held`]),
    /// read from them once when the row is read, so no request walks them.
    pub(crate) held: std::sync::Arc<[String]>,
}

impl CapabilityRow {
    /// The capability `role_id` grants `performer_id`: `operation` while
    /// `term` holds, under `credentials`.
    pub(crate) fn new(
        role_id: Uuid,
        performer_id: Uuid,
        operation: String,
        term: Term,
        credentials: Option<std::sync::Arc<Value>>,
    ) -> Self {
        let held = Operation::of(&operation).map_or_else(std::sync::Arc::default, |operation| {
            super::capabilities::held(operation, credentials.as_deref())
        });
        Self {
            role_id,
            performer_id,
            operation,
            term,
            credentials,
            held,
        }
    }
}

impl crate::memory::Weigh for CapabilityRow {
    fn heap(&self) -> usize {
        self.operation
            .heap()
            .saturating_add(self.term.lifecycle_state.heap())
            .saturating_add(self.credentials.heap())
            .saturating_add(self.held.heap())
    }
}

/// When a stored grant or caller fact holds: the lifecycle state of the head
/// that holds it, its dates as `[lower, upper)` bounds and its instants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Term {
    pub(crate) lifecycle_state: String,
    /// The `ROLE`'s dates, every date for a care relationship; `None` when
    /// they are empty.
    pub(crate) role_dates: Option<Dates>,
    /// The capability's or the relationship's dates; `None` when they are
    /// empty.
    pub(crate) dates: Option<Dates>,
    pub(crate) not_before: Option<jiff::Timestamp>,
    pub(crate) not_after: Option<jiff::Timestamp>,
}

impl Term {
    /// Whether it holds on `today` and at `now`: its head is `complete` or
    /// `incomplete`, both its date ranges hold `today`, and `now` is within
    /// its instants.
    pub(crate) fn holds(&self, today: jiff::civil::Date, now: jiff::Timestamp) -> bool {
        let live = matches!(self.lifecycle_state.as_str(), "532" | "553");
        let dated = self.role_dates.is_some_and(|dates| dates.holds(today))
            && self.dates.is_some_and(|dates| dates.holds(today));
        let timed = self.not_before.is_none_or(|before| before <= now)
            && self.not_after.is_none_or(|after| now <= after);
        live && dated && timed
    }

    /// The term of the stored row `row`, read from its `lifecycle_state`,
    /// `role_empty`, `role_lower`, `role_upper`, `empty`, `lower`, `upper`,
    /// `not_before` and `not_after` columns.
    pub(crate) fn of(row: &PgRow) -> Result<Self, StorageError> {
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
        Ok(Self {
            lifecycle_state: row.try_get("lifecycle_state")?,
            role_dates: dates("role_empty", "role_lower", "role_upper")?,
            dates: dates("empty", "lower", "upper")?,
            not_before: instant("not_before")?,
            not_after: instant("not_after")?,
        })
    }
}

/// The columns [`Term::of`] reads, as a select list over a table with the
/// `lifecycle_state`, `role_validity`, `validity`, `not_before` and
/// `not_after` columns of the grant rows.
pub(crate) const TERM_COLUMNS: &str = "lifecycle_state, \
    isempty(role_validity) AS role_empty, lower(role_validity) AS role_lower, \
    upper(role_validity) AS role_upper, isempty(validity) AS empty, lower(validity) AS lower, \
    upper(validity) AS upper, not_before, not_after";

/// A non-empty date range `[lower, upper)`, each bound `None` when unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
/// client identifier with its provider, every capability, and every caller
/// fact.
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub(crate) revision: i64,
    pub(crate) clients: Vec<ClientRow>,
    pub(crate) capabilities: Vec<CapabilityRow>,
    pub(crate) facts: super::caller::store::Changed,
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
    let mut conn = acquire(pool).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT c.issuer, c.client_id FROM binding_client c \
         JOIN binding_capability k ON k.performer_id = c.party_id \
         WHERE k.operation = $1 ORDER BY c.issuer, c.client_id",
    )
    .bind(Operation::ScimProvision.code())
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// The federation issuers the partner holds its client identifiers under,
/// for the agreement the `ROLE` container `role` grants, sorted; empty when
/// `role` grants no federation agreement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn agreement_issuers(
    pool: &sqlx::PgPool,
    role: Uuid,
) -> Result<Vec<String>, StorageError> {
    use crate::party_binding::grants::agreement::Accessor;
    let mut conn = acquire(pool).await?;
    let operations = [Accessor::Person, Accessor::Organisation].map(|a| a.operation().code());
    let issuers: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT c.issuer FROM binding_capability k \
         JOIN binding_client c ON c.party_id = k.performer_id \
         WHERE k.role_id = $1 AND k.operation = ANY($2) ORDER BY c.issuer",
    )
    .bind(role)
    .bind(&operations[..])
    .fetch_all(&mut *conn)
    .await?;
    Ok(issuers)
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
    let statement = format!(
        "SELECT role_id, performer_id, operation, {TERM_COLUMNS}, credentials \
         FROM binding_capability ORDER BY performer_id, role_id, ordinal"
    );
    for row in sqlx::query(sqlx::AssertSqlSafe(statement))
        .fetch_all(&mut *tx)
        .await?
    {
        capabilities.push(CapabilityRow::new(
            row.try_get("role_id")?,
            row.try_get("performer_id")?,
            row.try_get("operation")?,
            Term::of(&row)?,
            row.try_get::<Option<Value>, _>("credentials")?
                .map(std::sync::Arc::new),
        ));
    }
    let facts = super::caller::store::load(&mut tx, None).await?;
    // NOTE: our own design (#5762), a read-only snapshot that writes nothing.
    tx.commit().await?;
    Ok(Loaded {
        revision,
        clients,
        capabilities,
        facts,
    })
}
