// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the operator metrics and the health surface read of the service: the
//! policy copies' listeners, the size of the access copies and the audit
//! drain's queue ([`crate::telemetry::access_path`]). No openEHR spec governs
//! them: our own design (#5591).

use std::sync::Arc;

use crate::memory::Held;
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
            ("party_binding", Arc::clone(self.bindings().listened())),
            (
                "policy_registers",
                Arc::clone(self.policy_registers().listened()),
            ),
            ("access_policy", Arc::clone(self.access_policy().listened())),
            (
                "registered_templates",
                Arc::clone(self.registered_templates().listened()),
            ),
        ]
    }

    /// What each of this replica's access copies holds, by the `copy`
    /// label of the metrics: its entries and their bytes, both approximate
    /// while entries are being dropped (#5926).
    #[must_use]
    pub fn copies_held(&self) -> Vec<(&'static str, Held)> {
        let (ehrs, classes) = self.ehr_access.held();
        let grants = self.grants();
        let care = grants.care_held();
        let (lookups, names) = self.bindings().held();
        let (versions, version_bytes) = self.version_copy.held();
        vec![
            ("ehr", ehrs),
            ("classes", classes),
            ("care_subjects", care.subjects),
            ("care_persons", care.persons),
            ("care_teams", care.teams),
            ("care_patients", care.patients),
            ("care_declared", care.declared),
            ("caller_facts", grants.facts_held()),
            ("grants", grants.grants_held()),
            ("binding", lookups),
            ("party_name", names),
            (
                "object_version",
                Held {
                    entries: versions,
                    bytes: version_bytes,
                },
            ),
        ]
    }

    /// What each of this replica's caches holds, by the `cache` label of the
    /// metrics: its entries and their bytes, both approximate while entries
    /// are being evicted (#5926). The audit sender's memo is absent while
    /// auditing is off.
    #[must_use]
    pub fn caches_held(&self) -> Vec<(&'static str, Held)> {
        let (templates, template_bytes) = self.web_templates.held();
        let mut held = vec![
            ("created_ehr", crate::memory::held(&self.created_ehr_repr)),
            (
                "web_template",
                Held {
                    entries: templates,
                    bytes: template_bytes,
                },
            ),
            ("aql_plan", self.plan_cache.held()),
            (
                "archetype_lineage",
                crate::memory::held(&self.archetype_lineage),
            ),
        ];
        if let Some(audit) = &self.audit {
            held.push(("audit_subjects", audit.subjects_held()));
        }
        held
    }
}
