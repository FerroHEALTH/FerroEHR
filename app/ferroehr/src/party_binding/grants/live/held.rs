// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the grants copy, the caller facts copy and the care copy hold, for
//! the copies' gauges and the memory budget (#5760, #5926). No openEHR spec
//! governs the copies: our own design.

use std::collections::BTreeMap;

use super::LiveGrants;

impl LiveGrants {
    /// What the care copy holds: its subjects and care teams, the patients
    /// and declared record subjects of those teams, and their bytes; bounded
    /// by [`crate::party_binding::grants::care::SUBJECTS`] and
    /// [`crate::party_binding::grants::care::CENSUS_BYTES`].
    #[must_use]
    pub fn care_held(&self) -> crate::party_binding::grants::care::CareHeld {
        self.care.held()
    }

    /// Runs the care copy's pending cache upkeep ([`crate::service::upkeep`]).
    pub(crate) async fn upkeep(&self) {
        self.care.upkeep().await;
    }

    /// How many capabilities and client `ACTOR`s the copy of the grant rows
    /// holds, and their bytes. The copy holds every grant row and is bounded
    /// by the grant register, never by a cap of its own: a grant it left out
    /// would never admit.
    #[must_use]
    pub fn grants_held(&self) -> crate::memory::Held {
        let held = self.held();
        let clients: usize = held.clients.values().map(BTreeMap::len).sum();
        let rows: usize = held.capabilities.values().map(Vec::len).sum();
        crate::memory::Held {
            entries: crate::memory::gauge(clients.saturating_add(rows)),
            bytes: crate::memory::gauge(
                crate::memory::bytes(&held.clients)
                    .saturating_add(crate::memory::bytes(&held.capabilities)),
            ),
        }
    }

    /// How many parties the copy of the caller facts holds facts of, and
    /// the bytes of their facts. The copy holds the facts of every caller
    /// party and is bounded by the party register, never by a cap of its
    /// own: a party it left out would be decided without its facts.
    #[must_use]
    pub fn facts_held(&self) -> crate::memory::Held {
        let facts = self.held_facts();
        crate::memory::Held {
            entries: crate::memory::gauge(facts.by_party.len()),
            bytes: crate::memory::gauge(crate::memory::bytes(&facts.by_party)),
        }
    }
}
