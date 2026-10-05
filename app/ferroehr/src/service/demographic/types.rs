// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_DEMOGRAPHIC_SERVICE` (`i_demographic_service.adoc`: "Primary interface
//! to `DEMOGRAPHIC_SERVICE`") and `I_PARTY` (`i_party.adoc`: "Interface for
//! `PARTY` level operations").

/// The concrete PARTY resource families of the DEMOGRAPHIC group (the five
/// concrete `ACTOR`/`PARTY` leaves of the RM demographic package the wire
/// routes are keyed by).
///
/// NOTE: the SM addresses parties only by versioned-object id; the
/// per-kind routing is our wire extension (module NOTE) — the RM `_type`
/// of the payload is the authority, `kind` the route key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyKind {
    /// `AGENT` (`/demographic/agent`).
    Agent,
    /// `GROUP` (`/demographic/group`).
    Group,
    /// `ORGANISATION` (`/demographic/organisation`).
    Organisation,
    /// `PERSON` (`/demographic/person`).
    Person,
    /// `ROLE` (`/demographic/role`).
    Role,
}

impl PartyKind {
    /// The RM `_type` this resource family stores (`PERSON`, `ROLE`, …).
    #[must_use]
    pub fn rm_type(self) -> &'static str {
        match self {
            PartyKind::Agent => "AGENT",
            PartyKind::Group => "GROUP",
            PartyKind::Organisation => "ORGANISATION",
            PartyKind::Person => "PERSON",
            PartyKind::Role => "ROLE",
        }
    }

    /// The URL path segment of this resource family (`agent`, `person`, …).
    #[must_use]
    pub fn segment(self) -> &'static str {
        match self {
            PartyKind::Agent => "agent",
            PartyKind::Group => "group",
            PartyKind::Organisation => "organisation",
            PartyKind::Person => "person",
            PartyKind::Role => "role",
        }
    }
}
