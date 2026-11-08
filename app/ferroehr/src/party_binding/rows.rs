// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The writes of the binding side table: each takes the rows the parties in
//! its scope should hold, as [`super::derive`] derived them, and writes only
//! the difference against the live rows (#5220).
//!
//! A live row the parties no longer hold is retired, a held row with no live
//! twin is inserted, and a row both have is left as it is, so a change writes
//! one row per value it changes. Every statement binds one array per column
//! and its text never varies with the row count. No openEHR spec governs the
//! side table or its SQL: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored `time_validity` \
              fragments the side table's date ranges are read from"
)]

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::storage::error::StorageError;

/// One identifier row a party should hold.
#[derive(Debug, Clone)]
pub(crate) struct IdentifierRow {
    /// The party's container.
    pub(crate) party: Uuid,
    /// `key`, `user_name`, `external_id`, `group_external_id` or
    /// `group_display_name`.
    pub(crate) selector: &'static str,
    /// The `DV_IDENTIFIER.issuer`; empty for a display name.
    pub(crate) issuer: String,
    /// The `DV_IDENTIFIER.type`; empty for a display name.
    pub(crate) identifier_type: String,
    /// The stored value: the canonical key, or the value as written.
    pub(crate) value: String,
    /// The head's `version_lifecycle_state` code.
    pub(crate) lifecycle_state: String,
    /// Whether the `PERSON` is the source of a representation relationship;
    /// read on a key row only.
    pub(crate) representative: bool,
}

/// The validity a professional `ROLE` gives the `PERSON` that performs it.
#[derive(Debug, Clone)]
pub(crate) struct Professional {
    /// The performing `PERSON`'s container.
    pub(crate) party: Uuid,
    /// The `ROLE`'s `time_validity`.
    pub(crate) validity: Option<Value>,
}

/// One representation relationship a representative holds.
#[derive(Debug, Clone)]
pub(crate) struct RepresentationRow {
    /// The representative's container.
    pub(crate) representative: Uuid,
    /// The subject's container.
    pub(crate) subject: Uuid,
    /// The relationship's type code, before it is mapped.
    pub(crate) code: Option<String>,
    /// The relationship's `time_validity`.
    pub(crate) validity: Option<Value>,
}

/// The identifier rows the parties should hold, with the professional date
/// ranges and the stored compared form beside each: the `wanted` relation the
/// two identifier statements of [`write_identifiers`] share, over the
/// parameters `$3` to `$11`.
///
/// A key row is held only while the `PERSON` performs a professional `ROLE`
/// or represents someone. A display name is held once per folded value, the
/// first in document order.
macro_rules! wanted_identifiers {
    () => {
        "WITH professional AS ( \
         SELECT t.party, range_agg(party.binding_validity(t.validity)) AS dates \
         FROM unnest($10::uuid[], $11::jsonb[]) AS t(party, validity) GROUP BY t.party \
     ), held AS ( \
         SELECT w.party, w.selector, w.issuer, w.identifier_type, w.value, \
                CASE WHEN w.selector IN ('user_name', 'group_display_name') \
                     THEN party.binding_fold(w.value) ELSE w.value END AS folded, \
                w.lifecycle_state, \
                CASE WHEN w.selector = 'key' THEN coalesce(pr.dates, '{}') \
                     ELSE '{}'::datemultirange END AS professional, \
                w.selector = 'key' AND w.representative AS representative, w.ord \
         FROM unnest($3::uuid[], $4::text[], $5::text[], $6::text[], $7::text[], $8::text[], \
                     $9::boolean[]) WITH ORDINALITY \
              AS w(party, selector, issuer, identifier_type, value, lifecycle_state, \
                   representative, ord) \
         LEFT JOIN professional pr ON pr.party = w.party \
         WHERE w.selector <> 'key' OR NOT isempty(coalesce(pr.dates, '{}')) OR w.representative \
     ), wanted AS ( \
         SELECT DISTINCT ON (h.party, h.selector, h.issuer, \
                             CASE WHEN h.selector = 'group_display_name' THEN h.folded \
                                  ELSE h.value END) \
                h.party, h.selector, h.issuer, h.identifier_type, h.value, h.folded, \
                h.lifecycle_state, h.professional, h.representative \
         FROM held h \
         ORDER BY h.party, h.selector, h.issuer, \
                  CASE WHEN h.selector = 'group_display_name' THEN h.folded ELSE h.value END, \
                  h.ord \
     ) "
    };
}

/// The condition that the side-table row `b` and the wanted row `w` are the
/// same row.
macro_rules! same_identifier {
    () => {
        "w.party = b.party_id AND w.selector = b.selector \
     AND w.issuer = b.issuer AND w.identifier_type = b.identifier_type AND w.value = b.value \
     AND w.folded = b.folded AND w.lifecycle_state = b.lifecycle_state \
     AND w.professional = b.professional AND w.representative = b.representative"
    };
}

/// Retires the live identifier rows the parties no longer hold.
const RETIRE_IDENTIFIERS: &str = concat!(
    wanted_identifiers!(),
    "UPDATE binding_identifier b SET retired_at = now() \
     WHERE b.party_id = ANY($1) AND b.selector = ANY($2) AND b.retired_at IS NULL \
       AND NOT EXISTS (SELECT 1 FROM wanted w WHERE ",
    same_identifier!(),
    ")"
);
/// Inserts the held identifier rows with no live twin.
const INSERT_IDENTIFIERS: &str = concat!(
    wanted_identifiers!(),
    "INSERT INTO binding_identifier \
         (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
          professional, representative) \
     SELECT w.party, w.selector, w.issuer, w.identifier_type, w.value, w.folded, \
            w.lifecycle_state, w.professional, w.representative \
     FROM wanted w \
     WHERE NOT EXISTS (SELECT 1 FROM binding_identifier b \
                       WHERE b.party_id = w.party AND b.retired_at IS NULL AND ",
    same_identifier!(),
    ")"
);
/// Writes the identifier rows of `selectors` the parties `parties` should
/// hold: `rows`, with the key rows' date ranges from `professional`.
///
/// The retirement runs before the insert, as its own statement, so a value
/// that moves between rows never meets its old row in a unique index.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, a unique violation
/// included.
pub(crate) async fn write_identifiers(
    tx: &mut PgConnection,
    parties: &[Uuid],
    selectors: &[&str],
    rows: &[IdentifierRow],
    professional: &[Professional],
) -> Result<(), StorageError> {
    if parties.is_empty() {
        return Ok(());
    }
    let columns = IdentifierColumns::of(rows, professional);
    for statement in [RETIRE_IDENTIFIERS, INSERT_IDENTIFIERS] {
        columns
            .bind(sqlx::query(statement).bind(parties).bind(selectors))
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// The identifier rows and professional validities as the arrays the
/// identifier statements bind from `$3` on.
struct IdentifierColumns<'a> {
    party: Vec<Uuid>,
    selector: Vec<&'a str>,
    issuer: Vec<&'a str>,
    identifier_type: Vec<&'a str>,
    value: Vec<&'a str>,
    lifecycle_state: Vec<&'a str>,
    representative: Vec<bool>,
    professional_party: Vec<Uuid>,
    professional_validity: Vec<Option<&'a Value>>,
}

impl<'a> IdentifierColumns<'a> {
    fn of(rows: &'a [IdentifierRow], professional: &'a [Professional]) -> Self {
        Self {
            party: rows.iter().map(|r| r.party).collect(),
            selector: rows.iter().map(|r| r.selector).collect(),
            issuer: rows.iter().map(|r| r.issuer.as_str()).collect(),
            identifier_type: rows.iter().map(|r| r.identifier_type.as_str()).collect(),
            value: rows.iter().map(|r| r.value.as_str()).collect(),
            lifecycle_state: rows.iter().map(|r| r.lifecycle_state.as_str()).collect(),
            representative: rows.iter().map(|r| r.representative).collect(),
            professional_party: professional.iter().map(|p| p.party).collect(),
            professional_validity: professional.iter().map(|p| p.validity.as_ref()).collect(),
        }
    }

    fn bind<'q>(
        &'q self,
        query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
        query
            .bind(&self.party)
            .bind(&self.selector)
            .bind(&self.issuer)
            .bind(&self.identifier_type)
            .bind(&self.value)
            .bind(&self.lifecycle_state)
            .bind(&self.representative)
            .bind(&self.professional_party)
            .bind(&self.professional_validity)
    }
}

/// Inserts the tombstones `rows`, the binding keys of deleted `PERSON`s,
/// retired and deprovisioned, for each party that carries none yet.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn insert_tombstones(
    tx: &mut PgConnection,
    rows: &[IdentifierRow],
) -> Result<(), StorageError> {
    if rows.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              retired_at, deprovisioned_at) \
         SELECT DISTINCT w.party, 'key', w.issuer, w.identifier_type, w.value, w.value, \
                w.lifecycle_state, now(), now() \
         FROM unnest($1::uuid[], $2::text[], $3::text[], $4::text[], $5::text[]) \
              AS w(party, issuer, identifier_type, value, lifecycle_state) \
         WHERE NOT EXISTS (SELECT 1 FROM binding_identifier t \
                           WHERE t.party_id = w.party AND t.deprovisioned_at IS NOT NULL)",
    )
    .bind(rows.iter().map(|r| r.party).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.issuer.as_str()).collect::<Vec<_>>())
    .bind(
        rows.iter()
            .map(|r| r.identifier_type.as_str())
            .collect::<Vec<_>>(),
    )
    .bind(rows.iter().map(|r| r.value.as_str()).collect::<Vec<_>>())
    .bind(
        rows.iter()
            .map(|r| r.lifecycle_state.as_str())
            .collect::<Vec<_>>(),
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Clears the tombstone of every key a live row of `parties` binds again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn revive_keys(
    tx: &mut PgConnection,
    parties: &[Uuid],
) -> Result<(), StorageError> {
    if parties.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE binding_identifier t SET deprovisioned_at = NULL \
         FROM binding_identifier l \
         WHERE l.party_id = ANY($1) \
           AND l.selector = 'key' AND l.retired_at IS NULL \
           AND t.selector = 'key' AND t.deprovisioned_at IS NOT NULL \
           AND t.issuer = l.issuer AND t.identifier_type = l.identifier_type \
           AND t.value = l.value",
    )
    .bind(parties)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The representation rows the representatives should hold, their codes
/// mapped and their validities joined: the `wanted` relation of
/// [`write_representations`].
macro_rules! wanted_representations {
    () => {
        "WITH wanted AS ( \
             SELECT r.representative, r.subject, \
                    (SELECT c.mapped FROM unnest($6::text[], $7::text[]) AS c(code, mapped) \
                     WHERE c.code = r.code) AS relationship, \
                    range_agg(party.binding_validity(r.validity)) AS validity \
             FROM unnest($2::uuid[], $3::uuid[], $4::text[], $5::jsonb[]) \
                  AS r(representative, subject, code, validity) \
             GROUP BY 1, 2, 3 \
         ) "
    };
}
/// The condition that the side-table row `b` and the wanted row `w` are the
/// same representation.
macro_rules! same_representation {
    () => {
        "w.representative = b.representative_id AND w.subject = b.subject_id \
         AND w.relationship IS NOT DISTINCT FROM b.relationship AND w.validity = b.validity"
    };
}
/// Retires the live representation rows the representatives no longer hold.
const RETIRE_REPRESENTATIONS: &str = concat!(
    wanted_representations!(),
    "UPDATE binding_representation b SET retired_at = now() \
     WHERE b.representative_id = ANY($1) AND b.retired_at IS NULL \
       AND NOT EXISTS (SELECT 1 FROM wanted w WHERE ",
    same_representation!(),
    ")"
);
/// Inserts the held representation rows with no live twin.
const INSERT_REPRESENTATIONS: &str = concat!(
    wanted_representations!(),
    "INSERT INTO binding_representation \
         (representative_id, subject_id, relationship, validity) \
     SELECT w.representative, w.subject, w.relationship, w.validity FROM wanted w \
     WHERE NOT EXISTS (SELECT 1 FROM binding_representation b \
                       WHERE b.retired_at IS NULL AND ",
    same_representation!(),
    ")"
);
/// Writes the representation rows the representatives `parties` should hold:
/// `rows`, each type code mapped through `codes` and the validities of one
/// representative, subject and mapped code joined into one date range.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write_representations(
    tx: &mut PgConnection,
    parties: &[Uuid],
    rows: &[RepresentationRow],
    codes: (&[&str], &[&str]),
) -> Result<(), StorageError> {
    if parties.is_empty() {
        return Ok(());
    }
    let representative: Vec<Uuid> = rows.iter().map(|r| r.representative).collect();
    let subject: Vec<Uuid> = rows.iter().map(|r| r.subject).collect();
    let code: Vec<Option<&str>> = rows.iter().map(|r| r.code.as_deref()).collect();
    let validity: Vec<Option<&Value>> = rows.iter().map(|r| r.validity.as_ref()).collect();
    for statement in [RETIRE_REPRESENTATIONS, INSERT_REPRESENTATIONS] {
        sqlx::query(statement)
            .bind(parties)
            .bind(&representative)
            .bind(&subject)
            .bind(&code)
            .bind(&validity)
            .bind(codes.0)
            .bind(codes.1)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Writes the memberships the `GROUP`s `groups` should hold: `rows`, each a
/// `(group, member)` pair.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write_memberships(
    tx: &mut PgConnection,
    groups: &[Uuid],
    rows: &[(Uuid, Uuid)],
) -> Result<(), StorageError> {
    if groups.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "WITH wanted AS ( \
             SELECT DISTINCT w.group_id, w.member_id \
             FROM unnest($2::uuid[], $3::uuid[]) AS w(group_id, member_id) \
         ), gone AS ( \
             UPDATE binding_membership m SET retired_at = now() \
             WHERE m.retired_at IS NULL AND m.group_id = ANY($1) \
               AND NOT EXISTS (SELECT 1 FROM wanted w \
                               WHERE w.group_id = m.group_id AND w.member_id = m.member_id) \
         ) \
         INSERT INTO binding_membership (group_id, member_id) \
         SELECT w.group_id, w.member_id FROM wanted w \
         WHERE NOT EXISTS (SELECT 1 FROM binding_membership m \
                           WHERE m.group_id = w.group_id AND m.member_id = w.member_id \
                             AND m.retired_at IS NULL)",
    )
    .bind(groups)
    .bind(rows.iter().map(|(group, _)| *group).collect::<Vec<_>>())
    .bind(rows.iter().map(|(_, member)| *member).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await?;
    Ok(())
}
