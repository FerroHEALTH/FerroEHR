// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the caller facts: `binding_caller_fact`, the record subjects
//! of `binding_record_subject` ([`super::subject`]), the stamps of
//! `binding_caller_fact_party` and the facts revision.
//!
//! A commit of a `ROLE`, a `PERSON` or an `ORGANISATION` replaces the facts
//! its head holds, and a `PERSON` its record subjects, in its own
//! transaction; when they changed, it moves the facts revision,
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

use super::care_of::{CareFact, CarerKind, Cover, ProviderKey};
use super::{CallerFact, FactKind, FactValue};
use crate::party_binding::grants::derive::Bound;
use crate::party_binding::grants::store::{CHANNEL, GrantHead, Term};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// The columns of one fact, over `binding_caller_fact f`.
macro_rules! fact_columns {
    () => {
        "f.holder_id, f.kind, f.terminology_id, f.code_string, f.target_id, f.target_kind, \
         f.provider_issuer, f.provider_id, f.self_declared, f.writer_id, f.lifecycle_state, \
         f.holder_version, f.covers_issuer, f.covers_id, f.covers_kind, \
         isempty(f.role_validity) AS role_empty, \
         lower(f.role_validity) AS role_lower, upper(f.role_validity) AS role_upper, \
         isempty(f.validity) AS empty, lower(f.validity) AS lower, upper(f.validity) AS upper, \
         f.not_before, f.not_after"
    };
}

/// Every caller fact of the parties stamped after the facts revision `$1`:
/// the facts a replica's copy holds, so no care relationship of a subject
/// (#5615).
///
/// A stamped party that holds none reads one row of `NULL` facts. It is
/// answered from `ix_binding_caller_fact_party_revision` and
/// `ix_binding_caller_fact_party`, and public so a test can `EXPLAIN` it.
pub const FACTS: &str = concat!(
    "SELECT p.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact_party p \
     LEFT JOIN binding_caller_fact f ON f.party_id = p.party_id AND f.kind <> 'under_care_of' \
     WHERE p.revision > $1 \
     ORDER BY p.party_id, f.holder_id, f.ordinal, f.kind"
);

/// Every caller fact, the read of a whole copy: the parties that hold none,
/// patients in care among them, are not read.
const HELD_FACTS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.kind <> 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal, f.kind"
);

/// The care teams the subjects stamped after the facts revision `$1` are
/// under the care of now, whose cached patients a replica drops.
const CARED_TEAMS: &str = "SELECT DISTINCT f.target_id FROM binding_caller_fact_party p \
     JOIN binding_caller_fact f ON f.party_id = p.party_id AND f.kind = 'under_care_of' \
     WHERE p.revision > $1";

/// The care relationships of the subject `$1`. Answered from
/// `ix_binding_caller_fact_party`.
const CARE_OF_SUBJECT: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.party_id = $1 AND f.kind = 'under_care_of' \
     ORDER BY f.holder_id, f.ordinal"
);

/// The care relationships of each of a set of subjects.
const CARE_OF_SUBJECTS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.party_id = ANY($1) AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationships to the team `$1`, one per subject under its care.
/// Answered from `ix_binding_caller_fact_care_team`, and public so a test can
/// `EXPLAIN` it.
pub const CARE_OF_TEAM: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.target_id = $1 AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationships whose target is one of `$1`, by subject. Answered
/// from `ix_binding_caller_fact_care_team`.
const CARE_OF_CARERS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.target_id = ANY($1) AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationship the `PARTY_RELATIONSHIP` head `$1` holds. Answered
/// from `uq_binding_caller_fact`.
const CARE_OF_HOLDER: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.holder_id = $1 AND f.kind = 'under_care_of'"
);

/// The record subjects the parties stamped after the facts revision `$1`
/// declare now, whose cached resolution a replica drops; answered from
/// `ix_binding_caller_fact_party_revision` and the primary key of
/// `binding_record_subject`.
const DECLARED: &str = "SELECT r.namespace, r.subject_id FROM binding_caller_fact_party p \
     JOIN binding_record_subject r ON r.holder_id = p.party_id \
     WHERE p.revision > $1";

/// The text of one stored fact, as a commit compares the facts of a head
/// before and after.
const FACT_TEXT: &str = "ROW(kind, ordinal, party_id, terminology_id, code_string, target_id, \
     target_kind, provider_issuer, provider_id, self_declared, writer_id, lifecycle_state, \
     role_validity, validity, not_before, not_after, holder_version, covers_issuer, \
     covers_id, covers_kind)::text";

/// The party that wrote the trunk head of the holder `$1`: the container its
/// commit audit's committer names in `external_ref`, the bound `PERSON` of a
/// professional or the client `ACTOR` of a system caller
/// ([`crate::versioning::committer::bound::server_committer`]); `NULL` for a
/// committer that names none. Draft access control decision procedure,
/// Writing Relationships: a caller is never admitted on a relationship it
/// wrote naming itself.
pub(crate) const WRITER: &str = "(SELECT CASE WHEN w.ref ~ \
     '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN w.ref::uuid END \
     FROM (SELECT lower(split_part(a.committer #>> '{external_ref,id,value}', '::', 1)) AS ref \
           FROM vo_head h JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
                AND v.sys_version = h.trunk_head_sys_version \
           JOIN commit_audit a ON a.id = v.commit_audit_id WHERE h.vo_id = $1) w)";

/// The `OBJECT_VERSION_ID` of the trunk head of the holder `$1`, the version
/// its facts are derived from, which the access log names for a care
/// relationship (draft access control decision procedure, Refusals).
const HOLDER_VERSION: &str = "(SELECT ext.version_uid(v.vo_id, v.creating_system_id, \
     v.trunk_version, v.branch_number, v.branch_version) \
     FROM vo_head h JOIN version v ON v.tier = h.tier AND v.vo_id = h.vo_id \
          AND v.sys_version = h.trunk_head_sys_version WHERE h.vo_id = $1)";

/// One stored fact about a party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactRow {
    /// The `ROLE`, `PERSON` or `ORGANISATION` whose head holds it.
    pub(crate) holder: Uuid,
    /// What it is.
    pub(crate) kind: FactKind,
    /// What it names.
    pub(crate) value: FactValue,
    /// When it holds.
    pub(crate) term: Term,
    /// The party that wrote the holder's head ([`WRITER`]); `None` when its
    /// committer names none.
    pub(crate) writer: Option<Uuid>,
    /// The `OBJECT_VERSION_ID` of the holder's head the fact was derived
    /// from ([`HOLDER_VERSION`]).
    pub(crate) version: Option<String>,
}

impl crate::memory::Weigh for FactRow {
    fn heap(&self) -> usize {
        let value = match &self.value {
            FactValue::Code(code) => code
                .terminology
                .capacity()
                .saturating_add(code.code.capacity()),
            FactValue::Team(_) => 0,
            FactValue::Care(care) => care
                .role
                .terminology
                .capacity()
                .saturating_add(care.role.code.capacity())
                .saturating_add(care.provider.issuer.capacity())
                .saturating_add(care.provider.id.capacity())
                .saturating_add(care.covers_for.as_ref().map_or(0, |covers| {
                    covers
                        .key
                        .issuer
                        .capacity()
                        .saturating_add(covers.key.id.capacity())
                })),
        };
        value
            .saturating_add(self.term.lifecycle_state.capacity())
            .saturating_add(self.version.as_ref().map_or(0, String::capacity))
    }
}

/// The facts read since a facts revision: the revision read, every party
/// stamped since, the caller facts each holds, and the care teams the
/// stamped subjects are under the care of now.
#[derive(Debug, Clone, Default)]
pub(crate) struct Changed {
    /// The facts revision the read saw.
    pub(crate) revision: i64,
    /// Every party read, each with its caller facts; empty facts for a party
    /// that holds none, such as a patient in care.
    pub(crate) parties: Vec<(Uuid, Vec<FactRow>)>,
    /// The care teams the stamped subjects are under the care of now.
    pub(crate) cared_teams: Vec<Uuid>,
    /// The record subjects the stamped parties declare now.
    pub(crate) declared: Vec<EhrSubject>,
}

/// Replaces the facts of `head` on its commit transaction `tx`: the facts
/// about `party` (the performer of a `ROLE`, a `PERSON` or `ORGANISATION`
/// itself) its live body
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
    let parties = replace(tx, head, party, role_validity).await?;
    finish(tx, parties).await
}

/// Replaces the facts, the record subjects and the patient identifiers of
/// the `PERSON` `head` on its commit transaction `tx`, as [`write()`] does,
/// stamping the `PERSON` too when its record subjects changed.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn write_person(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
) -> Result<(), StorageError> {
    let mut parties = replace(tx, head, Some(head.vo_id), None).await?;
    if super::subject::replace(tx, head).await? {
        parties.push(head.vo_id);
    }
    super::patient::replace(tx, head).await?;
    finish(tx, parties).await
}

/// Stamps `parties`, when any, moves the facts revision and notifies every
/// replica, so each drops what it holds about them and reads it again; also
/// what a received PMIR message
/// does when it splits an identifier or a record subject off a `PERSON`
/// ([`crate::service::demographic::identity_feed`]).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn finish(
    tx: &mut PgConnection,
    mut parties: Vec<Uuid>,
) -> Result<(), StorageError> {
    if parties.is_empty() {
        return Ok(());
    }
    parties.sort_unstable();
    parties.dedup();
    stamp(tx, &parties).await?;
    crate::party_binding::grants::live::note_change();
    Ok(())
}

/// Replaces the facts of `head` about `party`; every party whose facts
/// changed.
async fn replace(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
    party: Option<Uuid>,
    role_validity: Option<&Value>,
) -> Result<Vec<Uuid>, StorageError> {
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
        return Ok(Vec::new());
    }
    Ok(gone.iter().chain(&came).map(|(party, _)| *party).collect())
}

/// The texts of `rows`, sorted.
fn sorted(rows: &[(Uuid, String)]) -> Vec<&str> {
    let mut texts: Vec<&str> = rows.iter().map(|(_, text)| text.as_str()).collect();
    texts.sort_unstable();
    texts
}

/// The code a fact names: a role's, an organisation type's, or a care
/// relationship's `care_role`; `None` for a membership.
fn code_of(fact: &CallerFact) -> Option<&ForeignCode> {
    match &fact.value {
        FactValue::Code(code) => Some(code),
        FactValue::Care(care) => Some(&care.role),
        FactValue::Team(_) => None,
    }
}

/// What a care relationship fact states; `None` for any other fact.
fn care_of(fact: &CallerFact) -> Option<&CareFact> {
    match &fact.value {
        FactValue::Care(care) => Some(care),
        FactValue::Code(_) | FactValue::Team(_) => None,
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
        .map(|f| match &f.value {
            FactValue::Team(team) => Some(*team),
            FactValue::Care(care) => Some(care.carer),
            FactValue::Code(_) => None,
        })
        .collect();
    let target_kind: Vec<Option<&str>> = facts
        .iter()
        .map(|f| care_of(f).map(|care| care.kind.rm_type()))
        .collect();
    let provider_issuer: Vec<Option<&str>> = facts
        .iter()
        .map(|f| care_of(f).map(|care| care.provider.issuer.as_str()))
        .collect();
    let provider_id: Vec<Option<&str>> = facts
        .iter()
        .map(|f| care_of(f).map(|care| care.provider.id.as_str()))
        .collect();
    let self_declared: Vec<bool> = facts
        .iter()
        .map(|f| care_of(f).is_some_and(|care| care.self_declared))
        .collect();
    let covers: Vec<Option<&Cover>> = facts
        .iter()
        .map(|f| care_of(f).and_then(|care| care.covers_for.as_ref()))
        .collect();
    let covers_issuer: Vec<_> = covers.iter().map(|c| c.map(|c| &*c.key.issuer)).collect();
    let covers_id: Vec<_> = covers.iter().map(|c| c.map(|c| &*c.key.id)).collect();
    let covers_kind: Vec<_> = covers.iter().map(|c| c.map(|c| c.kind.rm_type())).collect();
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
              target_kind, provider_issuer, provider_id, self_declared, writer_id, \
              lifecycle_state, role_validity, validity, not_before, not_after, \
              holder_version, covers_issuer, covers_id, covers_kind) \
         SELECT $1, t.ordinal, t.kind, $2, t.terminology_id, t.code_string, t.target_id, \
                t.target_kind, t.provider_issuer, t.provider_id, t.self_declared, writer.id, $3, \
                party.binding_validity($4::jsonb), \
                CASE WHEN t.unreadable THEN 'empty'::daterange \
                     WHEN c.effective_until IS NOT NULL AND t.kind = 'under_care_of' \
                          AND NOT isempty(party.binding_validity(t.validity)) \
                     THEN daterange(lower(party.binding_validity(t.validity)), c.effective_until) \
                     ELSE party.binding_validity(t.validity) END, \
                t.not_before, t.not_after, holder.version, t.covers_issuer, t.covers_id, \
                t.covers_kind \
         FROM unnest($5::integer[], $6::text[], $7::text[], $8::text[], $9::uuid[], \
                     $10::text[], $11::text[], $12::text[], \
                     $13::jsonb[], $14::boolean[], $15::timestamptz[], $16::timestamptz[], \
                     $17::boolean[], $18::text[], $19::text[], $20::text[]) \
              AS t(ordinal, kind, terminology_id, code_string, target_id, target_kind, \
                   provider_issuer, provider_id, validity, unreadable, not_before, not_after, \
                   self_declared, covers_issuer, covers_id, covers_kind) \
         CROSS JOIN (SELECT {WRITER} AS id) writer \
         CROSS JOIN (SELECT {HOLDER_VERSION} AS version) holder \
         LEFT JOIN binding_care_validity c ON c.vo_id = $1 \
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
        .bind(&target_kind)
        .bind(&provider_issuer)
        .bind(&provider_id)
        .bind(&validity)
        .bind(&unreadable)
        .bind(&not_before)
        .bind(&not_after)
        .bind(&self_declared)
        .bind(&covers_issuer)
        .bind(&covers_id)
        .bind(&covers_kind)
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

/// Deletes every fact a party of `vo_ids` holds or is about, the kept ends
/// of the care relationships among them, and their stamps, on the erasure
/// transaction `tx`; whether any fact went. The
/// caller moves the grant revision, which reads every fact again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn erase(tx: &mut PgConnection, vo_ids: &[Uuid]) -> Result<bool, StorageError> {
    sqlx::query(
        "DELETE FROM binding_care_validity c WHERE c.vo_id = ANY($1) OR EXISTS ( \
             SELECT FROM binding_caller_fact f WHERE f.holder_id = c.vo_id \
               AND f.kind = 'under_care_of' AND f.party_id = ANY($1))",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?;
    let gone = sqlx::query(
        "DELETE FROM binding_caller_fact WHERE holder_id = ANY($1) OR party_id = ANY($1)",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let subjects = sqlx::query("DELETE FROM binding_record_subject WHERE holder_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    super::patient::erase(tx, vo_ids).await?;
    sqlx::query("DELETE FROM binding_caller_fact_party WHERE party_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    Ok(gone > 0 || subjects > 0)
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
    let revision = crate::party_binding::grants::store::revisions(conn)
        .await?
        .facts;
    let (rows, cared_teams, declared) = match since {
        Some(since) => {
            let declared: Vec<(String, String)> = sqlx::query_as(DECLARED)
                .bind(since)
                .fetch_all(&mut *conn)
                .await?;
            (
                sqlx::query(FACTS).bind(since).fetch_all(&mut *conn).await?,
                sqlx::query_scalar(CARED_TEAMS)
                    .bind(since)
                    .fetch_all(&mut *conn)
                    .await?,
                declared
                    .into_iter()
                    .map(|(namespace, id)| EhrSubject::new(namespace, id))
                    .collect(),
            )
        }
        None => (
            sqlx::query(HELD_FACTS).fetch_all(&mut *conn).await?,
            Vec::new(),
            Vec::new(),
        ),
    };
    Ok(Changed {
        revision,
        parties: by_party(rows)?,
        cared_teams,
        declared,
    })
}

/// The care relationships of `subject`, read on `conn`.
///
/// # Errors
/// As [`load`].
pub(crate) async fn care_of_subject(
    conn: &mut PgConnection,
    subject: Uuid,
) -> Result<Vec<FactRow>, StorageError> {
    let rows = sqlx::query(CARE_OF_SUBJECT)
        .bind(subject)
        .fetch_all(&mut *conn)
        .await?;
    Ok(by_party(rows)?
        .into_iter()
        .flat_map(|(_, facts)| facts)
        .collect())
}

/// The care relationships of each of `subjects`, by subject, read on
/// `conn`; a subject with none is absent.
///
/// # Errors
/// As [`load`].
pub(crate) async fn care_of_subjects(
    conn: &mut PgConnection,
    subjects: &[Uuid],
) -> Result<Vec<(Uuid, Vec<FactRow>)>, StorageError> {
    let rows = sqlx::query(CARE_OF_SUBJECTS)
        .bind(subjects)
        .fetch_all(&mut *conn)
        .await?;
    by_party(rows)
}

/// Every `PERSON` holding a care relationship, read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn cared_persons(conn: &mut PgConnection) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT party_id FROM binding_caller_fact WHERE kind = 'under_care_of' \
         ORDER BY party_id",
    )
    .fetch_all(&mut *conn)
    .await?)
}

/// Every party stamped after the facts revision `since`, read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn stamped_since(
    conn: &mut PgConnection,
    since: i64,
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT party_id FROM binding_caller_fact_party WHERE revision > $1 ORDER BY party_id",
    )
    .bind(since)
    .fetch_all(&mut *conn)
    .await?)
}

/// The care relationships to `team`, by subject, read on `conn`.
///
/// # Errors
/// As [`load`].
pub(crate) async fn care_of_team(
    conn: &mut PgConnection,
    team: Uuid,
) -> Result<Vec<(Uuid, Vec<FactRow>)>, StorageError> {
    let rows = sqlx::query(CARE_OF_TEAM)
        .bind(team)
        .fetch_all(&mut *conn)
        .await?;
    by_party(rows)
}

/// The care relationships whose target is one of `carers`, by subject, read
/// on `conn`: the reverse reading of the relationship resources.
///
/// # Errors
/// As [`load`].
pub(crate) async fn care_of_carers(
    conn: &mut PgConnection,
    carers: &[Uuid],
) -> Result<Vec<(Uuid, Vec<FactRow>)>, StorageError> {
    let rows = sqlx::query(CARE_OF_CARERS)
        .bind(carers)
        .fetch_all(&mut *conn)
        .await?;
    by_party(rows)
}

/// The care relationship the head of the `PARTY_RELATIONSHIP` `holder`
/// holds, with its subject, read on `conn`; `None` when its head holds none:
/// it is deleted, or no care relationship.
///
/// # Errors
/// As [`load`].
pub(crate) async fn care_of_holder(
    conn: &mut PgConnection,
    holder: Uuid,
) -> Result<Option<(Uuid, FactRow)>, StorageError> {
    let rows = sqlx::query(CARE_OF_HOLDER)
        .bind(holder)
        .fetch_all(&mut *conn)
        .await?;
    Ok(by_party(rows)?
        .into_iter()
        .find_map(|(subject, facts)| facts.into_iter().next().map(|fact| (subject, fact))))
}

/// `rows`, ordered by their `stamped` party, grouped by it; a party whose
/// row carries no fact holds none.
fn by_party(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<(Uuid, Vec<FactRow>)>, StorageError> {
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
    Ok(parties)
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
    let target_kind: Option<String> = row.try_get("target_kind")?;
    let provider_issuer: Option<String> = row.try_get("provider_issuer")?;
    let provider_id: Option<String> = row.try_get("provider_id")?;
    let self_declared: bool = row.try_get("self_declared")?;
    let covers_kind: Option<String> = row.try_get("covers_kind")?;
    let covers_for = match (
        row.try_get::<Option<String>, _>("covers_issuer")?,
        row.try_get::<Option<String>, _>("covers_id")?,
        covers_kind.as_deref().map(CarerKind::of),
    ) {
        (None, None, None) => None,
        (Some(issuer), Some(id), Some(Some(kind))) => {
            Some(Cover::new(ProviderKey::new(issuer, id), kind))
        }
        _ => {
            return Err(StorageError::InvalidRows(
                "binding_caller_fact holds a covers_for ck_binding_caller_fact_covers refuses"
                    .to_owned(),
            ));
        }
    };
    let care = (
        target_kind.as_deref().and_then(CarerKind::of),
        provider_issuer,
        provider_id,
    );
    let value = match (kind, target, terminology, code, care) {
        (FactKind::MemberOf, Some(team), None, None, (None, None, None)) => FactValue::Team(team),
        (
            FactKind::UnderCareOf,
            Some(carer),
            Some(terminology),
            Some(code),
            (Some(carer_kind), Some(issuer), Some(id)),
        ) => FactValue::Care(CareFact {
            carer,
            kind: carer_kind,
            role: ForeignCode { terminology, code },
            provider: ProviderKey::new(issuer, id),
            self_declared,
            covers_for,
        }),
        (
            FactKind::Role | FactKind::OrganisationType,
            None,
            Some(terminology),
            Some(code),
            (None, None, None),
        ) => FactValue::Code(ForeignCode { terminology, code }),
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
        writer: row.try_get("writer_id")?,
        version: row.try_get("holder_version")?,
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
            for read in [
                FACTS,
                HELD_FACTS,
                CARE_OF_SUBJECT,
                CARE_OF_SUBJECTS,
                CARE_OF_TEAM,
            ] {
                assert!(read.contains(column), "{column} is not read");
            }
        }
    }
}
