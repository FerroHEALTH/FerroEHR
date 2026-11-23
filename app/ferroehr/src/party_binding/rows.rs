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

use crate::party_binding::step::{self, Query, Step, Tag};
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
}

/// The validity a professional `ROLE` gives the `PERSON` that performs it.
#[derive(Debug, Clone)]
pub(crate) struct Professional {
    /// The performing `PERSON`'s container.
    pub(crate) party: Uuid,
    /// The `ROLE`'s `time_validity`.
    pub(crate) validity: Option<Value>,
}

/// One representation a representative holds.
#[derive(Debug, Clone)]
pub(crate) struct RepresentationRow {
    /// The representative's container.
    pub(crate) representative: Uuid,
    /// What the relationship states about the subject.
    pub(crate) representation: crate::party_binding::derive::Representation,
}

/// The identifier rows the parties should hold, with the professional date
/// ranges and the stored compared form beside each: the `wanted` relation the
/// two identifier steps share, over the parameters `$3` to `$10`.
///
/// Every key row is held, whatever the `PERSON` does: its professional date
/// ranges and its representative flag say what it is bound for, and a
/// `PERSON` that is neither binds as the person themself (#6106). A key row
/// is a representative's when its `PERSON` is, or ever was, the source of a
/// representation (a live standalone container, or any relationship whose
/// row was retired), so a former representative's login is never taken for
/// the patient's (#5532). A display name is held once per folded value, the
/// first in document order.
macro_rules! wanted_identifiers {
    () => {
        "WITH professional AS ( \
         SELECT t.party, range_agg(party.binding_validity(t.validity)) AS dates \
         FROM unnest($9::uuid[], $10::jsonb[]) AS t(party, validity) GROUP BY t.party \
     ), held AS ( \
         SELECT w.party, w.selector, w.issuer, w.identifier_type, w.value, \
                CASE WHEN w.selector IN ('user_name', 'group_display_name') \
                     THEN party.binding_fold(w.value) ELSE w.value END AS folded, \
                w.lifecycle_state, \
                CASE WHEN w.selector = 'key' THEN coalesce(pr.dates, '{}') \
                     ELSE '{}'::datemultirange END AS professional, \
                w.selector = 'key' AND EXISTS ( \
                    SELECT 1 FROM binding_representation r \
                    WHERE r.representative_id = w.party) AS representative, \
                w.ord \
         FROM unnest($3::uuid[], $4::text[], $5::text[], $6::text[], $7::text[], $8::text[]) \
              WITH ORDINALITY \
              AS w(party, selector, issuer, identifier_type, value, lifecycle_state, ord) \
         LEFT JOIN professional pr ON pr.party = w.party \
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
       AND ($11::text IS NULL OR b.issuer = $11) \
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

/// The identifier rows a write holds: the selectors, and the one issuer it
/// is limited to when it is given.
pub(crate) type Selectors<'q> = (&'q [&'q str], Option<&'q str>);

/// Writes the identifier rows of `selectors`, under `issuer` alone when it is
/// given, the parties `parties` should hold: `rows`, with the key rows' date
/// ranges from `professional`.
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
    selectors: Selectors<'_>,
    rows: &[IdentifierRow],
    professional: &[Professional],
) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
    if parties.is_empty() {
        return Ok(());
    }
    retire_identifiers(parties, selectors, rows, professional)
        .run(tx)
        .await?;
    insert_identifiers(parties, selectors.0, rows, professional)
        .run(tx)
        .await?;
    Ok(())
}

/// The step that retires the live identifier rows of `selectors` of
/// `parties` that `rows` no longer hold, as [`write_identifiers`] does first.
pub(crate) fn retire_identifiers<'q>(
    parties: &'q [Uuid],
    (selectors, issuer): Selectors<'q>,
    rows: &'q [IdentifierRow],
    professional: &'q [Professional],
) -> Step<'q> {
    Step::new("ir", RETIRE_IDENTIFIERS, None, move |query| {
        IdentifierColumns::of(rows, professional)
            .bind(query.bind(parties).bind(selectors))
            .bind(issuer)
    })
}

/// The step that inserts each of `rows` with no live twin, as
/// [`write_identifiers`] does once the retirement ran.
pub(crate) fn insert_identifiers<'q>(
    parties: &'q [Uuid],
    selectors: &'q [&'q str],
    rows: &'q [IdentifierRow],
    professional: &'q [Professional],
) -> Step<'q> {
    Step::new("ii", INSERT_IDENTIFIERS, None, move |query| {
        IdentifierColumns::of(rows, professional).bind(query.bind(parties).bind(selectors))
    })
    .writing(!rows.is_empty())
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
            professional_party: professional.iter().map(|p| p.party).collect(),
            professional_validity: professional.iter().map(|p| p.validity.as_ref()).collect(),
        }
    }

    fn bind(self, query: Query<'a>) -> Query<'a> {
        query
            .bind(self.party)
            .bind(self.selector)
            .bind(self.issuer)
            .bind(self.identifier_type)
            .bind(self.value)
            .bind(self.lifecycle_state)
            .bind(self.professional_party)
            .bind(self.professional_validity)
    }
}

/// Inserts the tombstones `rows`, the binding keys of deleted `PERSON`s,
/// retired and deprovisioned, for each party that carries none yet under the
/// row's issuer.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn insert_tombstones(
    tx: &mut PgConnection,
    rows: &[IdentifierRow],
) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
    if rows.is_empty() {
        return Ok(());
    }
    tombstones(rows).run(tx).await?;
    Ok(())
}

/// The step [`insert_tombstones`] runs.
pub(crate) fn tombstones(rows: &[IdentifierRow]) -> Step<'_> {
    Step::new(
        "tb",
        "INSERT INTO binding_identifier \
             (party_id, selector, issuer, identifier_type, value, folded, lifecycle_state, \
              retired_at, deprovisioned_at) \
         SELECT DISTINCT w.party, 'key', w.issuer, w.identifier_type, w.value, w.value, \
                w.lifecycle_state, now(), now() \
         FROM unnest($1::uuid[], $2::text[], $3::text[], $4::text[], $5::text[]) \
              AS w(party, issuer, identifier_type, value, lifecycle_state) \
         WHERE NOT EXISTS (SELECT 1 FROM binding_identifier t \
                           WHERE t.party_id = w.party AND t.issuer = w.issuer \
                             AND t.deprovisioned_at IS NOT NULL)",
        None,
        move |query| {
            query
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
        },
    )
    .writing(!rows.is_empty())
}

/// Clears the tombstone of every key a live row of `parties` binds again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn revive_keys(
    tx: &mut PgConnection,
    parties: &[Uuid],
) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
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

/// The step that clears the tombstone of every key among `rows`, the rows a
/// settle holds live once it ran: [`revive_keys`] off the rows the parties
/// hold, in the statement that inserts them, which cannot read them back.
pub(crate) fn revive_held(rows: &[IdentifierRow]) -> Step<'_> {
    Step::new(
        "rk",
        "UPDATE binding_identifier t SET deprovisioned_at = NULL \
         FROM unnest($1::text[], $2::text[], $3::text[]) AS l(issuer, identifier_type, value) \
         WHERE t.selector = 'key' AND t.deprovisioned_at IS NOT NULL \
           AND t.issuer = l.issuer AND t.identifier_type = l.identifier_type \
           AND t.value = l.value",
        None,
        move |query| {
            let keys = || rows.iter().filter(|r| r.selector == "key");
            query
                .bind(keys().map(|r| r.issuer.as_str()).collect::<Vec<_>>())
                .bind(
                    keys()
                        .map(|r| r.identifier_type.as_str())
                        .collect::<Vec<_>>(),
                )
                .bind(keys().map(|r| r.value.as_str()).collect::<Vec<_>>())
        },
    )
    .writing(rows.iter().any(|r| r.selector == "key"))
}

/// The columns of a representation row, over `$2` to `$9`: the
/// representative, the subject, the basis, the provider, the validity and
/// the `subject_relationship` code.
macro_rules! representation_rows {
    () => {
        "unnest($2::uuid[], $3::uuid[], $4::text[], $5::text[], $6::text[], $7::text[], \
                $8::jsonb[], $9::text[]) WITH ORDINALITY \
         AS r(representative, subject, basis_terminology, basis_code, provider_issuer, \
              provider_id, validity, relationship, ord)"
    };
}
/// Retires the live row of the container `$1` unless the wanted row, of the
/// same representative, subject, basis, provider and validity, matches it,
/// returning the representative it named.
const RETIRE_STANDALONE: &str = concat!(
    "UPDATE binding_representation b SET retired_at = now() \
     WHERE b.relationship_id = $1 AND b.retired_at IS NULL \
       AND NOT EXISTS (SELECT 1 FROM ",
    representation_rows!(),
    " WHERE r.representative = b.representative_id AND r.subject = b.subject_id \
         AND r.basis_terminology = b.basis_terminology AND r.basis_code = b.basis_code \
         AND r.provider_issuer = b.provider_issuer AND r.provider_id = b.provider_id \
         AND r.relationship IS NOT DISTINCT FROM b.relationship \
         AND datemultirange(party.binding_validity(r.validity)) = b.validity) \
     RETURNING b.representative_id AS p, NULL::text AS t"
);
/// Inserts the wanted row of the container `$1` when it has no live row,
/// naming the writer of its head, `{WRITER}`
/// ([`crate::party_binding::grants::caller::store::WRITER`]).
const INSERT_STANDALONE: &str = concat!(
    "INSERT INTO binding_representation \
         (representative_id, subject_id, basis_terminology, basis_code, provider_issuer, \
          provider_id, validity, relationship, relationship_id, writer_id) \
     SELECT r.representative, r.subject, r.basis_terminology, r.basis_code, \
            r.provider_issuer, r.provider_id, datemultirange(party.binding_validity(r.validity)), \
            r.relationship, $1, {WRITER} \
     FROM ",
    representation_rows!(),
    " WHERE NOT EXISTS (SELECT 1 FROM binding_representation b \
                        WHERE b.relationship_id = $1 AND b.retired_at IS NULL)"
);

/// The step that retires the live row of the representation container
/// `container` unless it is `row`, returning the representative it named
/// ([`Tag::Representatives`]).
pub(crate) fn retire_standalone(container: Uuid, row: Option<&RepresentationRow>) -> Step<'_> {
    let columns = Columns::of(&row.into_iter().collect::<Vec<_>>());
    Step::new(
        "sr",
        RETIRE_STANDALONE,
        Some(Tag::Representatives),
        move |query| columns.bind_owned(query.bind(container)),
    )
}

/// The step that inserts `row` as the live row of the representation
/// container `container` when it has none, naming the writer of its head;
/// it runs a statement after [`retire_standalone`].
pub(crate) fn insert_standalone(container: Uuid, row: Option<&RepresentationRow>) -> Step<'_> {
    let columns = Columns::of(&row.into_iter().collect::<Vec<_>>());
    let insert = INSERT_STANDALONE.replace(
        "{WRITER}",
        crate::party_binding::grants::caller::store::WRITER,
    );
    Step::new("sn", insert, None, move |query| {
        columns.bind_owned(query.bind(container))
    })
    .writing(row.is_some())
}

/// The column arrays of some representation rows.
struct Columns<'r> {
    representative: Vec<Uuid>,
    subject: Vec<Uuid>,
    basis_terminology: Vec<&'r str>,
    basis_code: Vec<&'r str>,
    provider_issuer: Vec<&'r str>,
    provider_id: Vec<&'r str>,
    validity: Vec<Option<&'r Value>>,
    relationship: Vec<Option<&'r str>>,
}

impl<'r> Columns<'r> {
    /// The columns of `rows`.
    fn of(rows: &[&'r RepresentationRow]) -> Self {
        let held = |row: &&'r RepresentationRow| &row.representation;
        Self {
            representative: rows.iter().map(|r| r.representative).collect(),
            subject: rows.iter().map(|r| held(r).subject).collect(),
            basis_terminology: rows
                .iter()
                .map(|r| held(r).basis.terminology.as_str())
                .collect(),
            basis_code: rows.iter().map(|r| held(r).basis.code.as_str()).collect(),
            provider_issuer: rows
                .iter()
                .map(|r| held(r).provider.issuer.as_str())
                .collect(),
            provider_id: rows.iter().map(|r| held(r).provider.id.as_str()).collect(),
            validity: rows.iter().map(|r| held(r).validity.as_ref()).collect(),
            relationship: rows.iter().map(|r| held(r).code.as_deref()).collect(),
        }
    }

    /// `query` with the columns, which it takes, bound as `$2` to `$9`.
    fn bind_owned(self, query: Query<'r>) -> Query<'r> {
        query
            .bind(self.representative)
            .bind(self.subject)
            .bind(self.basis_terminology)
            .bind(self.basis_code)
            .bind(self.provider_issuer)
            .bind(self.provider_id)
            .bind(self.validity)
            .bind(self.relationship)
    }

    /// `query` with the columns bound as `$2` to `$9`.
    fn bind<'q>(&'q self, query: Query<'q>) -> Query<'q> {
        query
            .bind(&self.representative)
            .bind(&self.subject)
            .bind(&self.basis_terminology)
            .bind(&self.basis_code)
            .bind(&self.provider_issuer)
            .bind(&self.provider_id)
            .bind(&self.validity)
            .bind(&self.relationship)
    }
}

/// Inserts each of `held`, a representation with its container, as a
/// retired row: what the representatives once held, which a refill keeps.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write_history(
    tx: &mut PgConnection,
    held: &[(RepresentationRow, Uuid)],
) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
    if held.is_empty() {
        return Ok(());
    }
    let rows: Vec<&RepresentationRow> = held.iter().map(|(row, _)| row).collect();
    let containers: Vec<Uuid> = held.iter().map(|(_, container)| *container).collect();
    let columns = Columns::of(&rows);
    let statement = concat!(
        "INSERT INTO binding_representation \
             (representative_id, subject_id, basis_terminology, basis_code, provider_issuer, \
              provider_id, validity, relationship, relationship_id, retired_at) \
         SELECT r.representative, r.subject, r.basis_terminology, r.basis_code, \
                r.provider_issuer, r.provider_id, \
                datemultirange(party.binding_validity(r.validity)), r.relationship, \
                c.container, now() \
         FROM ",
        representation_rows!(),
        " JOIN unnest($1::uuid[]) WITH ORDINALITY AS c(container, ord) \
            ON c.ord = r.ord"
    );
    columns
        .bind(sqlx::query(statement).bind(&containers))
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// The memberships the member `PERSON`s should hold, over `$2` (each group)
/// and `$3` (each member), as the `wanted` relation the membership steps
/// share.
const WANTED_MEMBERSHIPS: &str = "WITH wanted AS ( \
         SELECT DISTINCT w.group_id, w.member_id \
         FROM unnest($2::uuid[], $3::uuid[]) AS w(group_id, member_id) \
     ) ";

/// Writes the memberships the member `PERSON`s `members` should hold:
/// `rows`, each a `(group, member)` pair.
///
/// A membership kept keeps its row, a removed one is retired and an added one
/// inserted, and the membership revision of every `GROUP` whose live rows
/// changed moves in the same statement, so its SCIM version changes with its
/// members (RFC 7644 §3.14).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write_memberships(
    tx: &mut PgConnection,
    members: &[Uuid],
    rows: &[(Uuid, Uuid)],
) -> Result<(), StorageError> {
    crate::party_binding::live::note_change();
    if members.is_empty() {
        return Ok(());
    }
    let steps = vec![
        retire_memberships(members, rows),
        add_memberships(members, rows),
        revise_groups(Retired::Beside),
    ];
    step::batch(tx, steps).await?;
    Ok(())
}

/// The step that retires each live membership of `members` that `rows` no
/// longer hold, returning each `GROUP` it retired a row of.
pub(crate) fn retire_memberships<'q>(members: &'q [Uuid], rows: &'q [(Uuid, Uuid)]) -> Step<'q> {
    let sql = format!(
        "{WANTED_MEMBERSHIPS}UPDATE binding_membership m SET retired_at = now() \
         WHERE m.retired_at IS NULL AND m.member_id = ANY($1) \
           AND NOT EXISTS (SELECT 1 FROM wanted w \
                           WHERE w.group_id = m.group_id AND w.member_id = m.member_id) \
         RETURNING m.group_id AS p, NULL::text AS t"
    );
    Step::new("mr", sql, Some(Tag::Groups), move |query| {
        bind_memberships(query, members, rows)
    })
}

/// The step that inserts each of `rows` with no live row, returning each
/// `GROUP` it added a row to.
pub(crate) fn add_memberships<'q>(members: &'q [Uuid], rows: &'q [(Uuid, Uuid)]) -> Step<'q> {
    let sql = format!(
        "{WANTED_MEMBERSHIPS}INSERT INTO binding_membership (group_id, member_id) \
         SELECT w.group_id, w.member_id FROM wanted w \
         WHERE NOT EXISTS (SELECT 1 FROM binding_membership m \
                           WHERE m.group_id = w.group_id AND m.member_id = w.member_id \
                             AND m.retired_at IS NULL) \
         RETURNING group_id AS p, NULL::text AS t"
    );
    Step::new("ma", sql, None, move |query| {
        bind_memberships(query, members, rows)
    })
    .writing(!rows.is_empty())
}

/// `query` with `members` and the groups and members of `rows` bound as `$1`
/// to `$3`.
fn bind_memberships<'q>(query: Query<'q>, members: &'q [Uuid], rows: &[(Uuid, Uuid)]) -> Query<'q> {
    query
        .bind(members)
        .bind(rows.iter().map(|(group, _)| *group).collect::<Vec<_>>())
        .bind(rows.iter().map(|(_, member)| *member).collect::<Vec<_>>())
}

/// The `GROUP`s whose membership rows [`retire_memberships`] retired, beside
/// those [`add_memberships`] added to.
#[derive(Debug)]
pub(crate) enum Retired {
    /// The retirement is a step of the same statement.
    Beside,
    /// The retirement ran in an earlier statement and retired rows of these.
    Before(Vec<Uuid>),
}

/// The step that moves the membership revision of every `GROUP` whose live
/// rows changed: those `retired` names and those the `ma` step of the same
/// statement added to.
pub(crate) fn revise_groups<'q>(retired: Retired) -> Step<'q> {
    let (gone, groups) = match retired {
        Retired::Beside => ("SELECT p FROM mr", None),
        Retired::Before(groups) => ("SELECT unnest($1::uuid[])", Some(groups)),
    };
    let writes = groups.as_ref().is_none_or(|groups| !groups.is_empty());
    let sql = format!(
        "INSERT INTO binding_group_revision (group_id, revision) \
         SELECT group_id, 1 FROM (SELECT p AS group_id FROM ma UNION {gone}) c \
         ON CONFLICT (group_id) DO UPDATE \
             SET revision = binding_group_revision.revision + 1, revised_at = now()"
    );
    Step::new("mv", sql, None, move |query| match groups {
        Some(groups) => query.bind(groups),
        None => query,
    })
    .writing(writes)
}
