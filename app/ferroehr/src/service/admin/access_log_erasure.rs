// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The erasure of an access-log entry a law orders before its retention ends
//! (#5128).
//!
//! The draft's retention rules (`docs/plans/access-log.md` §Retention) send
//! such an erasure down the path the end of a retention period takes: the
//! entry is replaced by its tombstone, which keeps its uid, `system_id`,
//! `time_recorded` and link, so the chain still verifies, and names the
//! erasure's own grounds. What changes the log is recorded (§Recording rules,
//! rule 7), so the act writes an access record naming the entry. No openEHR
//! spec governs the access log: our own design.

use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_log::store::AccessLogError;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::error::refusal::Refusal;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::system_log::AuditError;
use crate::system_log::event::{
    AuditEvent, EntryScope, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use crate::versioning::audit::dv_date_time;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;

/// The name the access record gives the act.
const ACT: &str = "access-log-erasure";
/// The operation the access record and the access-log entry name the act by.
const OPERATION: &str = "admin_access_log_erase";

impl FerroEhrService {
    /// Replace the access-log entry `uid` by its tombstone on `grounds`, the
    /// provisions of the law that orders its erasure, and return the
    /// tombstone.
    ///
    /// # Errors
    /// - `not_found` (`404`): no access-log record has the uid.
    /// - `conflict` (`409`): the record is a tombstone already, or the local
    ///   audit store, which holds the access log, is off.
    /// - `bad_request` (`400`): the tombstone breaks an invariant of the
    ///   draft model, such as a ground with an empty code.
    /// - `exception`: the store failed.
    /// - `service_overloaded`: the access record of the erasure was refused
    ///   under `[audit] fail_mode = "closed"`.
    pub async fn erase_access_log_entry(
        &self,
        uid: &str,
        grounds: NonEmptyVec<DvCodedText>,
    ) -> Result<AccessLogTombstone, ServiceError> {
        let log = self.access_log_store()?;
        let entry = match log.read(uid).await.map_err(refused)? {
            None => {
                return Err(ServiceError::not_found(
                    Missing::VersionedObject,
                    format!("no access-log record has the uid {uid}"),
                ));
            }
            Some(AccessLogRecord::AccessLogTombstone(_)) => {
                return Err(ServiceError::conflict(format!(
                    "the access-log entry {uid} has been removed already"
                )));
            }
            Some(AccessLogRecord::AccessLogEntry(entry)) => entry,
        };
        // NOTE: an entry's `ehr_id` that is no EHR id of this server names no EHR here, so the
        // record names none; the tombstone and its uid still identify the act.
        let ehr_id = entry
            .ehr_id
            .as_ref()
            .and_then(|id| id.value().parse::<EhrId>().ok());
        let mut event = self.erasure_event(uid, ehr_id);
        if self.audit.as_ref().is_some_and(|sender| !sender.admits()) {
            return Err(ServiceError::Unrecorded(Refusal::new(
                (),
                "audit trail unavailable (fail-closed)",
            )));
        }
        let time_removed = dv_date_time(&jiff::Timestamp::now());
        let removed = || {
            ServiceError::conflict(format!(
                "the access-log entry {uid} has been removed already"
            ))
        };
        #[cfg(feature = "fhir")]
        let erased = match &self.audit {
            Some(sender) => {
                let mut named = event.clone();
                named.object_id = Some(uid.to_owned());
                sender
                    .erase_entry(uid, time_removed.clone(), grounds.clone(), &named)
                    .await
            }
            None => None,
        };
        #[cfg(not(feature = "fhir"))]
        let erased: Option<Result<Option<AccessLogTombstone>, AuditError>> = None;
        let tombstone = match erased {
            Some(erased) => erased.map_err(unrecorded)?,
            None => log
                .tombstone(uid, time_removed, grounds)
                .await
                .map_err(refused)?,
        }
        .ok_or_else(removed)?;
        event.entry = EntryScope::Written;
        // NOTE: our own design (#5602), the entry was stored with the tombstone, so the emitted
        // record reaches only syslog and the store-off feed, the act's one record on each.
        let _outcome = self.emit(event);
        Ok(tombstone)
    }

    /// The record of the erasure of the entry `uid`, naming the EHR the entry
    /// was about when it named one; its access-log entry names the erased
    /// entry (what changes the log is recorded).
    fn erasure_event(&self, uid: &str, ehr_id: Option<EhrId>) -> AuditEvent {
        let object = if ehr_id.is_some() {
            ObjectClass::Ehr
        } else {
            ObjectClass::ApplicationActivity
        };
        let mut event = AuditEvent::new(EventActionCode::Delete, object, EventOutcome::Success);
        event.event_type = Some(EventType::rest(OPERATION));
        event.object_id = Some(format!("{ACT}:{uid}"));
        event.ehr_id = ehr_id.map(|id| id.to_string());
        event.entry = EntryScope::LogChange;
        self.stamp_one_access(&mut event);
        event
    }
}

/// The service error of a failed erasure through the audit store.
fn unrecorded(error: AuditError) -> ServiceError {
    match error {
        AuditError::AccessLog(error) => refused(error),
        other => ServiceError::internal("the audit store", other),
    }
}

/// The service error of a failed access-log operation.
fn refused(error: AccessLogError) -> ServiceError {
    match error {
        AccessLogError::Invalid(violations) => {
            ServiceError::bad_request("the tombstone breaks the access-log model", violations)
        }
        other => ServiceError::internal("the access-log store", other),
    }
}
