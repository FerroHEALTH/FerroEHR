// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What an act asks of the sender before it changes anything: whether a
//! record would be taken now, and the access-log entry the act writes in its
//! own transaction. No openEHR spec governs the audit trail: our own design.

use std::sync::atomic::Ordering;

use crate::system_log::config::FailMode;
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

    /// The access-log entry `event` writes, as the drain would build it, for
    /// an act that writes the entry in its own transaction; `None` when the
    /// event writes none or the local store is off.
    #[cfg(feature = "fhir")]
    #[must_use]
    pub fn entry_for(
        &self,
        event: &crate::system_log::event::AuditEvent,
    ) -> Option<Box<openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry>>
    {
        self.inner.store.as_ref()?;
        crate::access_log::entry::for_store(event, &self.inner.context)
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
