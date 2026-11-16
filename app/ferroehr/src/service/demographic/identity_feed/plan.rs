// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the store holds about the identities a PMIR message names: the
//! `PERSON` each is bound to, the merge it stands in, and the patient
//! identifiers its `PERSON` carries that the Registry no longer announces.
//!
//! Receipt reads these to decide what stops resolving at once, and the apply
//! reads them to decide what moves. An identity is bound to the one `PERSON`
//! that carries one of its announced identifiers; none is no binding yet, and
//! more than one is refused. No specification governs the binding: our own
//! design.

use std::collections::BTreeSet;

use sqlx::PgConnection;
use uuid::Uuid;

use super::body::SubjectRef;
use super::{Identifier, Refusal};
use crate::identifier_protection::engine::IdentifierProtection;
use crate::party_binding::grants::caller::patient::{self, CarriedRow};
use crate::storage::error::StorageError;

/// What the store holds about one identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Identity {
    /// The `PERSON` it is bound to.
    pub(super) person: Option<Uuid>,
    /// The identity it is merged into.
    pub(super) merged_into: Option<String>,
}

/// The identity `pix_id`, read on `conn`; `None` when the feed never named
/// it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(super) async fn identity(
    conn: &mut PgConnection,
    pix_id: &str,
) -> Result<Option<Identity>, StorageError> {
    let row: Option<(Option<Uuid>, Option<String>)> =
        sqlx::query_as("SELECT person_id, merged_into FROM pmir_identity WHERE pix_id = $1")
            .bind(pix_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(person, merged_into)| Identity {
        person,
        merged_into,
    }))
}

/// The identity bound to the `PERSON` `person`, read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(super) async fn identity_of(
    conn: &mut PgConnection,
    person: Uuid,
) -> Result<Option<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT pix_id FROM pmir_identity WHERE person_id = $1 LIMIT 1")
            .bind(person)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

/// The lookup digests of `value` in each protected scheme; none without
/// identifier protection.
pub(super) fn digests(
    engine: Option<&IdentifierProtection>,
    value: &str,
) -> Vec<(String, Vec<u8>)> {
    engine
        .map(|engine| engine.lookup_digests(value))
        .unwrap_or_default()
}

/// The rows of every `PERSON` that carries `identifier`, split or not.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(super) async fn carried(
    conn: &mut PgConnection,
    engine: Option<&IdentifierProtection>,
    identifier: &Identifier,
) -> Result<Vec<CarriedRow>, StorageError> {
    patient::carried(
        conn,
        &identifier.system,
        &identifier.value,
        &digests(engine, &identifier.value),
    )
    .await
}

/// The `PERSON` the identity `known` with `identifiers` is bound to: its
/// binding, or else the one `PERSON` other than `exclude` that carries one
/// of `identifiers`.
///
/// # Errors
/// [`Refusal::Ambiguous`] when more than one does; a storage failure as
/// [`Resolved::Storage`].
pub(super) async fn resolve(
    conn: &mut PgConnection,
    engine: Option<&IdentifierProtection>,
    known: Option<&Identity>,
    identifiers: &[Identifier],
    exclude: Option<Uuid>,
) -> Result<Option<Uuid>, Resolved> {
    if let Some(person) = known.and_then(|identity| identity.person) {
        return Ok(Some(person));
    }
    let mut holders = BTreeSet::new();
    for identifier in identifiers {
        for row in carried(conn, engine, identifier).await? {
            if Some(row.holder) != exclude {
                holders.insert(row.holder);
            }
        }
    }
    let mut holders = holders.into_iter();
    match (holders.next(), holders.next()) {
        (Some(_), Some(_)) => Err(Resolved::Refused(Refusal::Ambiguous)),
        (one, _) => Ok(one),
    }
}

/// Why a binding could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum Resolved {
    /// The store did not answer.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The message cannot be applied.
    #[error(transparent)]
    Refused(Refusal),
}

/// One patient identifier row of a `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Row {
    /// The issuer.
    pub(super) issuer: String,
    /// The id as stored.
    pub(super) value: String,
    /// The `DV_IDENTIFIER` type, empty when it names none.
    pub(super) identifier_type: String,
}

/// The patient identifier rows of `person` that match none of `announced`,
/// by value or, for a protected scheme, by its lookup digest; every row for
/// an empty `announced`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(super) async fn unannounced(
    conn: &mut PgConnection,
    engine: Option<&IdentifierProtection>,
    person: Uuid,
    announced: &[Identifier],
) -> Result<Vec<Row>, StorageError> {
    let issuers: Vec<&str> = announced.iter().map(|i| i.system.as_str()).collect();
    let values: Vec<&str> = announced.iter().map(|i| i.value.as_str()).collect();
    let mut digest_issuers = Vec::new();
    let mut schemes = Vec::new();
    let mut digest_values = Vec::new();
    for identifier in announced {
        for (scheme, digest) in digests(engine, &identifier.value) {
            digest_issuers.push(identifier.system.as_str());
            schemes.push(scheme);
            digest_values.push(digest);
        }
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT b.issuer, b.value, b.identifier_type FROM binding_patient_identifier b \
         WHERE b.holder_id = $1 \
           AND NOT EXISTS (SELECT 1 FROM unnest($2::text[], $3::text[]) AS a(issuer, value) \
             WHERE a.issuer = b.issuer AND a.value = b.value) \
           AND NOT EXISTS (SELECT 1 \
             FROM unnest($4::text[], $5::text[], $6::bytea[]) AS d(issuer, scheme, digest) \
             JOIN national_identifier n ON n.scheme = d.scheme AND n.lookup_digest = d.digest \
               AND n.retired_at IS NULL \
             WHERE n.party_id = b.holder_id AND d.issuer = b.issuer \
               AND b.identifier_type = n.scheme) \
         ORDER BY b.ordinal",
    )
    .bind(person)
    .bind(&issuers)
    .bind(&values)
    .bind(&digest_issuers)
    .bind(&schemes)
    .bind(&digest_values)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(issuer, value, identifier_type)| Row {
            issuer,
            value,
            identifier_type,
        })
        .collect())
}

/// The merge of `merged_pix` that stands unreverted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StandingMerge {
    /// Its id.
    pub(super) id: Uuid,
    /// The identity it merged into.
    pub(super) survivor_pix: String,
    /// The merged identity's `PERSON`.
    pub(super) merged_person: Option<Uuid>,
    /// The survivor's `PERSON`.
    pub(super) survivor_person: Option<Uuid>,
    /// The record subjects it moved.
    pub(super) subjects: Vec<SubjectRef>,
}

/// A `pmir_merge` row as [`standing_merge`] reads it.
type MergeRow = (
    Uuid,
    String,
    Option<Uuid>,
    Option<Uuid>,
    sqlx::types::Json<Vec<SubjectRef>>,
);

/// The merge of `merged_pix` that stands unreverted, read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, or a stored subject list
/// that does not read.
pub(super) async fn standing_merge(
    conn: &mut PgConnection,
    merged_pix: &str,
) -> Result<Option<StandingMerge>, StorageError> {
    let row: Option<MergeRow> = sqlx::query_as(
        "SELECT id, survivor_pix, merged_person, survivor_person, subjects FROM pmir_merge \
             WHERE merged_pix = $1 AND reverted_by IS NULL",
    )
    .bind(merged_pix)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(
        |(id, survivor_pix, merged_person, survivor_person, subjects)| StandingMerge {
            id,
            survivor_pix,
            merged_person,
            survivor_person,
            subjects: subjects.0,
        },
    ))
}

/// The one `DV_IDENTIFIER.type` the patient `PERSON`s carry identifiers of
/// `issuer` with, so a linked identifier is written as its domain's others
/// are and a protected scheme stays protected; `None` when they carry none
/// or more than one.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(super) async fn domain_type(
    conn: &mut PgConnection,
    issuer: &str,
) -> Result<Option<String>, StorageError> {
    let types: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT identifier_type FROM binding_patient_identifier WHERE issuer = $1 \
         ORDER BY 1 LIMIT 2",
    )
    .bind(issuer)
    .fetch_all(&mut *conn)
    .await?;
    Ok(match types.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    })
}
