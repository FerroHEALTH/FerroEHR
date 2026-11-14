// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the operator metrics and the health surface read of the service: the
//! policy copies' listeners, the size of the access copies and the audit
//! drain's queue ([`crate::telemetry::access_path`]). No openEHR spec governs
//! them: our own design (#5591).

use std::sync::Arc;

use crate::policy_register::listen::Listened;
use crate::service::FerroEhrService;

impl FerroEhrService {
    /// Each policy copy's listener, by the name the `policy_listeners` health
    /// indicator and the `listener` label of the metrics report it under.
    #[must_use]
    pub fn policy_listeners(&self) -> Vec<(&'static str, Arc<Listened>)> {
        vec![
            ("ehr_access", Arc::clone(self.ehr_access_listened())),
            ("grants", Arc::clone(self.grants().listened())),
            (
                "policy_registers",
                Arc::clone(self.policy_registers().listened()),
            ),
            ("access_policy", Arc::clone(self.access_policy().listened())),
        ]
    }

    /// How many EHRs' settings this replica's access copy holds, and how many
    /// settings classes; both approximate while entries are being dropped.
    #[must_use]
    pub fn ehr_access_held(&self) -> (u64, u64) {
        self.ehr_access.held_counts()
    }

    /// The records waiting in the audit drain's queue; `None` when auditing
    /// is off.
    #[must_use]
    pub fn audit_queued(&self) -> Option<u64> {
        self.audit
            .as_ref()
            .map(|sender| u64::try_from(sender.queued()).unwrap_or(u64::MAX))
    }
}
