// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Message service.
//!
//! EHR Extract export and import over the RM EHR Extract model, behind the
//! `/message` extension. Our own design; the operation names borrow from the
//! openEHR SM text, kept as reference and never an oracle.
//!
//! The layout follows the export / import split, each
//! file carrying its public `FerroEhrService` methods and the machinery behind
//! them:
//!
//! - `export` — `I_EHR_EXTRACT_SERVICE.export_ehrs` / `export_ehr_extracts`:
//!   the extract-building algorithm over the stored versions (RM EHR-Extract IM
//!   `master05`/`master09`).
//! - `import` — `I_EHR_EXTRACT_SERVICE.import_ehr` / `import_ehr_extract`:
//!   parse a received `EXTRACT` and dispatch clone against append; the
//!   `IMPORTED_VERSION` replay is [`crate::versioning`] (RM common master06
//!   §Copying).
//!
//! # Cross-module wiring
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
mod import;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::status::SmError;
use crate::system_log::categories::AccessedContent;
use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};

impl FerroEhrService {
    /// Emit one IHE-ATNA EHR-Extract audit event for a completed export or
    /// import of `ehr_id`, honouring the configured fail mode: under
    /// `[audit] fail_mode = "closed"` a record the sender rejects withholds the
    /// operation's result ([`FerroEhrService::record_access`]).
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
    /// The `service_overloaded` [`SmError`] when the sender rejected the record
    /// under `fail_mode = "closed"`.
    pub(super) fn emit_extract_audit(
        &self,
        ehr_id: EhrId,
        action: EventActionCode,
        content: Option<AccessedContent>,
    ) -> Result<(), SmError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(action, ObjectClass::Extract, EventOutcome::Success);
        let id = ehr_id.to_string();
        event.ehr_id = Some(id.clone());
        event.object_id = Some(id);
        event.purpose = crate::system_log::access_context::current_purpose();
        event.permit = crate::system_log::access_context::current_permit();
        event.category = content.and_then(|content| self.audit_classify(&content, action));
        self.record_access(event)
    }
}
