// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default definitions seeded at boot.
//!
//! The binary carries a default definition of each policy archetype
//! ([`crate::party_binding::policy::DEFINITIONS`]), of the access policy
//! archetype ([`crate::access_policy::DEFINITIONS`]) and the default
//! demographic definitions the default party mapping names
//! ([`crate::party_binding::shipped::DEFINITIONS`]), and the default
//! `EHR_ACCESS` definition of the `access_directives` scheme
//! ([`crate::service::ehr::access_settings::DEFINITIONS`]). Every boot stores each
//! one through the normal definition path when no definition of its id is
//! stored, and leaves a stored one alone whatever its text: it never
//! overwrites a deployment's definition and never re-seeds over its change.
//! After that they are ordinary definitions, uploaded, versioned and replaced
//! through the definition API. No openEHR spec governs seeding definitions:
//! our own design.

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::definition::load::{Loaded, SourceFormat};

/// What the seed did with one default definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seeded {
    /// No definition of the id was stored, so the default was.
    Stored(String),
    /// A definition of the id was stored already and is left as it is, the
    /// default's text or a deployment's own; the file name of the default.
    Kept(String),
}

impl FerroEhrService {
    /// Seeds the default definitions, the policy and access policy
    /// definitions first and the `EHR_ACCESS` definition last: stores
    /// each one whose id no stored definition holds, and keeps every stored
    /// one as it is.
    ///
    /// # Errors
    /// Every refusal of the ADL 2 upload path but the stored-id conflict, and
    /// a database failure.
    pub async fn seed_default_definitions(&self) -> Result<Vec<Seeded>, ServiceError> {
        let definitions = crate::party_binding::policy::DEFINITIONS
            .iter()
            .chain(&crate::access_policy::DEFINITIONS)
            .chain(&crate::party_binding::shipped::DEFINITIONS)
            .chain(&crate::service::ehr::access_settings::DEFINITIONS);
        let mut seeded = Vec::new();
        for (file, text) in definitions {
            seeded.push(match self.definition_load(SourceFormat::Adl2, text).await {
                Ok(Loaded::Stored(id)) => Seeded::Stored(id),
                Ok(Loaded::Unchanged(_)) | Err(ServiceError::Conflict(_)) => {
                    Seeded::Kept((*file).to_owned())
                }
                Err(e) => return Err(e),
            });
        }
        Ok(seeded)
    }
}
