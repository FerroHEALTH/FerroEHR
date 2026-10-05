// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading and writing `demographic.national_identifier`.
//!
//! **No openEHR spec governs this — our own design/extension.** Three
//! operations, and the split between them is the access-control boundary:
//! [`IdentifierStore::seal`] writes a value, [`IdentifierStore::open`] reads
//! one back for the audited expansion, and [`IdentifierStore::resolve`] goes
//! from an identifier to a party WITHOUT decrypting anything, through the
//! `SECURITY DEFINER` function, so a caller that only needs the mapping never
//! touches the ciphertext.

use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::service::demographic::identifier::crypto::{CryptoError, DomainKeys};

/// What went wrong holding or reading a protected identifier.
///
/// No variant carries the value: these render into logs and error bodies.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error("the protected-identifier store is unavailable")]
    Database(#[source] sqlx::Error),
    /// The value could not be sealed or opened.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// The scheme is not in `demographic.identifier_scheme`.
    ///
    /// A foreign-key refusal read back as its own variant, because "this
    /// deployment does not hold identifiers of that kind" is a configuration
    /// answer rather than a database fault.
    #[error("`{scheme}` is not a registered identifier scheme in this deployment")]
    UnknownScheme {
        /// The scheme code as configured.
        scheme: String,
    },
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        // 23503 foreign_key_violation on the scheme reference is the one
        // database refusal with a configuration meaning worth separating.
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

/// The protected-identifier store over the demographic pool.
#[derive(Debug, Clone)]
pub struct IdentifierStore {
    pool: PgPool,
}

impl IdentifierStore {
    /// Bind the store to the demographic pool.
    ///
    /// The pool is the domain: the clinical pool's role has no grant on these
    /// relations, so handing the wrong one in fails loudly at the first
    /// statement rather than reading across the boundary.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Seal `value` for `party` and return the row id the body will reference.
    ///
    /// Re-sealing the same party and scheme replaces the row: a party's
    /// identifier is a fact about that party, and a second row would leave the
    /// store unable to say which is current.
    ///
    /// # Errors
    /// [`StoreError`] when the scheme is unregistered, the value cannot be
    /// sealed, or the write fails.
    pub async fn seal(
        &self,
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
             ON CONFLICT (scheme, party_id) DO UPDATE \
                 SET nonce = EXCLUDED.nonce, \
                     ciphertext = EXCLUDED.ciphertext, \
                     lookup_digest = EXCLUDED.lookup_digest \
             RETURNING id",
        )
        .bind(party)
        .bind(scheme)
        .bind(&nonce)
        .bind(&ciphertext)
        .bind(&digest)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| match StoreError::from(error) {
            StoreError::UnknownScheme { .. } => StoreError::UnknownScheme {
                scheme: scheme.to_owned(),
            },
            other => other,
        })?;
        row.try_get::<Uuid, _>("id").map_err(StoreError::Database)
    }

    /// Open the sealed value of one row.
    ///
    /// The audited expansion path. `None` when no such row exists, which is a
    /// body referencing a row that was removed — reported as absent rather
    /// than as a decryption failure, because the two have different causes.
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
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(None);
        };
        let scheme: String = row.try_get("scheme").map_err(StoreError::Database)?;
        let nonce: Vec<u8> = row.try_get("nonce").map_err(StoreError::Database)?;
        let ciphertext: Vec<u8> = row.try_get("ciphertext").map_err(StoreError::Database)?;
        Ok(Some(keys.open(&scheme, &nonce, &ciphertext)?))
    }

    /// The party holding `value` in `scheme`, without decrypting anything.
    ///
    /// Goes through `party.resolve_national_identifier`, which takes the
    /// keyed digest rather than the value: the caller proves it already knows
    /// the identifier, and the plaintext never crosses into the database.
    ///
    /// # Errors
    /// [`StoreError::Database`] when the function call fails.
    pub async fn resolve(
        &self,
        keys: &DomainKeys,
        scheme: &str,
        value: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        let digest = keys.lookup_digest(scheme, value);
        let party: Option<Uuid> = sqlx::query_scalar("SELECT resolve_national_identifier($1, $2)")
            .bind(scheme)
            .bind(&digest)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
        Ok(party)
    }
}
