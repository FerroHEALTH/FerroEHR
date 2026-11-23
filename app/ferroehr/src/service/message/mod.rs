// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Message service.
//!
//! - `export` — `export_ehrs` / `export_ehr_extracts`:
//!   the extract-building algorithm over the stored versions (RM EHR-Extract IM
//!   `master05`/`master09`).
//! - `extract_type` — the `EXTRACT_SPEC.extract_type` check `export` applies.
//! - `import` — `import_ehr` / `import_ehr_extract`:
//!   parse a received `EXTRACT` and dispatch clone against append; the
//!   `IMPORTED_VERSION` replay is [`crate::versioning`] (RM common master06
//!   §Copying).
//!
//! `import` reaches [`crate::versioning`]'s `commit_import` /
//! `commit_demographic_import`, and `export` reaches its `original_version` /
//! `versioned_object` / `revision_history` and the version reads. A completed
//! export or import emits one EHR-Extract audit event
//! (`FerroEhrService::emit_extract_audit`) for non-repudiation (BASE
//! `architecture_overview/master07-security.adoc` §Non-repudiation).
//! `EXTRACT_SPEC.criteria` and `commit_time_interval` remain typed rejects (the
//! NOTEs in `export`). The extension REST wire lives in `ferroehr-rest`, ITS-REST
//! vending no message endpoints.

mod export;
mod export_objects;
mod extract_type;
mod import;
pub mod import_policy;

use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};

impl FerroEhrService {
    /// Emit one IHE-ATNA EHR-Extract audit event for a completed export or
    /// import of `ehr_id`, written before the result is reported: a record
    /// that cannot be written withholds the operation's result
    /// ([`FerroEhrService::record_access`]).
    ///
    /// EHR-Extract communication carries patient-identifiable clinical data
    /// across systems and is audited for **non-repudiation** — the security
    /// chapter requires that "logging of communication of Extracts … can be
    /// used to guarantee non-repudiation of information passed between systems"
    /// (BASE `architecture_overview/master07-security.adoc` §Non-repudiation).
    /// The event carries [`ObjectClass::Extract`], which renders the
    /// Patient-Record `EventID` family (DICOM PS3.15 §A.5.1, code 110110) with
    /// `originalText="extract"` and an ehr-id-scoped patient participant
    /// (`crate::system_log::codes`); the export-vs-import direction is carried
    /// by the [`EventActionCode`] (`Read` out / `Create` in). The user is the
    /// request's authenticated committer, which
    /// [`FerroEhrService::record_access`] stamps; the purpose and the data
    /// permit the caller declared (EHDS Art. 68) are the request's. `content` is what the
    /// Extract served or landed, which the record carries as its EHDS priority
    /// categories (Annex II 3.2(c), `docs/law/eu/ehds/text.html`).
    ///
    /// # Errors
    /// [`ServiceError::Unrecorded`] when the record cannot be written.
    pub(super) async fn emit_extract_audit(
        &self,
        ehr_id: EhrId,
        action: EventActionCode,
        content: Option<AccessedContent>,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        self.record_access(self.extract_record(ehr_id, action, content))
            .await
    }

    /// The EHR-Extract record of `ehr_id`, as [`Self::emit_extract_audit`]
    /// writes it.
    pub(super) fn extract_record(
        &self,
        ehr_id: EhrId,
        action: EventActionCode,
        content: Option<AccessedContent>,
    ) -> AuditEvent {
        let mut event = AuditEvent::new(action, ObjectClass::Extract, EventOutcome::Success);
        let id = ehr_id.to_string();
        event.ehr_id = Some(id.clone());
        event.object_id = Some(id);
        // NOTE: EHDS Annex II 3.2 and Art. 9 (`docs/law/eu/ehds/text.html`), the record of an
        // export names the request and the bound accessor, as the adapter's records do (#5638).
        let origin = crate::system_log::access_context::current_origin();
        event.request_id = origin.request_id;
        event.client_ip = origin.client_address;
        event.binding = crate::system_log::access_context::current_binding();
        event.purpose = crate::system_log::access_context::current_purpose();
        event.permit = crate::system_log::access_context::current_permit();
        event.category = content.and_then(|content| self.audit_classify(&content, action));
        event
    }

    /// Emit the EHR-Extract audit event of the import of `ehr_id`, whose
    /// record names the content of the versioned objects `landed`.
    ///
    /// # Errors
    /// As [`Self::emit_extract_audit`].
    async fn import_record(
        &self,
        tx: &mut sqlx::PgConnection,
        ehr_id: EhrId,
        landed: &[VoId],
    ) -> Vec<AuditEvent> {
        if !self.audit_enabled() {
            return Vec::new();
        }
        let content = self.landed_content(tx, landed).await;
        vec![self.extract_record(ehr_id, EventActionCode::Create, Some(content))]
    }

    /// What an import landed, for its access record's EHDS priority category
    /// (Annex II 3.2(c), `docs/law/eu/ehds/text.html`): the newest content of
    /// each received versioned object.
    ///
    /// The import has committed by then, so a failed read is logged and the
    /// record is written `unclassified` rather than failing a completed import.
    async fn landed_content(
        &self,
        tx: &mut sqlx::PgConnection,
        landed: &[VoId],
    ) -> AccessedContent {
        match self.audit_content_of_objects_on(tx, landed).await {
            Ok(content) => AccessedContent::Objects(content),
            Err(e) => {
                tracing::warn!(
                    error = %crate::error::ErrorChain::new(&e),
                    "reading the categories of an imported Extract failed; the access \
                     record is written unclassified"
                );
                AccessedContent::Objects(Vec::new())
            }
        }
    }
}
