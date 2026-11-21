// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who a query reads for: the EHRs whose `EHR_ACCESS` settings it reads rows
//! of, and whether a demographic query serves the policy parties.

use std::sync::Arc;

/// The EHRs whose `EHR_ACCESS` settings a query may read rows of.
///
/// The service derives it from the caller and the settings' gate-keepers
/// (`crate::service::ehr::access_rule`), and the SQL generator applies it to
/// the head row of every versioned-object source, so a restriction stays
/// invisible to a provider (EHDS Art. 8). No openEHR spec governs who reads
/// the settings: our own design.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EhrAccessReaders {
    /// Every EHR's: the controller, an administrator.
    #[default]
    Every,
    /// Every EHR's whose settings hold no exclusions of the controller: the
    /// subject in their own EHR.
    Unexcluded,
    /// Every EHR's whose settings hold no exclusions of the controller,
    /// without the marking: a gate-keeper who is not the subject, in the one
    /// EHR a query is pinned to.
    UnexcludedWithoutMarking,
    /// Only the EHRs whose settings name the caller a gate-keeper, judged
    /// per settings class, and hold no exclusions of the controller, without
    /// the marking.
    Only(Arc<crate::access_eval::class::Keepers>),
}

impl EhrAccessReaders {
    /// Whether the query serves the settings it reads without the marking,
    /// which only the subject and the controller read (draft access
    /// control, The Decision Procedure, Reading and Writing the Settings,
    /// item 3; #6165).
    #[must_use]
    pub fn redacts(&self) -> bool {
        matches!(self, Self::UnexcludedWithoutMarking | Self::Only(_))
    }
}

/// Whether a demographic query serves the policy parties
/// ([`crate::party_binding::policy`]).
///
/// The service decides it per request: only an administrator reads a policy
/// party, through the Demographic API (`party_binding::protect::check_read`)
/// and through AQL alike. No openEHR spec governs who reads a party, and
/// QUERY master03 §FROM gives semantics to the clinical repository only: our
/// own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyParties {
    /// Every party is served: an administrator's query, or one on a
    /// deployment whose mapping recognises no policy party.
    Served,
    /// The query leaves out every version of a party that ever had a policy
    /// version ([`crate::party_binding::policy_mark`]): its head row carries
    /// the mark (`vo_head.policy_party_at`) the commit of such a version sets.
    Hidden,
}

impl PolicyParties {
    /// What the running request reads under `mapping`: the policy parties
    /// are hidden when the mapping recognises them and the request is no
    /// administrator's (`party_binding::protect::administrator`).
    #[must_use]
    pub fn of_request(mapping: &crate::party_binding::BindingMapping) -> Self {
        if mapping.names_policy() && !crate::party_binding::protect::administrator(mapping) {
            Self::Hidden
        } else {
            Self::Served
        }
    }
}
