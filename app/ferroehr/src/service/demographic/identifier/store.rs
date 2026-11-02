// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Sealing and opening `party.national_identifier` values.
//!
//! **No openEHR spec governs this — our own design/extension.** The split
//! between the operations is the access-control boundary:
//! [`IdentifierStore::seal`] writes a value, and [`IdentifierStore::open`]
//! reads one back for the audited expansion. The key holder seals and opens
//! here; the rows themselves are read and written by
//! [`crate::storage::identifier_repo`], which only ever sees sealed bytes.
//!
//! The writes run on the caller's connection, never on the store's pool: a
//! sealed row exists only for a party version that commits, because it is
//! written inside that version's own transaction. The audited reads use the
//! pool.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::VoId;
use crate::service::demographic::identifier::crypto::{CryptoError, DomainKeys};
use crate::service::error::{ServiceError, Violation};
use crate::service::refusal::{Clash, Refusal};
use crate::storage::identifier_repo::{self, IdentifierRepoError, NewSealed};

/// What went wrong holding or reading a protected identifier.
///
/// No variant carries the value: these render into logs and error bodies.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The sealed row could not be read or written.
    #[error(transparent)]
    Repo(#[from] IdentifierRepoError),
    /// The value could not be sealed or opened.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
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
}

impl StoreError {
    /// Whether the body referenced a sealed row its party does not hold.
    #[must_use]
    pub fn is_foreign_reference(&self) -> bool {
        matches!(self, StoreError::ForeignReference)
    }
}

/// The commit's answer to a protected-identifier store refusal.
///
/// A value another party holds is ITS-REST `409 Conflict` ("the request could
/// not be processed because it might generate a duplicate or a conflict",
/// `ITS-REST/specifications/docs/overview/Requests_and_responses.md` §HTTP
/// status codes). A body that repeats a scheme, carries a reference the party
/// does not hold, or must be stored verbatim with a protected value in
/// plaintext is `422` (`responses/422.yaml`: semantic errors in a resource
/// that parsed). An unregistered scheme is the deployment's own
/// configuration disagreeing with its registry, which the client cannot act
/// on, so it stays a `500` whose logged context names the scheme. No message
/// carries an identifier value.
impl From<StoreError> for ServiceError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Repo(IdentifierRepoError::ValueTaken { ref scheme }) => {
                ServiceError::Conflict(
                    Refusal::new(
                        Clash::State,
                        format!(
                            "a `{scheme}` identifier of this party is already held by another \
                             party"
                        ),
                    )
                    .with_source(error),
                )
            }
            StoreError::Repo(IdentifierRepoError::SchemeRepeated { .. })
            | StoreError::ForeignReference
            | StoreError::Unsealed { .. } => {
                ServiceError::content_invalid(Violation::new(error.to_string()))
            }
            StoreError::Repo(IdentifierRepoError::UnknownScheme { ref scheme }) => {
                ServiceError::internal(
                    format!(
                        "identifier scheme `{scheme}` is configured for protection but absent \
                         from the identifier_scheme registry"
                    ),
                    error,
                )
            }
            other => ServiceError::internal("seal a protected national identifier", other),
        }
    }
}

/// The commit's answer to a refused sealed-row statement: the same answer as
/// for the [`StoreError::Repo`] that carries it.
impl From<IdentifierRepoError> for ServiceError {
    fn from(error: IdentifierRepoError) -> Self {
        ServiceError::from(StoreError::Repo(error))
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

    /// The demographic pool the audited reads run on.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Seal `value` for `party` on `conn` as a new live row and return the row
    /// id the body will reference.
    ///
    /// `conn` is the party domain's commit transaction, so the seal and the
    /// commit are one act and a rolled-back commit leaves no sealed row. There
    /// is deliberately no pool-backed variant, so no caller can seal outside a
    /// commit. [`identifier_repo::retire_live`] must have run first.
    ///
    /// # Errors
    /// [`StoreError::Repo`] carrying [`IdentifierRepoError::ValueTaken`] when
    /// the value is live for another party,
    /// [`IdentifierRepoError::SchemeRepeated`] when this party already has a
    /// live row in the scheme, [`IdentifierRepoError::UnknownScheme`] for an
    /// unregistered scheme, or a database fault, and [`StoreError::Crypto`]
    /// when the value cannot be sealed. A database refusal aborts the caller's
    /// transaction, so the caller must fail its commit.
    pub async fn seal(
        conn: &mut PgConnection,
        keys: &DomainKeys,
        party: Uuid,
        scheme: &str,
        value: &str,
    ) -> Result<Uuid, StoreError> {
        let (nonce, ciphertext) = keys.seal(scheme, value)?;
        let digest = keys.lookup_digest(scheme, value);
        let sealed = NewSealed {
            scheme,
            nonce: &nonce,
            ciphertext: &ciphertext,
            lookup_digest: &digest,
        };
        Ok(identifier_repo::insert_sealed(conn, VoId(party), sealed).await?)
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
        let Some(row) = identifier_repo::read_sealed(conn, row_id, Some(VoId(party))).await? else {
            return Ok(None);
        };
        let value = keys.open(&row.scheme, &row.nonce, &row.ciphertext)?;
        Ok(Some((row.scheme, value)))
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
        let mut conn = acquire(&self.pool)
            .await
            .map_err(IdentifierRepoError::from)?;
        let Some(row) = identifier_repo::read_sealed(&mut conn, row_id, None).await? else {
            return Ok(None);
        };
        Ok(Some(keys.open(&row.scheme, &row.nonce, &row.ciphertext)?))
    }
}
