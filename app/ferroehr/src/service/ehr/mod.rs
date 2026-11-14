// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR service (`service/ehr/`), behind the ITS-REST EHR API.
//!
//! Concrete `FerroEhrService` methods, our own design.
//!
//! Layout, one file per concern (arch-overview
//! `master06-design_of_the_ehr.adoc`):
//!
//! - `service`      — EHR create/get, the EHR summary, subject lookup,
//!   folder-hierarchy reads.
//! - `status`       — the `EHR_STATUS` reads + the five discrete mutators +
//!   the `is_modifiable` write guard.
//! - `directory`    — the DIRECTORY FOLDER surface.
//! - `composition`  — COMPOSITION commit and retrieval.
//! - `contributions` — CONTRIBUTION commit and retrieval.
//! - `access`       — the `EHR_ACCESS` top-level structure (arch-overview
//!   master06 §`EHR_ACCESS`) + the spec-silent per-EHR copy of its settings.
//! - `access_projection` — the settings, directives and labels each commit
//!   of an `EHR_ACCESS` version projects.
//! - `validation`   — the commit-validation choke point for every EHR-owned
//!   kind (`EHR_STATUS` / `EHR_ACCESS` / FOLDER / COMPOSITION) + the
//!   `VERSIONED_COMPOSITION` cross-version invariants.
//! - `meta` — the shared version-metadata helpers.
//! - `tags`         — `ITEM_TAG` (ITS-REST experimental extension).
//!
//! The versioned-object mechanics are delegated to [`crate::versioning`]
//! (change control, RM common master06) and [`crate::storage`] (row I/O — no
//! openEHR spec governs the SQL).
//!
//! # Integration seams
//!
//! The CONTRIBUTION commit (`crate::service::contribution`) calls
//! this chapter and storage directly: the EHR-existence gate is
//! `storage::ehr_repo::require_ehr`, `current_vo` and `invalidate_ehr_access`
//! are `FerroEhrService` methods here, and in the commit
//! transaction it runs `check_versioned_composition_invariants` (COMPOSITION
//! modify) and `FerroEhrService::sync_ehr_subject` (`EHR_STATUS` commit), the
//! same functions the direct create and update paths run inline. SQL row I/O is a storage seam
//! ([`crate::storage::ehr_repo`] / [`crate::storage::version_repo`]); no openEHR
//! spec governs the schema — our own design.

pub(in crate::service) mod access;
mod composition;
mod contributions;
mod directory;
pub(in crate::service) mod meta;
pub mod object;
pub(crate) mod service;
mod status;
pub(in crate::service) mod tags;
pub(crate) mod validation;

pub mod access_judge;
pub(in crate::service) mod access_parties;
pub mod access_projection;
pub mod access_rule;
pub mod access_settings;

use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::service::response::ResourceMeta;

/// The committed version's full [`ResourceMeta`] (version uid + commit
/// time), for write paths whose wire response carries `ETag` **and**
/// `Last-Modified` (ITS-REST overview §"`ETag` and Last-Modified": both
/// SHOULD accompany versioned resources).
fn committed_meta(
    resp: crate::service::response::ServiceResponse,
) -> Result<ResourceMeta, ServiceError> {
    resp.meta
        .ok_or_else(|| ServiceError::fault("write produced no version metadata"))
}

/// Verify a version-addressed READ returned the VERSION the caller named:
/// the addressed `OBJECT_VERSION_ID` must equal the served version's full
/// three-part identity — `object_id :: creating_system_id ::
/// version_tree_id` (ITS-REST overview `Resources.md` §Identifier types:
/// the `version_uid` "uniquely identifies a VERSION") — compared
/// case-insensitively — the shared composite-identifier rule
/// ([`composite_ids_equal`], BASE `base_types` master05 §"Composite Identifiers
/// and Case"). A tree-id-only fetch would satisfy a fabricated
/// `creating_system_id`; that names no VERSION in this repository → 404.
fn ensure_addressed_version(
    addressed: &openehr_base::prelude::ObjectVersionId,
    served_uid: &str,
) -> Result<(), ServiceError> {
    if composite_ids_equal(served_uid, addressed.value()) {
        Ok(())
    } else {
        Err(ServiceError::not_found(
            Missing::ObjectVersion,
            format!("version {}", addressed.value()),
        ))
    }
}
