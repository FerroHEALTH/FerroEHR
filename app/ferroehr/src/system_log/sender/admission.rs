// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What an act asks of the sender before it changes anything: whether a
//! record would be taken now, and the store an act that writes its record in
//! its own transaction writes it through. No openEHR spec governs the audit trail: our own design.

use std::sync::atomic::Ordering;

#[cfg(feature = "fhir")]
use openehr_base::containers::NonEmptyVec;
#[cfg(feature = "fhir")]
use openehr_rm::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;
#[cfg(feature = "fhir")]
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
#[cfg(feature = "fhir")]
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

#[cfg(feature = "fhir")]
use crate::system_log::AuditError;
use crate::system_log::config::FailMode;
#[cfg(feature = "fhir")]
use crate::system_log::event::AuditEvent;
use crate::system_log::sender::AuditSender;

impl AuditSender {
    /// Whether a record emitted now would be taken: always under
    /// `fail_mode = "open"`; under `"closed"` only while the queue has room
    /// and the local store accepts writes.
    ///
    /// An act that cannot be undone asks before it acts, so a record the
    /// sender would refuse refuses the act instead of following it.
    #[must_use]
    pub fn admits(&self) -> bool {
        match self.inner.fail_mode {
            FailMode::Open => true,
            FailMode::Closed => {
                self.inner.tx.capacity() > 0 && self.inner.store_healthy.load(Ordering::Relaxed)
            }
        }
    }

    /// Replace the access-log entry `uid` by its tombstone, removed at
    /// `time_removed` on `grounds`, and store `event`, the record of the act,
    /// as its own access-log entry, in one transaction of the local store;
    /// `None` when the local store is off. The answer within is `None` when
    /// no entry has the uid.
    ///
    /// # Errors
    /// The errors of [`crate::system_log::store::AuditStore::erase_entry`].
    #[cfg(feature = "fhir")]
    pub async fn erase_entry(
        &self,
        uid: &str,
        time_removed: DvDateTime,
        grounds: NonEmptyVec<DvCodedText>,
        event: &AuditEvent,
    ) -> Option<Result<Option<AccessLogTombstone>, AuditError>> {
        let store = self.inner.store.as_ref()?;
        let erased = store.erase_entry(uid, time_removed, grounds, event).await;
        Some(erased)
    }

    /// Append the access-log entries the drain writes through `access_log`,
    /// the store `[audit.access_log]` configured; `false` when the local
    /// store is off or one was installed already.
    #[must_use]
    pub fn use_access_log(&self, access_log: crate::access_log::store::AccessLogStore) -> bool {
        self.inner
            .store
            .as_ref()
            .is_some_and(|store| store.use_access_log(access_log))
    }

    /// The access-log store the server installed ([`Self::use_access_log`]),
    /// which signs links as configured.
    #[must_use]
    pub fn access_log(&self) -> Option<crate::access_log::store::AccessLogStore> {
        self.inner.store.as_ref()?.access_log()
    }
}
