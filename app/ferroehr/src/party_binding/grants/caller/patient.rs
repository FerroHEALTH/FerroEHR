// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient identifiers: the identifiers a patient's `PERSON` carries at
//! the `[authz.party_binding.patient]` identifier path, by issuer.
//!
//! They find the `PERSON` a contact point's treatment relationship
//! confirmation names by an identifier of another domain (eHDSI XCA Profile
//! 8.0.1 §4.2.1), and through its record subjects ([`super::subject`]) its
//! EHRs, in the party domain, so admitting the request asks no identity
//! source (#5648). An identifier issued by another organisation or the state
//! sits in `PARTY.details` (RM Demographic IM §2.1.3), and its issuer is its
//! domain, so no country-specific rule reads it.
//!
//! The rows are projected into `binding_patient_identifier` in each commit's
//! transaction beside the record subjects. A protected national identifier is
//! held as the reference the stored body carries and found through the
//! lookup digest of `national_identifier`. The projection and the lookup are
//! governed by no openEHR spec: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::party_binding::grants::store::GrantHead;
use crate::storage::error::StorageError;

/// The patient identifier rows of the identifier `$2` under the issuer
/// `$1`, by value or, for a protected scheme, by one of the schemes `$3`
/// with the lookup digest `$4` of the value: each holder, the issuer and the
/// id as the stored body carries it.
macro_rules! carried_rows {
    () => {
        "SELECT b.holder_id, b.issuer, b.value, b.identifier_type \
         FROM binding_patient_identifier b \
         WHERE b.issuer = $1 AND b.value = $2 AND b.lifecycle_state IN ('532', '553') \
         UNION \
         SELECT b.holder_id, b.issuer, b.value, b.identifier_type \
         FROM unnest($3::text[], $4::bytea[]) AS d(scheme, digest) \
         JOIN national_identifier n \
           ON n.scheme = d.scheme AND n.lookup_digest = d.digest AND n.retired_at IS NULL \
         JOIN binding_patient_identifier b \
           ON b.holder_id = n.party_id AND b.issuer = $1 AND b.identifier_type = n.scheme \
          AND b.lifecycle_state IN ('532', '553')"
    };
}

/// The patient `PERSON`s that carry the identifier `value` under `issuer`
/// while their head is `complete` or `incomplete`: by the value as written,
/// or, for an identifier of a protected scheme, by one of `protected`, each
/// a scheme with the lookup digest of `value` in it, unless a received PMIR
/// message has split the identifier off the `PERSON`. Both arms are indexed
/// reads: `ix_binding_patient_identifier_value`, and
/// `uq_national_identifier_value` with the primary key.
const CARRYING: &str = concat!(
    "SELECT DISTINCT c.holder_id FROM (",
    carried_rows!(),
    ") c WHERE NOT EXISTS (SELECT 1 FROM pmir_suppression s WHERE s.kind = 'identifier' \
       AND s.holder_id = c.holder_id AND s.namespace = c.issuer AND s.value = c.value) \
     ORDER BY 1"
);

/// The rows of every `PERSON` that carries the identifier, a received PMIR
/// message's split or not, sorted: what applying a message moves.
const CARRIED: &str = concat!("SELECT * FROM (", carried_rows!(), ") c ORDER BY 1, 3");

/// One patient identifier row a `PERSON` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CarriedRow {
    /// The `PERSON` container.
    pub(crate) holder: Uuid,
    /// The `DV_IDENTIFIER` issuer.
    pub(crate) issuer: String,
    /// The id as the stored body carries it: the value, or a protected
    /// scheme's reference.
    pub(crate) value: String,
    /// The `DV_IDENTIFIER` type, empty when it names none.
    pub(crate) identifier_type: String,
}

/// Replaces the patient identifiers of the `PERSON` `head` with the ones its
/// live body carries, on its commit transaction `tx`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn replace(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_patient_identifier WHERE holder_id = $1")
        .bind(head.vo_id)
        .execute(&mut *tx)
        .await?;
    let carried = head
        .live()
        .map_or(&[][..], |facts| facts.patient_identifiers.as_slice());
    if carried.is_empty() {
        return Ok(());
    }
    let ordinal: Vec<i32> = (0..carried.len())
        .map(|at| i32::try_from(at).unwrap_or(i32::MAX))
        .collect();
    let issuer: Vec<&str> = carried.iter().map(|i| i.issuer.as_str()).collect();
    let id_type: Vec<&str> = carried.iter().map(|i| i.id_type.as_str()).collect();
    let value: Vec<&str> = carried.iter().map(|i| i.id.as_str()).collect();
    sqlx::query(
        "INSERT INTO binding_patient_identifier \
             (holder_id, ordinal, issuer, identifier_type, value, lifecycle_state) \
         SELECT $1, t.ordinal, t.issuer, t.identifier_type, t.value, $2 \
         FROM unnest($3::integer[], $4::text[], $5::text[], $6::text[]) \
              AS t(ordinal, issuer, identifier_type, value)",
    )
    .bind(head.vo_id)
    .bind(head.lifecycle_state)
    .bind(&ordinal)
    .bind(&issuer)
    .bind(&id_type)
    .bind(&value)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The patient `PERSON`s that carry the identifier `value` under `issuer`,
/// read on `conn`, sorted: by the value as written, or by one of
/// `protected`, each a protected scheme with the lookup digest of `value` in
/// it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn carrying(
    conn: &mut PgConnection,
    issuer: &str,
    value: &str,
    protected: &[(String, Vec<u8>)],
) -> Result<Vec<Uuid>, StorageError> {
    let (schemes, digests) = split(protected);
    Ok(sqlx::query_scalar(CARRYING)
        .bind(issuer)
        .bind(value)
        .bind(&schemes)
        .bind(&digests)
        .fetch_all(&mut *conn)
        .await?)
}

/// The rows of every patient `PERSON` that carries the identifier `value`
/// under `issuer`, by value or by one of `protected`, whether or not a
/// received PMIR message has split it off, read on `conn`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn carried(
    conn: &mut PgConnection,
    issuer: &str,
    value: &str,
    protected: &[(String, Vec<u8>)],
) -> Result<Vec<CarriedRow>, StorageError> {
    let (schemes, digests) = split(protected);
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(CARRIED)
        .bind(issuer)
        .bind(value)
        .bind(&schemes)
        .bind(&digests)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(holder, issuer, value, identifier_type)| CarriedRow {
            holder,
            issuer,
            value,
            identifier_type,
        })
        .collect())
}

/// The schemes and the lookup digests of `protected`, as two bind arrays.
fn split(protected: &[(String, Vec<u8>)]) -> (Vec<&str>, Vec<&[u8]>) {
    protected
        .iter()
        .map(|(scheme, digest)| (scheme.as_str(), digest.as_slice()))
        .unzip()
}

/// Deletes the patient identifiers of every `PERSON` of `vo_ids`, on the
/// erasure transaction `tx`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn erase(tx: &mut PgConnection, vo_ids: &[Uuid]) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM binding_patient_identifier WHERE holder_id = ANY($1)")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
