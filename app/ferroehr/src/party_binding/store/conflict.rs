// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `409` a commit answers when a party it writes would take a selector or
//! a client identifier another live party holds. No openEHR spec governs the
//! side table's unique indexes: our own design.

use crate::error::ServiceError;
use crate::error::refusal::{Clash, Refusal};
use crate::storage::error::StorageError;

/// The `409` of a violation of one of the selector unique indexes: the commit
/// gives a binding key, a `userName`, an `externalId` or a group
/// `displayName` a second live party. Every other failure is classified as
/// storage.
pub(super) fn selector_in_use(e: StorageError) -> ServiceError {
    let StorageError::Database(source) = e else {
        return ServiceError::Storage(e);
    };
    let selector = source
        .as_database_error()
        .filter(|db| db.is_unique_violation())
        .and_then(|db| db.constraint())
        .and_then(|constraint| match constraint {
            "uq_binding_identifier_key" => Some("binding key"),
            "uq_binding_identifier_user_name" => Some("userName"),
            "uq_binding_identifier_external_id" => Some("externalId"),
            "uq_binding_identifier_display_name" => Some("displayName"),
            "uq_binding_client" => Some("client identifier"),
            _ => None,
        });
    match selector {
        Some(selector) => ServiceError::Conflict(
            Refusal::new(
                Clash::State,
                format!("the {selector} this party carries is already held by another party"),
            )
            .with_source(source),
        ),
        None => ServiceError::Storage(StorageError::Database(source)),
    }
}
