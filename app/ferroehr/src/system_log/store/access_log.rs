// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access-log store the audit store appends its records' entries
//! through: the one the server configures to sign links, installed after
//! boot. No openEHR spec governs the audit trail: our own design.

use crate::access_log::store::AccessLogStore;
use crate::system_log::store::AuditStore;

impl AuditStore {
    /// Append the access-log entries of every record this store and its
    /// clones write through `access_log`, the store configured to sign links;
    /// `false` when one was installed already, which stays.
    #[must_use]
    pub fn use_access_log(&self, access_log: AccessLogStore) -> bool {
        self.access_log.set(access_log).is_ok()
    }

    /// The access-log store the server installed ([`Self::use_access_log`]).
    #[must_use]
    pub fn access_log(&self) -> Option<AccessLogStore> {
        self.access_log.get().cloned()
    }
}
