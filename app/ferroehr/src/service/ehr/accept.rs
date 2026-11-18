// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The acceptance of a typed commit body: the privacy pass over its one
//! indexed rendering, the RM and root rules of its kind over typed RM, and the
//! offload of its large media, before any connection is held (#6003).
//!
//! The rules are those of [`super::validation`] for the canonical value of the
//! same kind, with the same messages and paths. A `553|incomplete|` commit
//! keeps the canonical-JSON passes by decision (#6002). No openEHR spec governs
//! the representation a commit is validated in: our own design.

use openehr_rm::prelude::{EhrAccess, EhrStatus, Folder};
use openehr_rm::v1_2::common::archetyped::archetyped::Archetyped;

use super::validation::{Submitted, rm_verdict};
use crate::error::{ServiceError, Violation};
use crate::service::FerroEhrService;
use crate::versioning::Kind;
use crate::versioning::body::Body;

impl FerroEhrService {
    /// Validates a commit body and, when it is valid, externalizes its large
    /// media, before any connection is held.
    ///
    /// The outer error is the offload's, which refuses the write at once; the
    /// inner result is the validation's, which the caller surfaces after its
    /// own gates.
    ///
    /// # Errors
    /// The offload's [`ServiceError`].
    pub(crate) async fn accept_body(
        &self,
        kind: Kind,
        body: &mut Body,
        incomplete: bool,
    ) -> Result<Result<(), ServiceError>, ServiceError> {
        let validated = self.validate_body(kind, body, incomplete).await;
        if validated.is_ok() {
            self.externalize_body(body).await?;
        }
        Ok(validated)
    }

    /// The validation half of [`Self::accept_body`].
    ///
    /// # Errors
    /// As [`Self::validate_for_commit`].
    pub(crate) async fn validate_body(
        &self,
        kind: Kind,
        body: &Body,
        incomplete: bool,
    ) -> Result<(), ServiceError> {
        match body {
            Body::Canonical(data) => self.validate_for_commit(kind, data, incomplete).await,
            // NOTE: a `553|incomplete|` commit keeps the canonical-JSON passes by
            // decision (#6002), so it alone takes a canonical view to validate.
            typed if incomplete => {
                let view = openehr_its::json::to_canonical_value(typed);
                self.validate_for_commit(kind, &view, true).await
            }
            typed => self.validate_typed_for_commit(kind, typed).await,
        }
    }

    /// [`Self::validate_for_commit`] over a typed body.
    ///
    /// # Errors
    /// As [`Self::validate_for_commit`].
    async fn validate_typed_for_commit(&self, kind: Kind, body: &Body) -> Result<(), ServiceError> {
        if !kind.is_demographic() && self.privacy.reads_content() {
            crate::telemetry::conversion::record("rm_to_index");
            let rendered = crate::storage::render::index::render(body)
                .map_err(|e| ServiceError::internal("serialize the commit body", e))?;
            self.privacy.enforce_in(
                kind.as_str(),
                crate::versioning::tree::Indexed::root(&rendered),
            )?;
        }
        match body {
            Body::Composition(rm) => {
                self.validate_composition_for_commit(Submitted::Typed(rm), false)
                    .await
            }
            Body::EhrStatus(rm) => validate_ehr_status(rm),
            Body::EhrAccess(rm) => self.validate_ehr_access(rm).await,
            Body::Folder(rm) => validate_folder(rm),
            other => {
                let view = openehr_its::json::to_canonical_value(other);
                self.validate_for_commit(kind, &view, false).await
            }
        }
    }

    /// The typed twin of
    /// [`validate_ehr_access_for_commit`](Self::validate_ehr_access_for_commit):
    /// the root rule, the scheme of a present `settings`, the RM pass below the
    /// root (which leaves the open `settings` slot out) and the parties the
    /// settings name.
    ///
    /// # Errors
    /// As the canonical twin.
    async fn validate_ehr_access(&self, rm: &EhrAccess) -> Result<(), ServiceError> {
        root_locatable(rm.archetype_details.as_deref(), "EHR_ACCESS")?;
        let settings = rm
            .settings
            .as_ref()
            .map(openehr_its::json::to_canonical_value);
        if let Some(settings) = &settings {
            crate::service::ehr::access_settings::check_written_settings(settings)?;
        }
        rm_verdict(openehr_sdt::rm_instance::validate_rm_and_terminology_typed(
            rm,
        ))?;
        match &settings {
            Some(settings) => {
                self.check_directive_parties(
                    settings,
                    &crate::service::ehr::access_parties::Landing::new(),
                )
                .await
            }
            None => Ok(()),
        }
    }
}

/// The typed twin of [`super::validation::validate_ehr_status`]: the root
/// rule and the RM pass (the typed value is an `EHR_STATUS` by construction).
///
/// # Errors
/// As the canonical twin.
fn validate_ehr_status(rm: &EhrStatus) -> Result<(), ServiceError> {
    root_locatable(rm.archetype_details.as_deref(), "EHR_STATUS")?;
    rm_verdict(openehr_sdt::rm_instance::validate_rm_and_terminology_typed(
        rm,
    ))
}

/// The typed twin of [`super::validation::validate_folder`].
///
/// # Errors
/// As the canonical twin.
fn validate_folder(rm: &Folder) -> Result<(), ServiceError> {
    // NOTE: RM master03 §The LOCATABLE Class: `archetype_details` "is non-Void for archetype
    // root points in data", where `archetype_node_id` "carries the archetype identifier".
    if openehr_adl::hrid::is_archetype_id(&rm.archetype_node_id) && rm.archetype_details.is_none() {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "is mandatory: the directory root names an archetype in archetype_node_id, \
                 so it is an archetype root and must carry ARCHETYPED",
            )
            .with_path("FOLDER.archetype_details")
            .with_invariant("LOCATABLE.Archetyped_valid"),
        ));
    }
    rm_verdict(openehr_sdt::rm_instance::validate_rm_and_terminology_typed(
        rm,
    ))
}

/// The typed twin of [`super::validation::validate_root_locatable`].
///
/// # Errors
/// As the canonical twin.
fn root_locatable(details: Option<&Archetyped>, kind: &str) -> Result<(), ServiceError> {
    let details = details.ok_or_else(|| {
        ServiceError::content_invalid(
            Violation::new(format!(
                "is mandatory: {kind} is an archetype root (Is_archetype_root), \
                 and a root without ARCHETYPED is invalid; supply \
                 archetype_details with its archetype_id (for a plain \
                 status, openEHR-EHR-EHR_STATUS.generic.v1)"
            ))
            .with_path(format!("{kind}.archetype_details"))
            .with_invariant("LOCATABLE.Archetyped_valid"),
        )
    })?;
    if details.archetype_id.value.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("is mandatory")
                .with_path(format!("{kind}.archetype_details.archetype_id.value"))
                .with_invariant("ARCHETYPED.archetype_id 1..1"),
        ));
    }
    Ok(())
}
