// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading and writing `linkage.subject_ehr`, the one relation of the linkage
//! domain.
//!
//! **No openEHR spec governs the storage — our own design/extension.** The
//! relation carries two kinds of row and this module is the only writer of
//! either: the party-to-EHR mapping the linkage service opens, and the EHR
//! Index association the SM `I_EHR_INDEX` operations record
//! (`docs/specs/openehr/SM/docs/openehr_platform/master07-ehr_index_service.adoc`).
//! Keeping both behind one module is what lets [`super`] and
//! [`crate::service::ehr_index`] stay a service seam over a store rather than
//! two dialects of the same table.
//!
//! Nothing here deletes. Every statement is scoped to the rows IN FORCE
//! (`upper_inf`) except the closes, which is what turns an in-force row into a
//! historical one: a merge, a split or an index correction closes a period and
//! opens the next, so "which party was the subject of this EHR when that
//! composition was written" stays answerable (GDPR Art. 5(1)(d) accuracy is
//! served by correcting forward, not by erasing the record of what was
//! believed). The one destructive path is erasure, which the role cannot
//! perform itself: [`erase_ehr`] calls the `SECURITY DEFINER` function the
//! migration set defines for it.
//!
//! Every statement names its relations unqualified and resolves through the
//! linkage pool's `search_path`, which carries neither `clinical` nor
//! `party`. A statement here therefore cannot reach either domain even if it
//! tried.

use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::ehr_index::types::{LocationDesc, ResourceStatus, SubjectRef};

/// `SQLSTATE` 23P01 `exclusion_violation` — what `PostgreSQL` reports when a
/// key carrying `WITHOUT OVERLAPS` is violated, that key being enforced by a
/// `GiST` exclusion index (`PostgreSQL` 18 docs § Appendix A "`PostgreSQL`
/// Error Codes", class 23,
/// <https://www.postgresql.org/docs/18/errcodes-appendix.html>).
const SQLSTATE_EXCLUSION_VIOLATION: &str = "23P01";

/// The columns every EHR Index read projects, written once so two reads of the
/// same association cannot drift apart.
const ASSOCIATION_COLUMNS: &str =
    "ehr_id, subject_id, subject_namespace, subject_type, status, location";

/// Whether `PostgreSQL` refused this statement because the party already holds
/// a mapping in force.
///
/// The temporal key is the enforcement, so the refusal arrives as a driver
/// error and is classified here once, rather than every caller reading
/// `SQLSTATE`s.
pub(super) fn is_overlap(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == SQLSTATE_EXCLUSION_VIOLATION)
}

/// A mapping that was just closed: the EHR it named, and the subject
/// identifier the row carried, if any.
///
/// The subject travels with the EHR, not with the party: a merge moves the
/// same EHR to another party and the clinical side still carries the same
/// `EHR_STATUS.subject.external_ref`, so the successor row keeps it; a split
/// moves the party to a DIFFERENT EHR, whose status this map has never seen,
/// so the successor row carries no subject until something writes one.
pub(super) struct ClosedMapping {
    /// The EHR the closed row named.
    pub(super) ehr: EhrId,
    /// The subject identifier the closed row carried.
    pub(super) subject: Option<SubjectRef>,
}

/// The EHR `party` is the subject of right now, or `None`.
///
/// # Errors
/// The driver error, when the read fails.
pub(super) async fn open_mapping(pool: &PgPool, party: VoId) -> Result<Option<EhrId>, sqlx::Error> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT ehr_id FROM subject_ehr WHERE party_id = $1 AND upper_inf(sys_period)",
    )
    .bind(party.0)
    .fetch_optional(pool)
    .await?;
    Ok(found.map(EhrId))
}

/// Open a mapping from `party` to `ehr`, starting now, optionally recording
/// the subject identifier the clinical side carries for it.
///
/// # Errors
/// The driver error, including the `23P01` refusal when `party` already holds
/// a mapping in force ([`is_overlap`]).
pub(super) async fn open(
    conn: &mut PgConnection,
    party: VoId,
    ehr: EhrId,
    subject: Option<&SubjectRef>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO subject_ehr \
         (party_id, ehr_id, subject_id, subject_namespace, subject_type) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(party.0)
    .bind(ehr.0)
    .bind(subject.map(|s| s.id.as_str()))
    .bind(subject.map(|s| s.namespace.as_str()))
    .bind(subject.map(|s| s.r#type.as_str()))
    .execute(conn)
    .await?;
    Ok(())
}

/// Close `party`'s mapping in force and return what it named, or `None` when
/// the party holds none.
///
/// The upper bound is `now()` — the TRANSACTION timestamp, so the closing row
/// and the successor the same transaction opens meet at one instant and the
/// half-open periods do not overlap.
///
/// # Errors
/// The driver error, when the update fails.
pub(super) async fn close(
    conn: &mut PgConnection,
    party: VoId,
) -> Result<Option<ClosedMapping>, sqlx::Error> {
    let closed: Option<PgRow> = sqlx::query(
        "UPDATE subject_ehr SET sys_period = tstzrange(lower(sys_period), now(), '[)') \
         WHERE party_id = $1 AND upper_inf(sys_period) \
         RETURNING ehr_id, subject_id, subject_namespace, subject_type",
    )
    .bind(party.0)
    .fetch_optional(conn)
    .await?;
    closed.as_ref().map(closed_mapping).transpose()
}

/// Reassemble a [`ClosedMapping`] from the `RETURNING` row of [`close`].
fn closed_mapping(row: &PgRow) -> Result<ClosedMapping, sqlx::Error> {
    let ehr: EhrId = row.try_get("ehr_id")?;
    let id: Option<String> = row.try_get("subject_id")?;
    let namespace: Option<String> = row.try_get("subject_namespace")?;
    let r#type: Option<String> = row.try_get("subject_type")?;
    // The pair is held together by a CHECK constraint, so an id without a
    // namespace cannot be stored; the match reads both rather than assuming it.
    let subject = match (id, namespace) {
        (Some(id), Some(namespace)) => Some(SubjectRef {
            id,
            namespace,
            r#type: r#type.unwrap_or_else(|| SubjectRef::DEFAULT_TYPE.to_owned()),
        }),
        _ => None,
    };
    Ok(ClosedMapping { ehr, subject })
}

/// Every EHR the `parties` are the subject of right now, distinct.
///
/// The batch form of [`open_mapping`], and the crossing step of a cohort query:
/// identifiers in, identifiers out, in one round trip. An empty input makes no
/// round trip at all — an `= ANY('{}')` would be a statement whose answer is
/// already known.
///
/// # Errors
/// The driver error, when the read fails.
pub(super) async fn open_mappings(
    pool: &PgPool,
    parties: &[VoId],
) -> Result<Vec<EhrId>, sqlx::Error> {
    if parties.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = parties.iter().map(|party| party.0).collect();
    let found: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT ehr_id FROM subject_ehr \
         WHERE party_id = ANY($1) AND upper_inf(sys_period)",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    Ok(found.into_iter().map(EhrId).collect())
}

/// Record the association of `subject` with `ehr`, replacing the metadata of
/// the association already in force.
///
/// The `ON CONFLICT` target is the partial unique index over the associations
/// in force, so re-adding a subject refreshes it while a CLOSED association of
/// the same pair stays untouched as history.
///
/// # Errors
/// The driver error, when the write fails.
pub(crate) async fn add_association(
    pool: &PgPool,
    ehr: EhrId,
    subject: &SubjectRef,
    status: &ResourceStatus,
    location: Option<&LocationDesc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO subject_ehr \
         (ehr_id, subject_id, subject_namespace, subject_type, status, location) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (ehr_id, subject_id, subject_namespace) \
             WHERE subject_id IS NOT NULL AND upper_inf(sys_period) \
         DO UPDATE SET subject_type = EXCLUDED.subject_type, \
                       status = EXCLUDED.status, location = EXCLUDED.location",
    )
    .bind(ehr.0)
    .bind(&subject.id)
    .bind(&subject.namespace)
    .bind(&subject.r#type)
    .bind(sqlx::types::Json(status))
    .bind(location.map(sqlx::types::Json))
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace the `RESOURCE_STATUS` of the association in force, returning how
/// many rows matched.
///
/// # Errors
/// The driver error, when the update fails.
pub(crate) async fn set_association_status(
    pool: &PgPool,
    ehr: EhrId,
    subject: &SubjectRef,
    status: &ResourceStatus,
) -> Result<u64, sqlx::Error> {
    let updated = sqlx::query(
        "UPDATE subject_ehr SET status = $4 \
         WHERE ehr_id = $1 AND subject_id = $2 AND subject_namespace = $3 \
           AND upper_inf(sys_period)",
    )
    .bind(ehr.0)
    .bind(&subject.id)
    .bind(&subject.namespace)
    .bind(sqlx::types::Json(status))
    .execute(pool)
    .await?;
    Ok(updated.rows_affected())
}

/// Replace (or clear) the `LOCATION_DESC` of the association in force,
/// returning how many rows matched.
///
/// # Errors
/// The driver error, when the update fails.
pub(crate) async fn set_association_location(
    pool: &PgPool,
    ehr: EhrId,
    subject: &SubjectRef,
    location: Option<&LocationDesc>,
) -> Result<u64, sqlx::Error> {
    let updated = sqlx::query(
        "UPDATE subject_ehr SET location = $4 \
         WHERE ehr_id = $1 AND subject_id = $2 AND subject_namespace = $3 \
           AND upper_inf(sys_period)",
    )
    .bind(ehr.0)
    .bind(&subject.id)
    .bind(&subject.namespace)
    .bind(location.map(sqlx::types::Json))
    .execute(pool)
    .await?;
    Ok(updated.rows_affected())
}

/// The shared body of the two closes.
///
/// Closed, not deleted: the SM's `remove_ehr_subject` and `remove_subject` END
/// an association, and ending one is exactly what a closed period records. The
/// role holds no `DELETE` to do otherwise with.
///
/// A row may carry a party mapping as well as the association — one written by
/// `link_as_subject`, which records the pseudonym it put on `EHR_STATUS` — and
/// ending the association must not end the mapping. So a closed row that named
/// a party is reopened in the same statement without its subject: the
/// association leaves force, the mapping stays, and the history keeps the
/// period during which the two were one row. The successor's period opens at
/// the same transaction timestamp the closed one ends at, so the two meet
/// without overlapping.
const CLOSE_ASSOCIATIONS: &str = concat!(
    "WITH closed AS (",
    "  UPDATE subject_ehr SET sys_period = tstzrange(lower(sys_period), now(), '[)')",
    "  WHERE {predicate} AND upper_inf(sys_period)",
    "  RETURNING party_id, ehr_id",
    "), reopened AS (",
    "  INSERT INTO subject_ehr (party_id, ehr_id)",
    "  SELECT party_id, ehr_id FROM closed WHERE party_id IS NOT NULL",
    ") SELECT count(*) FROM closed",
);

/// [`CLOSE_ASSOCIATIONS`] with its one placeholder filled by a predicate this
/// module wrote itself.
fn close_statement(predicate: &str) -> String {
    CLOSE_ASSOCIATIONS.replace("{predicate}", predicate)
}

/// Close the association of `subject` with `ehr`, returning how many rows
/// matched.
///
/// # Errors
/// The driver error, when the statement fails.
pub(crate) async fn close_association(
    pool: &PgPool,
    ehr: EhrId,
    subject: &SubjectRef,
) -> Result<u64, sqlx::Error> {
    let closed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(close_statement(
        "ehr_id = $1 AND subject_id = $2 AND subject_namespace = $3",
    )))
    .bind(ehr.0)
    .bind(&subject.id)
    .bind(&subject.namespace)
    .fetch_one(pool)
    .await?;
    Ok(closed.unsigned_abs())
}

/// Close every association of `subject`, whichever EHR it names, returning how
/// many rows matched.
///
/// # Errors
/// The driver error, when the statement fails.
pub(crate) async fn close_subject_associations(
    pool: &PgPool,
    subject: &SubjectRef,
) -> Result<u64, sqlx::Error> {
    let closed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(close_statement(
        "subject_id = $1 AND subject_namespace = $2",
    )))
    .bind(&subject.id)
    .bind(&subject.namespace)
    .fetch_one(pool)
    .await?;
    Ok(closed.unsigned_abs())
}

/// The associations of `ehr` in force, ordered by subject key.
///
/// # Errors
/// The driver error, when the read fails.
pub(crate) async fn ehr_associations(pool: &PgPool, ehr: EhrId) -> Result<Vec<PgRow>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSOCIATION_COLUMNS} FROM subject_ehr \
         WHERE ehr_id = $1 AND subject_id IS NOT NULL AND upper_inf(sys_period) \
         ORDER BY subject_id, subject_namespace"
    )))
    .bind(ehr.0)
    .fetch_all(pool)
    .await
}

/// The associations of `subject` in force, ordered by EHR id.
///
/// # Errors
/// The driver error, when the read fails.
pub(crate) async fn subject_associations(
    pool: &PgPool,
    subject: &SubjectRef,
) -> Result<Vec<PgRow>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ASSOCIATION_COLUMNS} FROM subject_ehr \
         WHERE subject_id = $1 AND subject_namespace = $2 AND upper_inf(sys_period) \
         ORDER BY ehr_id"
    )))
    .bind(&subject.id)
    .bind(&subject.namespace)
    .fetch_all(pool)
    .await
}

/// The subjects that more than one EHR is associated with, in force.
///
/// # Errors
/// The driver error, when the read fails.
pub(crate) async fn subjects_with_multiple_ehrs(pool: &PgPool) -> Result<Vec<PgRow>, sqlx::Error> {
    sqlx::query(
        "SELECT subject_id, subject_namespace FROM subject_ehr \
         WHERE subject_id IS NOT NULL AND upper_inf(sys_period) \
         GROUP BY subject_id, subject_namespace HAVING count(DISTINCT ehr_id) > 1 \
         ORDER BY subject_id, subject_namespace",
    )
    .fetch_all(pool)
    .await
}

/// The EHRs that more than one subject is associated with, in force.
///
/// # Errors
/// The driver error, when the read fails.
pub(crate) async fn ehrs_with_multiple_subjects(pool: &PgPool) -> Result<Vec<EhrId>, sqlx::Error> {
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT ehr_id FROM subject_ehr \
         WHERE subject_id IS NOT NULL AND upper_inf(sys_period) \
         GROUP BY ehr_id HAVING count(*) > 1 ORDER BY ehr_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(EhrId).collect())
}

/// Remove every row naming `ehr`, in force or historical, returning how many.
///
/// The one destructive statement in this domain, and the role cannot issue it:
/// it calls the `SECURITY DEFINER` function the migration set defines, which
/// runs as its owner and deletes only rows naming the one EHR it is given. An
/// EHR that has been erased under GDPR Art. 17(1)
/// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>) must not keep a row here
/// asserting whose record it was.
///
/// # Errors
/// The driver error, when the call fails — including the privilege refusal
/// when the deployment has not granted the role `EXECUTE` on the function.
pub(crate) async fn erase_ehr(pool: &PgPool, ehr: EhrId) -> Result<u64, sqlx::Error> {
    let erased: i64 = sqlx::query_scalar("SELECT erase_ehr($1)")
        .bind(ehr.0)
        .fetch_one(pool)
        .await?;
    Ok(erased.unsigned_abs())
}
