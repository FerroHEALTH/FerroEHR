// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading and writing `party.national_identifier`.
//!
//! **No openEHR spec governs this — our own design/extension.** The split
//! between the operations is the access-control boundary:
//! [`IdentifierStore::seal`] writes a value, and [`IdentifierStore::open`]
//! reads one back for the audited expansion.
//!
//! Every row belongs to the one party version whose body references it, and
//! is never rewritten, so expanding an older version yields the value that
//! version committed: a versioned repository "is by definition indelible", and
//! every modification is "achieved by physically committing new Versions" (RM
//! common `master06-change_control_package.adoc` §Contributions). The rows of
//! the party's latest version are LIVE (`retired_at IS NULL`); the
//! one-party-per-value rule reads the live rows only.
//!
//! The writes run on the caller's connection, never on the store's pool: a
//! sealed row exists only for a party version that commits, because it is
//! written inside that version's own transaction. The audited reads use the
//! pool.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::service::demographic::identifier::crypto::{CryptoError, DomainKeys};

/// The partial unique index holding one live row per value and scheme.
const VALUE_INDEX: &str = "uq_national_identifier_value";

/// The partial unique index holding one live row per party and scheme.
const PARTY_INDEX: &str = "uq_national_identifier_party";

/// What went wrong holding or reading a protected identifier.
///
/// No variant carries the value: these render into logs and error bodies.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error("the protected-identifier store is unavailable")]
    Database(#[source] sqlx::Error),
    /// No connection to the store could be obtained, so nothing ran.
    #[error("the protected-identifier store is unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// The value could not be sealed or opened.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// The scheme is not in `party.identifier_scheme`.
    ///
    /// A foreign-key refusal read back as its own variant, because "this
    /// deployment does not hold identifiers of that kind" is a configuration
    /// answer rather than a database fault: the configured scheme list and the
    /// registry disagree, which the boot check
    /// ([`IdentifierStore::unregistered`]) exists to catch first.
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
    /// A body references a sealed row that is not this party's, or whose
    /// scheme differs from the `DV_IDENTIFIER.type` beside it.
    ///
    /// Carrying such a reference forward would let this party's expansion
    /// reveal another party's value.
    #[error(
        "the party references a protected identifier it does not hold, or one of another scheme"
    )]
    ForeignReference,
    /// A body that must be stored verbatim carries a protected scheme's value
    /// in plaintext.
    ///
    /// An imported or restored version is a faithful copy that is never
    /// modified, so it cannot be sealed on the way in; storing it would put the
    /// value into the record this deployment keeps free of it.
    #[error(
        "the version carries a `{scheme}` identifier in plaintext and must be stored verbatim, \
         so this deployment cannot seal it"
    )]
    Unsealed {
        /// The scheme code.
        scheme: String,
    },
    /// An archived sealed row is not well-formed.
    #[error("an archived protected-identifier row is malformed")]
    MalformedRow(#[source] base64::DecodeError),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        // 23503 foreign_key_violation on the scheme reference is the one
        // database refusal with a configuration meaning worth separating; the
        // two unique refusals are separated by their constraint at the write.
        let unknown_scheme = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .is_some_and(|code| code == "23503");
        if unknown_scheme {
            return StoreError::UnknownScheme {
                scheme: String::new(),
            };
        }
        StoreError::Database(error)
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

/// The protected-identifier store: audited reads over the demographic pool,
/// writes on the caller's commit transaction.
#[derive(Debug, Clone)]
pub struct IdentifierStore {
    pool: PgPool,
}

impl IdentifierStore {
    /// Bind the store's reads to the demographic pool.
    ///
    /// The pool is the domain: the clinical pool's role has no grant on these
    /// relations, so handing the wrong one in fails loudly at the first
    /// statement rather than reading across the boundary. Writes take the
    /// commit's own connection instead ([`Self::seal`]).
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The first of `schemes` absent from `party.identifier_scheme`, if any.
    ///
    /// The boot check behind the configuration rule that an unregistered
    /// scheme is a boot error rather than a first-write surprise.
    ///
    /// # Errors
    /// [`StoreError::Acquire`] or [`StoreError::Database`] when the registry
    /// cannot be read.
    pub async fn unregistered(&self, schemes: &[String]) -> Result<Option<String>, StoreError> {
        let missing: Option<String> = sqlx::query_scalar(
            "SELECT configured FROM unnest($1::text[]) AS configured \
             WHERE NOT EXISTS (SELECT 1 FROM identifier_scheme WHERE code = configured) \
             LIMIT 1",
        )
        .bind(schemes)
        .fetch_optional(&mut *acquire(&self.pool).await?)
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
    /// [`StoreError::Database`] when the update fails; the caller's
    /// transaction is then aborted and its commit must fail.
    pub async fn retire_live(conn: &mut PgConnection, party: Uuid) -> Result<u64, StoreError> {
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
    /// [`StoreError::Database`] when the delete fails.
    pub async fn erase(conn: &mut PgConnection, parties: &[Uuid]) -> Result<u64, StoreError> {
        let erased = sqlx::query("DELETE FROM national_identifier WHERE party_id = ANY($1)")
            .bind(parties)
            .execute(&mut *conn)
            .await?;
        Ok(erased.rows_affected())
    }

    /// Seal `value` for `party` on `conn` as a new live row and return the row
    /// id the body will reference.
    ///
    /// `conn` is the party domain's commit transaction: the
    /// `national_identifier` relation is created by the party migration set, so
    /// it lives in whichever database serves that domain, beside the party's
    /// own `version` rows. Writing on that transaction makes the seal and the
    /// commit one act, and a rolled-back commit leaves no sealed row. There is
    /// deliberately no pool-backed variant, so no caller can seal outside a
    /// commit. The row is always new and never rewritten, which keeps every
    /// earlier version's value; [`Self::retire_live`] must have run first.
    ///
    /// # Errors
    /// [`StoreError::ValueTaken`] when the value is live for another party,
    /// [`StoreError::SchemeRepeated`] when this party already has a live row in
    /// the scheme, [`StoreError::UnknownScheme`] for an unregistered scheme,
    /// and [`StoreError`] when the value cannot be sealed or the write fails.
    /// A database refusal aborts the caller's transaction, so the caller must
    /// fail its commit.
    pub async fn seal(
        conn: &mut PgConnection,
        keys: &DomainKeys,
        party: Uuid,
        scheme: &str,
        value: &str,
    ) -> Result<Uuid, StoreError> {
        let (nonce, ciphertext) = keys.seal(scheme, value)?;
        let digest = keys.lookup_digest(scheme, value);
        let row = sqlx::query(
            "INSERT INTO national_identifier \
                 (party_id, scheme, nonce, ciphertext, lookup_digest) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id",
        )
        .bind(party)
        .bind(scheme)
        .bind(&nonce)
        .bind(&ciphertext)
        .bind(&digest)
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| refusal_of(error, scheme))?;
        row.try_get::<Uuid, _>("id").map_err(StoreError::Database)
    }

    /// Open the sealed value of a row `party` holds, on `conn`, returning its
    /// scheme and value.
    ///
    /// The write path's read: a new version that carries a reference forward
    /// re-seals the referenced value into a row of its own, and only a row of
    /// this party may be carried. `None` when the row is absent or another
    /// party's. Retired rows qualify, because an earlier version of this party
    /// may be what the client sent back.
    ///
    /// # Errors
    /// [`StoreError`] when the read fails or the record does not authenticate.
    pub async fn open_held(
        conn: &mut PgConnection,
        keys: &DomainKeys,
        party: Uuid,
        row_id: Uuid,
    ) -> Result<Option<(String, String)>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT scheme, nonce, ciphertext FROM national_identifier \
             WHERE id = $1 AND party_id = $2",
        )
        .bind(row_id)
        .bind(party)
        .fetch_optional(&mut *conn)
        .await?
        else {
            return Ok(None);
        };
        let scheme: String = row.try_get("scheme").map_err(StoreError::Database)?;
        let nonce: Vec<u8> = row.try_get("nonce").map_err(StoreError::Database)?;
        let ciphertext: Vec<u8> = row.try_get("ciphertext").map_err(StoreError::Database)?;
        let value = keys.open(&scheme, &nonce, &ciphertext)?;
        Ok(Some((scheme, value)))
    }

    /// Every sealed row of `party`, live and retired, in id order, as an
    /// archive carries it.
    ///
    /// # Errors
    /// [`StoreError::Acquire`] or [`StoreError::Database`] when the read fails.
    pub async fn sealed_rows(&self, party: Uuid) -> Result<Vec<SealedRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, party_id, scheme, nonce, ciphertext, lookup_digest, \
             created_at::text AS created_at, retired_at::text AS retired_at \
             FROM national_identifier WHERE party_id = $1 ORDER BY id",
        )
        .bind(party)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        let mut sealed = Vec::with_capacity(rows.len());
        for row in rows {
            let bytes = |column: &str| -> Result<String, StoreError> {
                let raw: Vec<u8> = row.try_get(column).map_err(StoreError::Database)?;
                Ok(STANDARD.encode(raw))
            };
            sealed.push(SealedRow {
                id: row.try_get("id").map_err(StoreError::Database)?,
                party_id: row.try_get("party_id").map_err(StoreError::Database)?,
                scheme: row.try_get("scheme").map_err(StoreError::Database)?,
                nonce: bytes("nonce")?,
                ciphertext: bytes("ciphertext")?,
                lookup_digest: bytes("lookup_digest")?,
                created_at: row.try_get("created_at").map_err(StoreError::Database)?,
                retired_at: row.try_get("retired_at").map_err(StoreError::Database)?,
            });
        }
        Ok(sealed)
    }

    /// Restore archived sealed rows verbatim on `conn`, the load's
    /// transaction, ids and states included.
    ///
    /// Nothing is decrypted: the rows open again only under the deployment key
    /// they were sealed with.
    ///
    /// # Errors
    /// [`StoreError::MalformedRow`] for a row whose base64 does not decode,
    /// [`StoreError::ValueTaken`] when a live value is live for another party
    /// here, [`StoreError::UnknownScheme`] for an unregistered scheme, and
    /// [`StoreError::Database`] for any other refusal.
    pub async fn restore_sealed(
        conn: &mut PgConnection,
        rows: &[SealedRow],
    ) -> Result<(), StoreError> {
        for row in rows {
            let decode = |text: &str| STANDARD.decode(text).map_err(StoreError::MalformedRow);
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

    /// Open the sealed value of one row.
    ///
    /// The audited expansion path. Retired rows open like live ones: each is
    /// the value an earlier version committed. `None` when no such row exists,
    /// which is a body referencing a row that was removed — reported as absent
    /// rather than as a decryption failure, because the two have different
    /// causes.
    ///
    /// # Errors
    /// [`StoreError`] when the read fails or the record does not authenticate.
    pub async fn open(
        &self,
        keys: &DomainKeys,
        row_id: Uuid,
    ) -> Result<Option<String>, StoreError> {
        let Some(row) =
            sqlx::query("SELECT scheme, nonce, ciphertext FROM national_identifier WHERE id = $1")
                .bind(row_id)
                .fetch_optional(&mut *acquire(&self.pool).await?)
                .await?
        else {
            return Ok(None);
        };
        let scheme: String = row.try_get("scheme").map_err(StoreError::Database)?;
        let nonce: Vec<u8> = row.try_get("nonce").map_err(StoreError::Database)?;
        let ciphertext: Vec<u8> = row.try_get("ciphertext").map_err(StoreError::Database)?;
        Ok(Some(keys.open(&scheme, &nonce, &ciphertext)?))
    }
}

/// The typed refusal of a failed write of a `scheme` row: the scheme
/// foreign key and the two live-row unique indexes each name their own answer.
fn refusal_of(error: sqlx::Error, scheme: &str) -> StoreError {
    let constraint = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::constraint)
        .map(str::to_owned);
    match (constraint.as_deref(), StoreError::from(error)) {
        (_, StoreError::UnknownScheme { .. }) => StoreError::UnknownScheme {
            scheme: scheme.to_owned(),
        },
        (Some(VALUE_INDEX), _) => StoreError::ValueTaken {
            scheme: scheme.to_owned(),
        },
        (Some(PARTY_INDEX), _) => StoreError::SchemeRepeated {
            scheme: scheme.to_owned(),
        },
        (_, other) => other,
    }
}
