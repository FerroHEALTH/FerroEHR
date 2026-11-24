// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for the caller facts: `binding_caller_fact`, the record subjects
//! of `binding_record_subject` ([`super::subject`]) and the stamps of
//! `binding_caller_fact_party`.
//!
//! A commit of a `ROLE`, a `PERSON` or an `ORGANISATION` replaces the facts
//! its head holds, and a `PERSON` its record subjects, in its own
//! transaction; when they changed, it stamps each party whose facts changed
//! with its transaction's xid, writing no row another commit shares and
//! sending no notification, so commits about different parties never queue
//! on one row or on the notification queue's commit lock (#6074). Each
//! replica reads again only the facts of the parties stamped by transactions
//! its copy's snapshot did not see ([`FACTS`]), with their care
//! relationships (`CARED`) and record subjects, which its care copy applies
//! to the care teams it holds, once a request finds such a stamp
//! (`party.facts_behind`). A refill and an erasure move the grant revision
//! instead, which reads every fact again. No openEHR spec governs the side
//! table or its SQL: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the capability and relationship \
              `time_validity` fragments the fact rows are derived from"
)]

use std::sync::Arc;

use serde_json::Value;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::care_of::{CareFact, CarerKind, Cover, Episode, ProviderKey};
use super::kin::KinFact;
use super::{CallerFact, FactKind, FactValue};
use crate::db::snapshot::Snapshot;
use crate::party_binding::grants::store::{GrantHead, Term};
use crate::party_binding::step::{Returned, Step, Tag, differ, parties};
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;
use openehr_rm::draft::access_control::terminology::ForeignCode;

mod insert;

use insert::insert;

/// The stamps `$1`, a `pg_snapshot`, did not see, over
/// `binding_caller_fact_party p`: the xmin bound keeps it a range scan of
/// `ix_binding_caller_fact_party_stamped`, as `party.facts_behind` reads it.
macro_rules! unseen {
    () => {
        "p.stamped_xid >= pg_snapshot_xmin($1::pg_snapshot) \
         AND NOT pg_visible_in_snapshot(p.stamped_xid, $1::pg_snapshot)"
    };
}

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

/// The columns of one care relationship, over `binding_caller_fact f`: a
/// fact's and its episode's, which only a care relationship holds.
macro_rules! care_columns {
    () => {
        concat!(fact_columns!(), ", f.episode_issuer, f.episode_id")
    };
}

/// Every caller fact of the parties stamped by transactions the snapshot
/// `$1` did not see: the facts a replica's copy holds, so no care
/// relationship of a subject (#5615).
///
/// A stamped party that holds none reads one row of `NULL` facts. It is
/// answered from `ix_binding_caller_fact_party_stamped` and
/// `ix_binding_caller_fact_party`, and public so a test can `EXPLAIN` it.
pub const FACTS: &str = concat!(
    "SELECT p.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact_party p \
     LEFT JOIN binding_caller_fact f ON f.party_id = p.party_id AND f.kind <> 'under_care_of' \
     WHERE ",
    unseen!(),
    " ORDER BY p.party_id, f.holder_id, f.ordinal, f.kind"
);

/// Every caller fact, the read of a whole copy: the parties that hold none,
/// patients in care among them, are not read.
const HELD_FACTS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    fact_columns!(),
    " FROM binding_caller_fact f WHERE f.kind <> 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal, f.kind"
);

/// The care relationships of the subjects stamped by transactions the
/// snapshot `$1` did not see, by subject.
///
/// A replica applies them to the census of each care team it holds (#6074).
/// Answered from `ix_binding_caller_fact_party_stamped` and
/// `ix_binding_caller_fact_party`.
const CARED: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact_party p \
     JOIN binding_caller_fact f ON f.party_id = p.party_id AND f.kind = 'under_care_of' \
     WHERE ",
    unseen!(),
    " ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationships of the subject `$1`. Answered from
/// `ix_binding_caller_fact_party`.
const CARE_OF_SUBJECT: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact f WHERE f.party_id = $1 AND f.kind = 'under_care_of' \
     ORDER BY f.holder_id, f.ordinal"
);

/// The care relationships of each of a set of subjects.
const CARE_OF_SUBJECTS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact f WHERE f.party_id = ANY($1) AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationships to the team `$1`, one per subject under its care.
/// Answered from `ix_binding_caller_fact_care_team`, and public so a test can
/// `EXPLAIN` it.
pub const CARE_OF_TEAM: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact f WHERE f.target_id = $1 AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationships whose target is one of `$1`, by subject. Answered
/// from `ix_binding_caller_fact_care_team`.
const CARE_OF_CARERS: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact f WHERE f.target_id = ANY($1) AND f.kind = 'under_care_of' \
     ORDER BY f.party_id, f.holder_id, f.ordinal"
);

/// The care relationship the `PARTY_RELATIONSHIP` head `$1` holds. Answered
/// from `uq_binding_caller_fact`.
const CARE_OF_HOLDER: &str = concat!(
    "SELECT f.party_id AS stamped, ",
    care_columns!(),
    " FROM binding_caller_fact f WHERE f.holder_id = $1 AND f.kind = 'under_care_of'"
);

/// The record subjects the parties stamped by transactions the snapshot `$1`
/// did not see declare now, by `PERSON` and position, each with whether it
/// is live as [`super::subject::declared_by`] reads it: whose cached
/// resolution a replica drops, and which its care teams' censuses hold.
/// Answered from `ix_binding_caller_fact_party_stamped` and the primary key
/// of `binding_record_subject`.
const DECLARED: &str = concat!(
    "SELECT r.holder_id, r.namespace, r.subject_id, \
     r.lifecycle_state IN ('532', '553') AND NOT EXISTS (SELECT 1 FROM pmir_suppression s \
       WHERE s.kind = 'subject' AND s.holder_id = r.holder_id AND s.namespace = r.namespace \
       AND lower(s.value) = lower(r.subject_id)) AS live \
     FROM binding_caller_fact_party p \
     JOIN binding_record_subject r ON r.holder_id = p.party_id \
     WHERE ",
    unseen!(),
    " ORDER BY r.holder_id, r.ordinal"
);

/// The text of one stored fact, as a commit compares the facts of a head
/// before and after; an episode adds to it only where a fact holds one.
const FACT_TEXT: &str = "ROW(kind, ordinal, party_id, terminology_id, code_string, target_id, \
     target_kind, provider_issuer, provider_id, self_declared, writer_id, lifecycle_state, \
     role_validity, validity, not_before, not_after, holder_version, covers_issuer, \
     covers_id, covers_kind)::text \
     || CASE WHEN episode_id IS NULL THEN '' ELSE ROW(episode_issuer, episode_id)::text END";

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
            FactValue::Kin(kin) => kin
                .kinship
                .terminology
                .capacity()
                .saturating_add(kin.kinship.code.capacity()),
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
                }))
                .saturating_add(care.episode.as_ref().map_or(0, |episode| {
                    episode
                        .issuer
                        .capacity()
                        .saturating_add(episode.id.capacity())
                })),
        };
        value
            .saturating_add(self.term.lifecycle_state.capacity())
            .saturating_add(self.version.as_ref().map_or(0, String::capacity))
    }
}

/// The facts read since a snapshot: the snapshot of the read, every party
/// stamped by a transaction the earlier one did not see, the caller facts
/// each holds, and the care relationships and record subjects of the stamped
/// parties now.
#[derive(Debug, Clone)]
pub(crate) struct Changed {
    /// The snapshot the read ran under.
    pub(crate) snapshot: Arc<Snapshot>,
    /// Every party read, sorted, each with its caller facts; empty facts for
    /// a party that holds none, such as a patient in care.
    pub(crate) parties: Vec<(Uuid, Vec<FactRow>)>,
    /// The care relationships of each stamped subject that holds any.
    pub(crate) cared: Vec<(Uuid, Vec<FactRow>)>,
    /// The record subjects the stamped parties declare now.
    pub(crate) declared: Vec<Declared>,
}

/// A record subject a stamped `PERSON` declares, as [`DECLARED`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    /// The `PERSON` that declares it.
    pub(crate) person: Uuid,
    /// The reference.
    pub(crate) subject: EhrSubject,
    /// Whether the `PERSON` declares it while it is `complete` or
    /// `incomplete` and no received PMIR message split it off.
    pub(crate) live: bool,
}

/// Replaces the facts of `head` on its commit transaction `tx`: the facts
/// about `party` (the performer of a `ROLE`, a `PERSON` or `ORGANISATION`
/// itself) its live body
/// holds, under the `ROLE`'s `role_validity`. When they changed, stamps every
/// party whose facts changed.
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

/// Stamps `parties`, when any, so each replica that finds the stamps drops
/// what it holds about them and reads it again; also what a received PMIR
/// message
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
    let (gone, came) = steps(head, party, role_validity);
    let mut rows = gone.run(tx).await?;
    let derived = head
        .live()
        .map_or(&[][..], |facts| facts.grants.facts.as_slice());
    if derived.iter().any(|fact| fact.about.or(party).is_some()) {
        rows.extend(came.run(tx).await?);
    }
    Ok(changed(&rows))
}

/// The two steps that replace the facts of `head` about `party`: the delete
/// of the facts it held and the insert of those its live body holds, each
/// returning the party and the text of every fact it wrote
/// ([`Tag::FactsGone`], [`Tag::FactsCame`]).
pub(crate) fn steps<'q>(
    head: GrantHead<'q>,
    party: Option<Uuid>,
    role_validity: Option<&'q Value>,
) -> (Step<'q>, Step<'q>) {
    let holder = head.vo_id;
    let gone = Step::new(
        "fd",
        format!(
            "DELETE FROM binding_caller_fact WHERE holder_id = $1 \
             RETURNING party_id AS p, {FACT_TEXT} AS t"
        ),
        Some(Tag::FactsGone),
        move |query| query.bind(holder),
    );
    let facts = head
        .live()
        .map_or(&[][..], |facts| facts.grants.facts.as_slice());
    let (facts, about): (Vec<&CallerFact>, Vec<Uuid>) = facts
        .iter()
        .filter_map(|fact| fact.about.or(party).map(|about| (fact, about)))
        .unzip();
    (gone, insert(head, about, role_validity, &facts))
}

/// Every party whose facts changed, from the rows the steps of [`steps`]
/// returned: none when the facts that came are the facts that went.
pub(crate) fn changed(rows: &[Returned]) -> Vec<Uuid> {
    if !differ(rows, Tag::FactsGone, Tag::FactsCame) {
        return Vec::new();
    }
    let mut moved = parties(rows, Tag::FactsGone);
    moved.extend(parties(rows, Tag::FactsCame));
    moved
}

/// Stamps `parties` with the xid of `tx`, on `tx`: the rows of those parties
/// alone, and no notification (#6074).
async fn stamp(tx: &mut PgConnection, parties: &[Uuid]) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO binding_caller_fact_party (party_id, stamped_xid) \
         SELECT p, pg_current_xact_id() FROM unnest($1::uuid[]) AS p \
         ON CONFLICT (party_id) DO UPDATE SET stamped_xid = EXCLUDED.stamped_xid",
    )
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

/// Reads the facts of every party stamped by a transaction `since` did not
/// see, or of every party when `since` is `None`, with the snapshot of the
/// read, on `conn`, which holds a repeatable-read snapshot.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and a stored kind or
/// value the side table's checks admit and this reading does not.
pub(crate) async fn load(
    conn: &mut PgConnection,
    since: Option<&Snapshot>,
) -> Result<Changed, StorageError> {
    let snapshot = Arc::new(crate::db::snapshot::current(conn).await?);
    let (rows, cared, declared) = match since.map(Snapshot::text) {
        Some(since) => {
            let declared: Vec<(Uuid, String, String, bool)> = sqlx::query_as(DECLARED)
                .bind(since)
                .fetch_all(&mut *conn)
                .await?;
            (
                sqlx::query(FACTS).bind(since).fetch_all(&mut *conn).await?,
                sqlx::query(CARED).bind(since).fetch_all(&mut *conn).await?,
                declared
                    .into_iter()
                    .map(|(person, namespace, id, live)| Declared {
                        person,
                        subject: EhrSubject::new(namespace, id),
                        live,
                    })
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
        snapshot,
        parties: by_party(rows)?,
        cared: by_party(cared)?,
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

/// Every party stamped by a transaction the snapshot `since` did not see,
/// read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn stamped_since(
    conn: &mut PgConnection,
    since: &Snapshot,
) -> Result<Vec<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(concat!(
        "SELECT p.party_id FROM binding_caller_fact_party p WHERE ",
        unseen!(),
        " ORDER BY p.party_id"
    ))
    .bind(since.text())
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
            episode: episode_of(row)?,
        }),
        (
            FactKind::Role | FactKind::OrganisationType,
            None,
            Some(terminology),
            Some(code),
            (None, None, None),
        ) => FactValue::Code(ForeignCode { terminology, code }),
        (FactKind::RelatedTo, Some(person), Some(terminology), Some(code), (None, None, None)) => {
            FactValue::Kin(KinFact {
                person,
                kinship: ForeignCode { terminology, code },
            })
        }
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

/// The episode the care relationship `row` holds, read by the care reads
/// alone ([`care_columns`]).
fn episode_of(row: &sqlx::postgres::PgRow) -> Result<Option<Episode>, StorageError> {
    match (
        row.try_get::<Option<String>, _>("episode_issuer")?,
        row.try_get::<Option<String>, _>("episode_id")?,
    ) {
        (None, None) => Ok(None),
        (Some(issuer), Some(id)) => Ok(Some(Episode::new(issuer, id))),
        _ => Err(StorageError::InvalidRows(
            "binding_caller_fact holds an episode ck_binding_caller_fact_episode refuses"
                .to_owned(),
        )),
    }
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
                CARED,
            ] {
                assert!(read.contains(column), "{column} is not read");
            }
        }
    }
}
