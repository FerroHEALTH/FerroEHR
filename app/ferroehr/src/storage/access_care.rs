// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care projection of the clinical domain (#5699).
//!
//! `clinical/0009_baseline_access.sql` holds the care relationships of the party
//! domain, one row per subject reference and relationship, each reference
//! held as its SHA-256 digest, and the digest of each EHR's own subject
//! reference beside it.
//!
//! The care projector writes the rows; a query's access condition probes
//! them by the EHR row's digest, the caller's principals and the providers
//! it acts for. No openEHR spec governs the projection: our own design.

use openehr_rm::draft::access_control::terminology::ForeignCode;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::db::snapshot::{self, Snapshot};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::care_of::CarerKind;
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The text a subject reference is digested from.
///
/// It is the namespace and the id joined by U+001F, which no
/// `OBJECT_REF.namespace` holds (BASE `OBJECT_REF`), the id of a reference in
/// this CDR's own namespace `own` in lower case, as the container UUID a
/// `PERSON` is named by is written.
#[must_use]
pub fn subject_key(own: &str, subject: &EhrSubject) -> String {
    if subject.namespace == own {
        format!("{}\u{1f}{}", subject.namespace, subject.id.to_lowercase())
    } else {
        format!("{}\u{1f}{}", subject.namespace, subject.id)
    }
}

/// The SHA-256 digest of the subject key `key`, [`subject_key`] or
/// [`cared_key`], the fixed-width value the projection and the EHR row are
/// matched by.
///
/// The reference is a pseudonym (data-minimisation rule 1), so its digest
/// needs no key; the clinical `ehr` row holds the reference itself.
#[must_use]
pub fn subject_digest(key: &str) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(key.as_bytes()).to_vec()
}

/// The text whose [`subject_digest`] is the check value of the digest form,
/// which tells the projector that the stored digests were taken in another
/// form and must be taken again.
pub const DIGEST_CHECK: &str = "ferroehr-care-subject-sha256";

/// The text a subject reference a `PERSON` is named by is digested from.
///
/// It is the reference as the party domain holds it, the namespace and the
/// id joined by U+001F with the id as written, so it matches an EHR's
/// [`subject_key`] exactly when the census matched it.
#[must_use]
pub fn cared_key(subject: &EhrSubject) -> String {
    format!("{}\u{1f}{}", subject.namespace, subject.id)
}

/// One projected care relationship of one subject reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CareRow {
    /// The digest of the subject reference.
    pub digest: Vec<u8>,
    /// The `PERSON` under care.
    pub person: Uuid,
    /// The carer: a `PERSON`, a `ROLE`, a `GROUP` or an `ORGANISATION`.
    pub carer: Uuid,
    /// What the carer is.
    pub carer_kind: CarerKind,
    /// The issuer of the provider in whose care it is.
    pub provider_issuer: String,
    /// The id of that provider.
    pub provider_id: String,
    /// The first date it holds; `None` when unbounded.
    pub from: Option<jiff::civil::Date>,
    /// The first date it no longer holds; `None` when unbounded.
    pub until: Option<jiff::civil::Date>,
    /// The party that wrote it; `None` when its committer names none.
    pub writer: Option<Uuid>,
    /// Whether its basis is `self_declared`.
    pub self_declared: bool,
    /// Its `care_role`.
    pub role: ForeignCode,
}

/// The code the projection's `carer_kind` column holds for `kind`.
#[must_use]
pub const fn kind_code(kind: CarerKind) -> &'static str {
    match kind {
        CarerKind::Person => "P",
        CarerKind::Role => "R",
        CarerKind::Group => "G",
        CarerKind::Organisation => "O",
    }
}

/// What the state row of the projection holds: the party snapshot it was
/// read under and the check value of the digest form its digests were taken
/// in, each `None` before the first run.
pub type State = (Option<Snapshot>, Option<Vec<u8>>);

/// How a projector reads the state row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRead {
    /// Without a lock, before the projector reads the party side.
    Unlocked,
    /// Locked until the transaction ends, so one projector writes at a time.
    Locked,
}

/// The [`State`] of the projection, read as `read` says.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn state(conn: &mut PgConnection, read: StateRead) -> Result<State, StorageError> {
    let sql = match read {
        StateRead::Unlocked => {
            "SELECT snapshot::text AS snapshot, key_check FROM access_care_state WHERE singleton"
        }
        StateRead::Locked => {
            "SELECT snapshot::text AS snapshot, key_check FROM access_care_state WHERE singleton \
             FOR UPDATE"
        }
    };
    let row = sqlx::query(sql).fetch_one(&mut *conn).await?;
    let snapshot: Option<String> = row.try_get("snapshot")?;
    Ok((
        snapshot.map(snapshot::read).transpose()?,
        row.try_get("key_check")?,
    ))
}

/// Record that the projection was read under the party snapshot `snapshot`,
/// its digests taken in the form `key_check` names.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn set_state(
    tx: &mut PgConnection,
    snapshot: &Snapshot,
    key_check: &[u8],
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE access_care_state SET snapshot = $1::pg_snapshot, key_check = $2, \
         updated_at = now() WHERE singleton",
    )
    .bind(snapshot.text())
    .bind(key_check)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Empty the projection and every EHR row's digest, before a projection under
/// another digest form or from no revision is written whole.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn clear(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM access_care")
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE ehr SET subject_digest = NULL WHERE subject_digest IS NOT NULL")
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Replace the rows of each of `persons` by `rows`, in one transaction.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    persons: &[Uuid],
    rows: &[CareRow],
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM access_care WHERE person_id = ANY($1)")
        .bind(persons)
        .execute(&mut *tx)
        .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let digests: Vec<&[u8]> = rows.iter().map(|row| row.digest.as_slice()).collect();
    let people: Vec<Uuid> = rows.iter().map(|row| row.person).collect();
    let carers: Vec<Uuid> = rows.iter().map(|row| row.carer).collect();
    let kinds: Vec<&str> = rows.iter().map(|row| kind_code(row.carer_kind)).collect();
    let issuers: Vec<&str> = rows
        .iter()
        .map(|row| row.provider_issuer.as_str())
        .collect();
    let ids: Vec<&str> = rows.iter().map(|row| row.provider_id.as_str()).collect();
    let froms: Vec<Option<jiff_sqlx::Date>> = rows
        .iter()
        .map(|row| row.from.map(jiff_sqlx::Date::from))
        .collect();
    let untils: Vec<Option<jiff_sqlx::Date>> = rows
        .iter()
        .map(|row| row.until.map(jiff_sqlx::Date::from))
        .collect();
    let writers: Vec<Option<Uuid>> = rows.iter().map(|row| row.writer).collect();
    let declared: Vec<bool> = rows.iter().map(|row| row.self_declared).collect();
    let role_terminologies: Vec<&str> = rows
        .iter()
        .map(|row| row.role.terminology.as_str())
        .collect();
    let role_codes: Vec<&str> = rows.iter().map(|row| row.role.code.as_str()).collect();
    sqlx::query(
        "INSERT INTO access_care (subject_digest, person_id, carer_id, provider_issuer, \
             provider_id, care_from, care_until, writer_id, self_declared, role_terminology, \
             role_code, carer_kind) \
         SELECT * FROM unnest($1::bytea[], $2::uuid[], $3::uuid[], $4::text[], $5::text[], \
             $6::date[], $7::date[], $8::uuid[], $9::boolean[], $10::text[], $11::text[], \
             $12::\"char\"[])",
    )
    .bind(&digests)
    .bind(&people)
    .bind(&carers)
    .bind(&issuers)
    .bind(&ids)
    .bind(&froms)
    .bind(&untils)
    .bind(&writers)
    .bind(&declared)
    .bind(&role_terminologies)
    .bind(&role_codes)
    .bind(&kinds)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Delete the rows of each of `persons`, the erasure of a `PERSON` removing
/// its care in the flow that erases it.
///
/// # Errors
/// [`StorageError::Acquire`] when no connection is free, and
/// [`StorageError::Database`] on a driver failure.
pub async fn forget(pool: &PgPool, persons: &[Uuid]) -> Result<(), StorageError> {
    if persons.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM access_care WHERE person_id = ANY($1)")
        .bind(persons)
        .execute(&mut *acquire(pool).await?)
        .await?;
    Ok(())
}

/// Up to `limit` EHRs with a subject reference and no digest, each with that
/// reference.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn undigested(
    conn: &mut PgConnection,
    limit: i64,
) -> Result<Vec<(EhrId, EhrSubject)>, StorageError> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, subject_namespace, subject_id FROM ehr \
         WHERE subject_digest IS NULL AND subject_namespace IS NOT NULL \
         AND subject_id IS NOT NULL ORDER BY id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, namespace, subject)| (EhrId(id), EhrSubject::new(namespace, subject)))
        .collect())
}

/// Write each EHR's digest, and answer the number of rows written.
///
/// A row whose reference is no longer the one read keeps the digest the
/// write of the new reference gave it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn set_digests(
    conn: &mut PgConnection,
    digests: &[(EhrId, EhrSubject, Vec<u8>)],
) -> Result<u64, StorageError> {
    let ids: Vec<Uuid> = digests.iter().map(|(ehr, _, _)| ehr.0).collect();
    let namespaces: Vec<&str> = digests
        .iter()
        .map(|(_, subject, _)| subject.namespace.as_str())
        .collect();
    let subjects: Vec<&str> = digests
        .iter()
        .map(|(_, subject, _)| subject.id.as_str())
        .collect();
    let values: Vec<&[u8]> = digests.iter().map(|(_, _, d)| d.as_slice()).collect();
    let written = sqlx::query(
        "UPDATE ehr e SET subject_digest = t.digest \
         FROM unnest($1::uuid[], $2::text[], $3::text[], $4::bytea[]) \
              AS t(id, namespace, subject, digest) \
         WHERE e.id = t.id AND e.subject_namespace = t.namespace \
         AND e.subject_id = t.subject AND e.subject_digest IS NULL",
    )
    .bind(&ids)
    .bind(&namespaces)
    .bind(&subjects)
    .bind(&values)
    .execute(&mut *conn)
    .await?;
    Ok(written.rows_affected())
}
