// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version containers an EHR Extract reads: those the request may see,
//! the restricted objects and an unreadable `EHR_ACCESS` left out. No openEHR
//! spec governs which objects a disclosure leaves out: our own design.

use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::storage::extract_repo::{self, Enforced};
use crate::versioning::Kind;

impl FerroEhrService {
    /// Every version container of an EHR, restricted objects omitted.
    ///
    /// An Extract is a disclosure GDPR Art. 18(2) does not admit for a
    /// restricted object (`docs/law/eu/gdpr/text.html`), so the object is
    /// absent rather than failing the export, as in the AQL result set. An
    /// object under an EHDS Art. 8 restriction, or withheld from the person
    /// under Art. 3(3), is absent while the request enforces it
    /// (`docs/law/eu/ehds/text.html`). No openEHR spec governs either: our own
    /// design/extension. The `EHR_ACCESS` is absent unless the extract may
    /// carry it ([`Self::carries_ehr_access`]), as EHDS Art. 8 keeps a
    /// restriction invisible to a provider.
    pub(super) async fn ehr_versioned_objects(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<(VoId, String)>, ServiceError> {
        let objects = extract_repo::ehr_objects(&self.pool, ehr_id, enforced(ehr_id));
        self.readable_objects(ehr_id, objects.await?).await
    }

    /// The kind of a version container of `ehr_id` (an
    /// `EXTRACT_ENTITY_MANIFEST.item_list` entry); `None` for an `EHR_ACCESS`
    /// the extract may not carry, as for an unknown object.
    pub(super) async fn ehr_vo_kind(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
    ) -> Result<Option<String>, ServiceError> {
        let kind = extract_repo::ehr_object_kind(&self.pool, ehr_id, vo_id, enforced(ehr_id));
        self.readable_kind(ehr_id, kind.await?).await
    }

    /// Whether an extract for the request's caller carries the `EHR_ACCESS` of
    /// `ehr_id`: for a caller who reads its settings
    /// ([`FerroEhrService::ehr_access_view`]), and for the holder of an
    /// `export_ehr_extract` grant covering the EHR, whose extract transfers
    /// the whole EHR (RM EHR Extract IM, `X_VERSIONED_EHR_ACCESS`); our own
    /// design (#5559).
    async fn carries_ehr_access(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(Box::pin(self.ehr_access_view(ehr_id)).await?.reads()
            || self.export_granted(ehr_id).await?)
    }

    /// Whether an extract for the request's caller carries the `EHR_ACCESS`
    /// versions of `ehr_id` without their marking and signature: for a
    /// gate-keeper who is not the subject and holds no `export_ehr_extract`
    /// grant covering the EHR, whose extract would transfer the marking with
    /// the record (draft access control, The Decision Procedure, Reading and
    /// Writing the Settings and Exchange; our own design, #6165).
    ///
    /// # Errors
    /// [`ServiceError`] when the settings or the subject cannot be read.
    pub async fn extract_redacts(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(Box::pin(self.ehr_access_view(ehr_id)).await?.redacts()
            && !self.export_granted(ehr_id).await?)
    }

    /// Redacts `version`, an `ORIGINAL_VERSION` of a container of `kind` in
    /// `ehr_id`, where the extract carries it without its marking
    /// ([`Self::extract_redacts`]).
    #[expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/dump-load compose over \
                  verbatim stored content (RM common master06 §Copying)"
    )]
    pub(super) async fn redact_extracted(
        &self,
        ehr_id: EhrId,
        kind: &str,
        version: &mut serde_json::Value,
    ) -> Result<(), ServiceError> {
        if kind == Kind::EhrAccess.as_str() && self.extract_redacts(ehr_id).await? {
            crate::service::ehr::access_view::redact_version(version);
        }
        Ok(())
    }

    /// `objects` without the `EHR_ACCESS` when the extract may not carry it
    /// ([`Self::carries_ehr_access`]).
    async fn readable_objects(
        &self,
        ehr_id: EhrId,
        mut objects: Vec<(VoId, String)>,
    ) -> Result<Vec<(VoId, String)>, ServiceError> {
        let access = Kind::EhrAccess.as_str();
        if objects.iter().any(|(_, kind)| kind == access)
            && !self.carries_ehr_access(ehr_id).await?
        {
            objects.retain(|(_, kind)| kind != access);
        }
        Ok(objects)
    }

    /// `kind`, or `None` for the `EHR_ACCESS` when the extract may not carry
    /// it ([`Self::carries_ehr_access`]).
    async fn readable_kind(
        &self,
        ehr_id: EhrId,
        kind: Option<String>,
    ) -> Result<Option<String>, ServiceError> {
        if kind.as_deref() == Some(Kind::EhrAccess.as_str())
            && !self.carries_ehr_access(ehr_id).await?
        {
            return Ok(None);
        }
        Ok(kind)
    }
}

/// The access marks the current request enforces on `ehr_id`, which every
/// extract read of it honours.
pub(super) fn enforced(ehr_id: EhrId) -> Enforced {
    Enforced {
        access_delay: crate::system_log::access_context::access_delay_enforced_on(ehr_id),
    }
}
