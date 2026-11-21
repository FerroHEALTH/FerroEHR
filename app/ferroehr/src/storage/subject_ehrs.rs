// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHRs a set of subject references names, and the markings of their
//! settings.
//!
//! A care relationship resource reads them to decide each subject it reveals
//! by that subject's record (#6076), and its write to judge its writer at
//! the subject's confidentiality (#6141). No openEHR spec governs it: our own
//! design.

use sqlx::PgPool;

use crate::db::acquire::acquire;
use crate::ids::EhrId;
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

/// The EHRs whose subject is one of `subjects`, each with the reference that
/// named it, in EHR order, read in one statement on `uq_ehr_subject`. An
/// empty `subjects` makes no round trip.
///
/// # Errors
/// [`StorageError`] when no connection is free or the read fails.
pub async fn ehrs_of_subjects(
    pool: &PgPool,
    subjects: &[EhrSubject],
) -> Result<Vec<(EhrSubject, EhrId)>, StorageError> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let (namespaces, ids): (Vec<&str>, Vec<&str>) = subjects
        .iter()
        .map(|subject| (subject.namespace.as_str(), subject.id.as_str()))
        .unzip();
    let rows: Vec<(String, String, EhrId)> = sqlx::query_as(
        "SELECT e.subject_namespace, e.subject_id, e.id \
         FROM unnest($1::text[], $2::text[]) AS s(namespace, id) \
         JOIN ehr e ON e.subject_id = s.id AND e.subject_namespace = s.namespace \
         ORDER BY e.id",
    )
    .bind(&namespaces)
    .bind(&ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(namespace, id, ehr)| (EhrSubject::new(namespace, id), ehr))
        .collect())
}

/// The `marking` of the settings of every EHR whose subject is one of
/// `subjects`, as stored JSON text.
///
/// One statement reads them on `uq_ehr_subject` and the settings' primary
/// key; an EHR whose settings carry no marking yields none, and an empty
/// `subjects` makes no round trip.
///
/// # Errors
/// [`StorageError`] when no connection is free or the read fails.
pub async fn markings_of_subjects(
    pool: &PgPool,
    subjects: &[EhrSubject],
) -> Result<Vec<String>, StorageError> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let (namespaces, ids): (Vec<&str>, Vec<&str>) = subjects
        .iter()
        .map(|subject| (subject.namespace.as_str(), subject.id.as_str()))
        .unzip();
    Ok(sqlx::query_scalar(
        "SELECT (s.settings -> 'marking')::text \
         FROM unnest($1::text[], $2::text[]) AS r(namespace, id) \
         JOIN ehr e ON e.subject_id = r.id AND e.subject_namespace = r.namespace \
         JOIN access_settings s ON s.ehr_id = e.id \
         WHERE s.settings ? 'marking' \
         ORDER BY e.id",
    )
    .bind(&namespaces)
    .bind(&ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}
