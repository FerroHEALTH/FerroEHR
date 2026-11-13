// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the caller facts: `binding_caller_fact`, the stamps of
//! `binding_caller_fact_party` and the facts revision.
//!
//! A commit of a `ROLE` or a `PERSON` replaces the facts its head holds, in
//! its own transaction; when they changed, it moves the facts revision,
//! stamps each party whose facts changed with it and notifies
//! [`super::super::store::CHANNEL`], so each replica reads again only the
//! facts of the parties stamped since its copy ([`FACTS`]). A refill and an
//! erasure move the grant revision instead, which reads every fact again. No
//! openEHR spec governs the side table or its SQL: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the capability and relationship \
              `time_validity` fragments the fact rows are derived from"
)]

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::{CallerFact, FactKind, FactValue};
use crate::party_binding::grants::derive::Bound;
use crate::party_binding::grants::store::{CHANNEL, GrantHead, Term};
use crate::storage::error::StorageError;
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// Every fact of the parties stamped after the facts revision `$1`.
///
/// A stamped party that holds none reads one row of `NULL` facts. It is
/// answered from `ix_binding_caller_fact_party_revision` and
/// `ix_binding_caller_fact_party`, and public so a test can `EXPLAIN` it.
pub const FACTS: &str = "SELECT p.party_id AS stamped, f.holder_id, f.kind, f.terminology_id, \
            f.code_string, f.target_id, f.lifecycle_state, \
            isempty(f.role_validity) AS role_empty, lower(f.role_validity) AS role_lower, \
            upper(f.role_validity) AS role_upper, isempty(f.validity) AS empty, \
            lower(f.validity) AS lower, upper(f.validity) AS upper, f.not_before, f.not_after \
     FROM binding_caller_fact_party p \
     LEFT JOIN binding_caller_fact f ON f.party_id = p.party_id \
     WHERE p.revision > $1 \
     ORDER BY p.party_id, f.holder_id, f.ordinal, f.kind";

/// The text of one stored fact, as a commit compares the facts of a head
/// before and after.
const FACT_TEXT: &str = "ROW(kind, ordinal, party_id, terminology_id, code_string, target_id, \
     lifecycle_state, role_validity, validity, not_before, not_after)::text";

/// One stored fact about a party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactRow {
    /// The `ROLE` or `PERSON` whose head holds it.
    pub(crate) holder: Uuid,
    /// What it is.
    pub(crate) kind: FactKind,
    /// What it names.
    pub(crate) value: FactValue,
    /// When it holds.
    pub(crate) term: Term,
}

/// The facts read since a facts revision: the revision read, every party
/// stamped since, and the facts each holds.
#[derive(Debug, Clone, Default)]
pub(crate) struct Changed {
    /// The facts revision the read saw.
    pub(crate) revision: i64,
    /// Every party read, each with its facts; empty facts for a party that
    /// holds none any more.
    pub(crate) parties: Vec<(Uuid, Vec<FactRow>)>,
}

/// Replaces the facts of `head` on its commit transaction `tx`: the facts
/// about `party` (the performer of a `ROLE`, a `PERSON` itself) its live body
/// holds, under the `ROLE`'s `role_validity`. When they changed, moves the
/// facts revision, stamps every party whose facts changed and notifies.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
    party: Option<Uuid>,
    role_validity: Option<&Value>,
) -> Result<(), StorageError> {
    let statement = format!(
        "DELETE FROM binding_caller_fact WHERE holder_id = $1 \
         RETURNING party_id, {FACT_TEXT} AS fact"
    );
    let gone: Vec<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(statement))
        .bind(head.vo_id)
        .fetch_all(&mut *tx)
        .await?;
    let facts = head
        .live()
        .map_or(&[][..], |facts| facts.grants.facts.as_slice());
    let came = match party {
        Some(party) if !facts.is_empty() => insert(tx, head, party, role_validity, facts).await?,
        _ => Vec::new(),
    };
    if sorted(&gone) == sorted(&came) {
        return Ok(());
    }
    let mut parties: Vec<Uuid> = gone.iter().chain(&came).map(|(party, _)| *party).collect();
    parties.sort_unstable();
    parties.dedup();
    stamp(tx, &parties).await?;
    crate::party_binding::grants::live::note_change();
    Ok(())
}

/// The texts of `rows`, sorted.
fn sorted(rows: &[(Uuid, String)]) -> Vec<&str> {
    let mut texts: Vec<&str> = rows.iter().map(|(_, text)| text.as_str()).collect();
    texts.sort_unstable();
    texts
}

/// The code a clearance or a purpose names; `None` for a care relationship.
fn code_of(fact: &CallerFact) -> Option<&ForeignCode> {
    match &fact.value {
        FactValue::Code(code) => Some(code),
        FactValue::Team(_) => None,
    }
}

/// Inserts `facts` of `head` about `party`; the inserted rows as their party
/// and text.
async fn insert(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
    party: Uuid,
    role_validity: Option<&Value>,
    facts: &[CallerFact],
) -> Result<Vec<(Uuid, String)>, StorageError> {
    let instant = |bound: Bound| match bound {
        Bound::At(at) => Some(jiff_sqlx::Timestamp::from(at)),
        Bound::Open | Bound::Unreadable => None,
    };
    let ordinal: Vec<i32> = facts.iter().map(|f| f.ordinal).collect();
    let kind: Vec<&str> = facts.iter().map(|f| f.kind.code()).collect();
    let terminology: Vec<Option<&str>> = facts
        .iter()
        .map(|f| code_of(f).map(|code| code.terminology.as_str()))
        .collect();
    let code_string: Vec<Option<&str>> = facts
        .iter()
        .map(|f| code_of(f).map(|code| code.code.as_str()))
        .collect();
    let target: Vec<Option<Uuid>> = facts
        .iter()
        .map(|f| match f.value {
            FactValue::Team(team) => Some(team),
            FactValue::Code(_) => None,
        })
        .collect();
    let validity: Vec<Option<&Value>> = facts.iter().map(|f| f.validity.as_ref()).collect();
    let unreadable: Vec<bool> = facts
        .iter()
        .map(|f| f.not_before == Bound::Unreadable || f.not_after == Bound::Unreadable)
        .collect();
    let not_before: Vec<_> = facts.iter().map(|f| instant(f.not_before)).collect();
    let not_after: Vec<_> = facts.iter().map(|f| instant(f.not_after)).collect();
    let statement = format!(
        "INSERT INTO binding_caller_fact \
             (holder_id, ordinal, kind, party_id, terminology_id, code_string, target_id, \
              lifecycle_state, role_validity, validity, not_before, not_after) \
         SELECT $1, t.ordinal, t.kind, $2, t.terminology_id, t.code_string, t.target_id, $3, \
                party.binding_validity($4::jsonb), \
                CASE WHEN t.unreadable THEN 'empty'::daterange \
                     ELSE party.binding_validity(t.validity) END, \
                t.not_before, t.not_after \
         FROM unnest($5::integer[], $6::text[], $7::text[], $8::text[], $9::uuid[], \
                     $10::jsonb[], $11::boolean[], $12::timestamptz[], $13::timestamptz[]) \
              AS t(ordinal, kind, terminology_id, code_string, target_id, validity, unreadable, \
                   not_before, not_after) \
         ON CONFLICT DO NOTHING \
         RETURNING party_id, {FACT_TEXT} AS fact"
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(statement))
        .bind(head.vo_id)
        .bind(party)
        .bind(head.lifecycle_state)
        .bind(role_validity)
        .bind(&ordinal)
        .bind(&kind)
        .bind(&terminology)
        .bind(&code_string)
        .bind(&target)
        .bind(&validity)
        .bind(&unreadable)
        .bind(&not_before)
        .bind(&not_after)
        .fetch_all(&mut *tx)
        .await?)
}

/// Moves the facts revision, stamps `parties` with it and notifies
/// [`CHANNEL`], on `tx`.
async fn stamp(tx: &mut PgConnection, parties: &[Uuid]) -> Result<(), StorageError> {
    sqlx::query(
        "WITH revised AS ( \
             UPDATE binding_grant_revision SET facts_revision = facts_revision + 1, \
                 revised_at = now() \
             RETURNING facts_revision), \
         stamped AS ( \
             INSERT INTO binding_caller_fact_party (party_id, revision) \
             SELECT p, revised.facts_revision FROM revised, unnest($2::uuid[]) AS p \
             ON CONFLICT (party_id) DO UPDATE SET revision = EXCLUDED.revision) \
         SELECT pg_notify($1, 'facts ' || revised.facts_revision) FROM revised",
    )
    .bind(CHANNEL)
    .bind(parties)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The care relationships the `PERSON` `person` holds, as their texts,
/// sorted: what a commit compares before and after it writes them.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn care(tx: &mut PgConnection, person: Uuid) -> Result<Vec<String>, StorageError> {
    let statement = format!(
        "SELECT {FACT_TEXT} AS fact FROM binding_caller_fact \
         WHERE holder_id = $1 AND target_id IS NOT NULL ORDER BY fact"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
        .bind(person)
        .fetch_all(&mut *tx)
        .await?)
}

/// Deletes every fact a party of `vo_ids` holds or is about, and their
/// stamps, on the erasure transaction `tx`; whether any fact went. The
/// caller moves the grant revision, which reads every fact again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn erase(tx: &mut PgConnection, vo_ids: &[Uuid]) -> Result<bool, StorageError> {
    let gone = sqlx::query(
        "DELETE FROM binding_caller_fact WHERE holder_id = ANY($1) OR party_id = ANY($1)",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM binding_caller_fact_party WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    Ok(gone > 0)
}

/// Reads the facts of every party stamped after `since`, or of every party
/// when `since` is `None`, with the facts revision, on `conn`, which holds a
/// repeatable-read snapshot or reads both in one.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and a stored kind or
/// value the side table's checks admit and this reading does not.
pub(crate) async fn load(
    conn: &mut PgConnection,
    since: Option<i64>,
) -> Result<Changed, StorageError> {
    let (_, revision) = crate::party_binding::grants::store::revisions(conn).await?;
    let rows = sqlx::query(FACTS)
        .bind(since.unwrap_or(-1))
        .fetch_all(&mut *conn)
        .await?;
    let mut parties: Vec<(Uuid, Vec<FactRow>)> = Vec::new();
    for row in rows {
        let stamped: Uuid = row.try_get("stamped")?;
        if parties.last().is_none_or(|(party, _)| *party != stamped) {
            parties.push((stamped, Vec::new()));
        }
        let Some(holder) = row.try_get::<Option<Uuid>, _>("holder_id")? else {
            continue;
        };
        let fact = fact_row(holder, &row)?;
        if let Some((_, facts)) = parties.last_mut() {
            facts.push(fact);
        }
    }
    Ok(Changed { revision, parties })
}

/// The fact `row` of `holder` holds.
fn fact_row(holder: Uuid, row: &sqlx::postgres::PgRow) -> Result<FactRow, StorageError> {
    let kind: String = row.try_get("kind")?;
    let Some(kind) = FactKind::of(&kind) else {
        return Err(StorageError::InvalidRows(format!(
            "binding_caller_fact holds the kind {kind:?}, which ck_binding_caller_fact_kind refuses"
        )));
    };
    let target: Option<Uuid> = row.try_get("target_id")?;
    let terminology: Option<String> = row.try_get("terminology_id")?;
    let code: Option<String> = row.try_get("code_string")?;
    let value = match (target, terminology, code) {
        (Some(team), None, None) => FactValue::Team(team),
        (None, Some(terminology), Some(code)) => FactValue::Code(ForeignCode { terminology, code }),
        _ => {
            return Err(StorageError::InvalidRows(
                "binding_caller_fact holds a row ck_binding_caller_fact_value refuses".to_owned(),
            ));
        }
    };
    Ok(FactRow {
        holder,
        kind,
        value,
        term: Term::of(row)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::store::TERM_COLUMNS;

    #[test]
    fn the_facts_read_selects_the_term_columns() {
        for column in TERM_COLUMNS
            .split(',')
            .map(|column| column.split_whitespace().last().unwrap_or_default())
        {
            assert!(FACTS.contains(column), "{column} is not read");
        }
    }
}
