// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `party.national_identifier`, over sealed bytes only.
//!
//! **No openEHR spec governs this — our own design/extension.** Every
//! statement here takes or returns ciphertext, nonce and keyed digest; the
//! plaintext value never reaches this module, because sealing and opening
//! stay with the key holder in the service.
//!
//! Every row belongs to the one party version whose body references it, and
//! is never rewritten, so expanding an older version yields the value that
//! version committed: a versioned repository "is by definition indelible", and
//! every modification is "achieved by physically committing new Versions" (RM
//! common `master06-change_control_package.adoc` §Contributions). The rows of
//! the party's latest version are LIVE (`retired_at IS NULL`); the
//! one-party-per-value rule reads the live rows only.
//!
//! The writes run on the caller's connection: a sealed row exists only for a
//! party version that commits, because it is written inside that version's own
//! transaction.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::VoId;

/// The partial unique index holding one live row per value and scheme.
const VALUE_INDEX: &str = "uq_national_identifier_value";

/// The partial unique index holding one live row per party and scheme.
const PARTY_INDEX: &str = "uq_national_identifier_party";

/// What went wrong reading or writing a sealed identifier row.
///
/// No variant carries the value: these render into logs and error bodies.
#[derive(Debug, thiserror::Error)]
pub enum IdentifierRepoError {
    /// The database refused or failed.
    #[error("the protected-identifier store is unavailable")]
    Database(#[source] sqlx::Error),
    /// No connection to the store could be obtained, so nothing ran.
    #[error("the protected-identifier store is unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// The scheme is not in `party.identifier_scheme`.
    ///
    /// A foreign-key refusal read back as its own variant, because "this
    /// deployment does not hold identifiers of that kind" is a configuration
    /// answer rather than a database fault: the configured scheme list and the
    /// registry disagree, which the boot check ([`unregistered`]) exists to
    /// catch first.
    #[error("`{scheme}` is not a registered identifier scheme in this deployment")]
    UnknownScheme {
        /// The scheme code as configured.
        scheme: String,
    },
    /// The value is live for another party in this scheme.
    ///
    /// One value identifies one party: a commit giving a second party a value
    /// another party's latest version holds is a conflict with stored state,
    /// not a database fault.
    #[error("the `{scheme}` identifier is already held by another party")]
    ValueTaken {
        /// The scheme code.
        scheme: String,
    },
    /// One body carries two different values in one scheme.
    ///
    /// A party holds one value per scheme; two would leave resolution unable
    /// to say which is the party's.
    #[error("the party carries more than one `{scheme}` identifier")]
    SchemeRepeated {
        /// The scheme code.
        scheme: String,
    },
    /// An archived sealed row is not well-formed.
    #[error("an archived protected-identifier row is malformed")]
    MalformedRow(#[source] base64::DecodeError),
}

impl From<sqlx::Error> for IdentifierRepoError {
    fn from(error: sqlx::Error) -> Self {
        // 23503 foreign_key_violation on the scheme reference is the one
        // database refusal with a configuration meaning worth separating; the
        // two unique refusals are separated by their constraint at the write.
        let unknown_scheme = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .is_some_and(|code| code == "23503");
        if unknown_scheme {
            return IdentifierRepoError::UnknownScheme {
                scheme: String::new(),
            };
        }
        IdentifierRepoError::Database(error)
    }
}

/// One identifier sealed by the key holder, ready to be written.
#[derive(Clone, Copy)]
pub struct NewSealed<'a> {
    /// The registered scheme.
    pub scheme: &'a str,
    /// The AES-256-GCM nonce.
    pub nonce: &'a [u8],
    /// The sealed value.
    pub ciphertext: &'a [u8],
    /// The keyed lookup digest.
    pub lookup_digest: &'a [u8],
}

impl std::fmt::Debug for NewSealed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewSealed")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

/// The sealed half of a stored row: what the key holder opens.
pub struct StoredSealed {
    /// The registered scheme the row was sealed under.
    pub scheme: String,
    /// The AES-256-GCM nonce.
    pub nonce: Vec<u8>,
    /// The sealed value.
    pub ciphertext: Vec<u8>,
}

impl std::fmt::Debug for StoredSealed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSealed")
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

/// One sealed row as an archive carries it: ciphertext, nonce and keyed digest
/// in base64, never the value.
///
/// The dump/load round trip moves these rows beside the party versions that
/// reference them, so a restored version expands to what it committed under
/// the same deployment key. `Debug` prints the row identity only: the digest
/// matches a known value, which the party reader is not granted either.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SealedRow {
    /// The row id the versions reference.
    pub id: Uuid,
    /// The party the row belongs to.
    pub party_id: Uuid,
    /// The registered scheme.
    pub scheme: String,
    /// The AES-256-GCM nonce, base64.
    pub nonce: String,
    /// The sealed value, base64.
    pub ciphertext: String,
    /// The keyed lookup digest, base64.
    pub lookup_digest: String,
    /// When the row was sealed, as PostgreSQL renders a `timestamptz`.
    pub created_at: String,
    /// When a later version superseded it, if one has.
    pub retired_at: Option<String>,
}

impl std::fmt::Debug for SealedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedRow")
            .field("id", &self.id)
            .field("party_id", &self.party_id)
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

/// The first of `schemes` absent from `party.identifier_scheme`, if any, read
/// on `pool`, the party domain's.
///
/// The boot check behind the configuration rule that an unregistered scheme
/// is a boot error rather than a first-write surprise.
///
/// # Errors
/// [`IdentifierRepoError::Acquire`] or [`IdentifierRepoError::Database`] when
/// the registry cannot be read.
pub async fn unregistered(
    pool: &PgPool,
    schemes: &[String],
) -> Result<Option<String>, IdentifierRepoError> {
    let missing: Option<String> = sqlx::query_scalar(
        "SELECT configured FROM unnest($1::text[]) AS configured \
         WHERE NOT EXISTS (SELECT 1 FROM identifier_scheme WHERE code = configured) \
         LIMIT 1",
    )
    .bind(schemes)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    Ok(missing)
}

/// Retire every live row of `party` on `conn`, the party commit's
/// transaction, and return how many were retired.
///
/// Runs once per party commit before the new version's rows are sealed, so
/// the live rows are always exactly the latest version's. A retired row
/// stays: the version that references it still expands to its value.
///
/// # Errors
/// [`IdentifierRepoError::Database`] when the update fails; the caller's
/// transaction is then aborted and its commit must fail.
pub async fn retire_live(conn: &mut PgConnection, party: VoId) -> Result<u64, IdentifierRepoError> {
    let retired = sqlx::query(
        "UPDATE national_identifier SET retired_at = now() \
         WHERE party_id = $1 AND retired_at IS NULL",
    )
    .bind(party)
    .execute(&mut *conn)
    .await?;
    Ok(retired.rows_affected())
}

/// Remove every sealed row, live and retired, of `parties` on `conn`, the
/// physical erasure's transaction, and return how many were removed.
///
/// The erasure of a party removes the values its versions referenced with
/// it; a row left behind would keep the identifier after the record that
/// held it is gone.
///
/// # Errors
/// [`IdentifierRepoError::Database`] when the delete fails.
pub async fn erase(conn: &mut PgConnection, parties: &[VoId]) -> Result<u64, IdentifierRepoError> {
    let erased = sqlx::query("DELETE FROM national_identifier WHERE party_id = ANY($1)")
        .bind(parties)
        .execute(&mut *conn)
        .await?;
    Ok(erased.rows_affected())
}

/// Write `sealed` for `party` on `conn` as a new live row and return the row
/// id the body will reference.
///
/// `conn` is the party domain's commit transaction: the
/// `national_identifier` relation is created by the party migration set, so
/// it lives in whichever database serves that domain, beside the party's own
/// `version` rows. There is deliberately no pool-backed variant, so no caller
/// can write a row outside a commit. The row is always new and never
/// rewritten, which keeps every earlier version's value; [`retire_live`] must
/// have run first.
///
/// # Errors
/// [`IdentifierRepoError::ValueTaken`] when the value is live for another
/// party, [`IdentifierRepoError::SchemeRepeated`] when this party already has
/// a live row in the scheme, [`IdentifierRepoError::UnknownScheme`] for an
/// unregistered scheme, and [`IdentifierRepoError::Database`] for any other
/// refusal. A database refusal aborts the caller's transaction, so the caller
/// must fail its commit.
pub async fn insert_sealed(
    conn: &mut PgConnection,
    party: VoId,
    sealed: NewSealed<'_>,
) -> Result<Uuid, IdentifierRepoError> {
    let row = sqlx::query(
        "INSERT INTO national_identifier \
             (party_id, scheme, nonce, ciphertext, lookup_digest) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id",
    )
    .bind(party)
    .bind(sealed.scheme)
    .bind(sealed.nonce)
    .bind(sealed.ciphertext)
    .bind(sealed.lookup_digest)
    .fetch_one(&mut *conn)
    .await
    .map_err(|error| refusal_of(error, sealed.scheme))?;
    row.try_get::<Uuid, _>("id")
        .map_err(IdentifierRepoError::Database)
}

/// The sealed half of row `row_id` on `conn`, restricted to the rows `holder`
/// holds when one is named; `None` when no such row exists.
///
/// Retired rows qualify like live ones: each is the value an earlier version
/// committed.
///
/// # Errors
/// [`IdentifierRepoError::Database`] when the read fails.
pub async fn read_sealed(
    conn: &mut PgConnection,
    row_id: Uuid,
    holder: Option<VoId>,
) -> Result<Option<StoredSealed>, IdentifierRepoError> {
    let Some(row) = sqlx::query(
        "SELECT scheme, nonce, ciphertext FROM national_identifier \
         WHERE id = $1 AND ($2::uuid IS NULL OR party_id = $2)",
    )
    .bind(row_id)
    .bind(holder)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(StoredSealed {
        scheme: row
            .try_get("scheme")
            .map_err(IdentifierRepoError::Database)?,
        nonce: row
            .try_get("nonce")
            .map_err(IdentifierRepoError::Database)?,
        ciphertext: row
            .try_get("ciphertext")
            .map_err(IdentifierRepoError::Database)?,
    }))
}

/// Every sealed row of `party`, live and retired, in id order, as an archive
/// carries it, read on `pool`, the party domain's.
///
/// # Errors
/// [`IdentifierRepoError::Acquire`] or [`IdentifierRepoError::Database`] when
/// the read fails.
pub async fn sealed_rows(
    pool: &PgPool,
    party: VoId,
) -> Result<Vec<SealedRow>, IdentifierRepoError> {
    let rows = sqlx::query(
        "SELECT id, party_id, scheme, nonce, ciphertext, lookup_digest, \
         created_at::text AS created_at, retired_at::text AS retired_at \
         FROM national_identifier WHERE party_id = $1 ORDER BY id",
    )
    .bind(party)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    let mut sealed = Vec::with_capacity(rows.len());
    for row in rows {
        let bytes = |column: &str| -> Result<String, IdentifierRepoError> {
            let raw: Vec<u8> = row.try_get(column).map_err(IdentifierRepoError::Database)?;
            Ok(STANDARD.encode(raw))
        };
        sealed.push(SealedRow {
            id: row.try_get("id").map_err(IdentifierRepoError::Database)?,
            party_id: row
                .try_get("party_id")
                .map_err(IdentifierRepoError::Database)?,
            scheme: row
                .try_get("scheme")
                .map_err(IdentifierRepoError::Database)?,
            nonce: bytes("nonce")?,
            ciphertext: bytes("ciphertext")?,
            lookup_digest: bytes("lookup_digest")?,
            created_at: row
                .try_get("created_at")
                .map_err(IdentifierRepoError::Database)?,
            retired_at: row
                .try_get("retired_at")
                .map_err(IdentifierRepoError::Database)?,
        });
    }
    Ok(sealed)
}

/// Restore archived sealed rows verbatim on `conn`, the load's transaction,
/// ids and states included.
///
/// Nothing is decrypted: the rows open again only under the deployment key
/// they were sealed with.
///
/// # Errors
/// [`IdentifierRepoError::MalformedRow`] for a row whose base64 does not
/// decode, [`IdentifierRepoError::ValueTaken`] when a live value is live for
/// another party here, [`IdentifierRepoError::UnknownScheme`] for an
/// unregistered scheme, and [`IdentifierRepoError::Database`] for any other
/// refusal.
pub async fn restore_sealed(
    conn: &mut PgConnection,
    rows: &[SealedRow],
) -> Result<(), IdentifierRepoError> {
    for row in rows {
        let decode = |text: &str| {
            STANDARD
                .decode(text)
                .map_err(IdentifierRepoError::MalformedRow)
        };
        sqlx::query(
            "INSERT INTO national_identifier \
                 (id, party_id, scheme, nonce, ciphertext, lookup_digest, created_at, \
                  retired_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz, $8::timestamptz)",
        )
        .bind(row.id)
        .bind(row.party_id)
        .bind(&row.scheme)
        .bind(decode(&row.nonce)?)
        .bind(decode(&row.ciphertext)?)
        .bind(decode(&row.lookup_digest)?)
        .bind(&row.created_at)
        .bind(row.retired_at.as_deref())
        .execute(&mut *conn)
        .await
        .map_err(|error| refusal_of(error, &row.scheme))?;
    }
    Ok(())
}

/// The typed refusal of a failed write of a `scheme` row: the scheme
/// foreign key and the two live-row unique indexes each name their own answer.
fn refusal_of(error: sqlx::Error, scheme: &str) -> IdentifierRepoError {
    let constraint = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::constraint)
        .map(str::to_owned);
    match (constraint.as_deref(), IdentifierRepoError::from(error)) {
        (_, IdentifierRepoError::UnknownScheme { .. }) => IdentifierRepoError::UnknownScheme {
            scheme: scheme.to_owned(),
        },
        (Some(VALUE_INDEX), _) => IdentifierRepoError::ValueTaken {
            scheme: scheme.to_owned(),
        },
        (Some(PARTY_INDEX), _) => IdentifierRepoError::SchemeRepeated {
            scheme: scheme.to_owned(),
        },
        (_, other) => other,
    }
}
