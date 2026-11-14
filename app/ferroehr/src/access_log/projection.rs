// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projections of a stored access-log entry: the EHDS access record, the
//! IHE BALP `AuditEvent` and the DICOM audit message (#5127).
//!
//! The entry is the record (`docs/plans/access-log.md` §Mappings and their
//! losses). Each projection reads it through
//! [`crate::access_log::recorded::stored`], computes its categories again
//! through the category map that classified it, and hands the result to the
//! FerroAUDIT crates: `ehds-logging` writes the EHDS access record
//! (Regulation (EU) 2025/327 Annex II 3.2) and, from it, the BALP
//! `AuditEvent` (IHE BALP 1.1.4, `docs/specs/ihe-balp/`; FHIR R4
//! `AuditEvent`, `docs/specs/fhir-auditevent/R4/`); an access the map did
//! not classify is a BALP exchange `ihe-atna` writes; the DICOM PS3.15 A.5
//! message is written on `ihe-atna`'s message model. The ITI-81 search, the
//! ITI-20 FHIR feed and the syslog sender serve these projections. The
//! FerroAUDIT crates take no openEHR type.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): a carrier of an \
              already-rendered FHIR document stays a JSON value"
)]

use ehds_logging::record::AccessRecord;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use secrecy::SecretSlice;

use crate::access_log::recorded::stored;
use crate::system_log::AuditError;
use crate::system_log::categories::Classifier;
use crate::system_log::event::AuditEvent;
use crate::system_log::message::AuditContext;
use crate::system_log::render::classified;

/// What a projection is written under beside the entry: the deployment's
/// audit source and fills, and the category map that classified the entry.
#[derive(Debug, Clone, Copy)]
pub struct Projection<'a> {
    /// The audit source, its site and host, and the record policies.
    pub context: &'a AuditContext,
    /// The category map the entry was classified through.
    pub categories: &'a Classifier,
}

impl Projection<'_> {
    /// The audit record `entry` holds at chain position `chain_seq`, its
    /// categories computed again, and the subject of care of its EHR.
    ///
    /// # Errors
    /// [`AuditError::Recorded`] when the entry does not read as an audit
    /// record, and [`AuditError::Unreadable`] when the map is not the one
    /// that classified it.
    pub fn record(
        &self,
        entry: &AccessLogEntry,
        chain_seq: i64,
    ) -> Result<(AuditEvent, Option<String>), AuditError> {
        classified(stored(entry, chain_seq)?, self.categories)
    }

    /// The EHDS access record of `entry`; `None` for an access the category
    /// map did not reach, which is no access to classified data.
    ///
    /// # Errors
    /// As [`Self::record`], and [`AuditError::Render`] when the binding
    /// record has no form in the access record.
    pub fn access_record(
        &self,
        entry: &AccessLogEntry,
        chain_seq: i64,
    ) -> Result<Option<AccessRecord>, AuditError> {
        let (event, subject) = self.record(entry, chain_seq)?;
        crate::system_log::fhir::access_record(&event, self.context, subject.as_deref())
    }

    /// The FHIR R4 `AuditEvent` of `entry` on the IHE BALP patterns.
    ///
    /// # Errors
    /// As [`Self::record`], and [`AuditError::Render`] when the document
    /// cannot be written.
    pub fn audit_event(
        &self,
        entry: &AccessLogEntry,
        chain_seq: i64,
    ) -> Result<serde_json::Value, AuditError> {
        let (event, subject) = self.record(entry, chain_seq)?;
        crate::system_log::fhir::to_fhir(&event, self.context, subject.as_deref())
    }

    /// The DICOM PS3.15 A.5 audit message of `entry`, as the UTF-8 XML the
    /// syslog frame carries.
    ///
    /// # Errors
    /// As [`Self::record`], [`AuditError::Render`] when the binding record has
    /// no form in the message, and [`AuditError::Xml`] when it cannot be
    /// written.
    pub fn audit_message(
        &self,
        entry: &AccessLogEntry,
        chain_seq: i64,
    ) -> Result<SecretSlice<u8>, AuditError> {
        let (event, subject) = self.record(entry, chain_seq)?;
        crate::system_log::message::to_xml(&event, self.context, subject.as_deref())
    }
}

#[cfg(test)]
mod tests;
