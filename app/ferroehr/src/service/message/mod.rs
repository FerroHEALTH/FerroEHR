// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Message service.
//!
//! The platform crate's realization of SM `I_MESSAGE_SERVICE` /
//! `I_EHR_EXTRACT_SERVICE` / `I_TDD_SERVICE`
//! (`docs/specs/openehr/SM/docs/openehr_platform/master09-message_service.adoc`
//! and the UML classes `i_message_service.adoc`, `i_ehr_extract_service.adoc`,
//! `i_tdd_service.adoc`).
//!
//! The layout mirrors the spec's own export / import / TDD decomposition, each
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
//! - `tdd` — `I_TDD_SERVICE.import_tdd` / `import_tdds`: TDD XML to COMPOSITION
//!   to validated commit.
//!
//! `I_MESSAGE_SERVICE` declares no functions (`i_message_service.adoc`), so it
//! gets no code.
//!
//! # Cross-module wiring
//!
//! `import` reaches [`crate::versioning`]'s `commit_import` /
//! `commit_demographic_import`, and `export` reaches its `original_version` /
//! `versioned_object` / `revision_history` and the version reads. `tdd` resolves
//! the OPT and commits through the validated COMPOSITION path
//! (`FerroEhrService::web_template_for` / `get_template_xml` /
//! `create_composition`). A completed export or import emits one EHR-Extract
//! audit event (`FerroEhrService::emit_extract_audit`) for non-repudiation (BASE
//! `architecture_overview/master07-security.adoc` §Non-repudiation).
//! `EXTRACT_SPEC.criteria` and `commit_time_interval` remain typed rejects (the
//! NOTEs in `export`). The extension REST wire lives in `ferroehr-rest`, ITS-REST
//! vending no message endpoints.

mod export;
mod import;
mod tdd;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::status::SmError;
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
    /// by the [`EventActionCode`] (`Read` out / `Create` in). The native
    /// service layer has no HTTP principal, so `user_id` stays empty and the
    /// ATNA renderer supplies `UNKNOWN`.
    ///
    /// # Errors
    /// The `service_overloaded` [`SmError`] when the sender rejected the record
    /// under `fail_mode = "closed"`.
    pub(super) fn emit_extract_audit(
        &self,
        ehr_id: EhrId,
        action: EventActionCode,
    ) -> Result<(), SmError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(action, ObjectClass::Extract, EventOutcome::Success);
        let id = ehr_id.to_string();
        event.ehr_id = Some(id.clone());
        event.object_id = Some(id);
        self.record_access(event)
    }
}
