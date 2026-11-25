// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The capabilities this replica resolved for each actor, kept while the
//! copy they were resolved from stands and no term they read starts or ends
//! (#6329).
//!
//! An entry holds for the grant revision and the facts install of the copy
//! it was resolved from. A request's confirm read brings the copy to the
//! store's revisions before it resolves, so a moved revision installs a new
//! copy, which no entry holds for, and the decision rests on that exact read
//! (#5891). A grant or fact also starts and ends on its own dates and
//! instants, so an entry holds only until the next of those its rows name,
//! and never past the end of the UTC day it was resolved on. No openEHR spec
//! governs the copy: our own design.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use uuid::Uuid;

use super::LiveGrants;
use crate::party_binding::grants::caller::care_of::ProviderKey;
use crate::party_binding::grants::capabilities::{Actor, Capabilities};
use crate::party_binding::grants::store::Term;

/// The most actors the copy keeps capabilities for; past it, an actor's
/// capabilities are resolved again on each request.
const KEPT: usize = 4096;

/// What kept capabilities were resolved from: the copy's grant revision and
/// facts install.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Stamp {
    revision: Option<i64>,
    facts: u64,
}

/// One actor's capabilities and the instant they stop holding.
#[derive(Debug)]
struct Kept {
    capabilities: Arc<Capabilities>,
    until: jiff::Timestamp,
}

/// The kept capabilities of one copy.
#[derive(Debug, Default)]
pub(super) struct Resolved {
    /// Moved by every change of the ceiling or the namespace, which the
    /// capabilities are resolved under.
    epoch: u64,
    stamp: Stamp,
    count: usize,
    /// Each client `ACTOR`'s, by issuer, then `client_id`.
    clients: BTreeMap<String, BTreeMap<String, Kept>>,
    /// Each party's, by the provider it acts for.
    parties: BTreeMap<Uuid, Vec<(Option<ProviderKey>, Kept)>>,
}

impl Resolved {
    /// The capabilities kept for `actor` acting for `provider` that hold at
    /// `now`.
    fn find(
        &self,
        actor: Actor<'_>,
        provider: Option<(&str, &str)>,
        now: jiff::Timestamp,
    ) -> Option<Arc<Capabilities>> {
        let kept = match actor {
            Actor::Client { issuer, client_id } => self.clients.get(issuer)?.get(client_id)?,
            Actor::Party(party) => self
                .parties
                .get(&party)?
                .iter()
                .find(|(held, _)| {
                    held.as_ref()
                        .map(|key| (key.issuer.as_str(), key.id.as_str()))
                        == provider
                })
                .map(|(_, kept)| kept)?,
        };
        (now < kept.until).then(|| Arc::clone(&kept.capabilities))
    }

    /// Keep `kept` for `actor` acting for `provider`, while fewer than
    /// [`KEPT`] are kept.
    fn keep(&mut self, actor: Actor<'_>, provider: Option<(&str, &str)>, kept: Kept) {
        if self.count >= KEPT {
            return;
        }
        match actor {
            Actor::Client { issuer, client_id } => {
                let clients = self.clients.entry(issuer.to_owned()).or_default();
                if clients.insert(client_id.to_owned(), kept).is_none() {
                    self.count = self.count.saturating_add(1);
                }
            }
            Actor::Party(party) => {
                let provider = provider.map(|(issuer, id)| ProviderKey::new(issuer, id));
                let held = self.parties.entry(party).or_default();
                let before = held.len();
                held.retain(|(key, _)| *key != provider);
                held.push((provider, kept));
                self.count = self.count.saturating_sub(before).saturating_add(held.len());
            }
        }
    }

    /// Forget every kept capability and hold the next ones for `stamp`.
    fn reset(&mut self, stamp: Stamp) {
        self.stamp = stamp;
        self.count = 0;
        self.clients.clear();
        self.parties.clear();
    }
}

impl LiveGrants {
    /// The capabilities of `actor` at `now`, a party acting for the
    /// deployment's `provider`: those kept for it from the copy held when
    /// they still hold, else resolved from the copy ([`Self::capabilities`])
    /// and kept.
    #[must_use]
    pub fn resolved(
        &self,
        actor: Actor<'_>,
        provider: Option<(&str, &str)>,
        now: jiff::Timestamp,
    ) -> Arc<Capabilities> {
        let provider = match actor {
            Actor::Party(_) => provider,
            Actor::Client { .. } => None,
        };
        let stamp = self.stamp();
        let epoch = {
            let resolved = self.resolved.read().unwrap_or_else(PoisonError::into_inner);
            if resolved.stamp == stamp
                && let Some(kept) = resolved.find(actor, provider, now)
            {
                return kept;
            }
            resolved.epoch
        };
        let mut capabilities = self.capabilities(actor, now);
        if let Actor::Party(_) = actor {
            capabilities =
                capabilities.acting_for(provider.map(|(issuer, id)| ProviderKey::new(issuer, id)));
        }
        let until = self.settles(actor, now);
        let capabilities = Arc::new(capabilities);
        // NOTE: our own design (#6329), capabilities resolved while the copy or the settings
        // moved answer this request only.
        if self.stamp() == stamp {
            let mut resolved = self
                .resolved
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if resolved.epoch == epoch {
                if resolved.stamp != stamp {
                    resolved.reset(stamp);
                }
                let capabilities = Arc::clone(&capabilities);
                resolved.keep(
                    actor,
                    provider,
                    Kept {
                        capabilities,
                        until,
                    },
                );
            }
        }
        capabilities
    }

    /// Forget every kept capability: the settings they were resolved under
    /// changed.
    pub(super) fn forget_resolved(&self) {
        let mut resolved = self
            .resolved
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        resolved.epoch = resolved.epoch.wrapping_add(1);
        let stamp = resolved.stamp;
        resolved.reset(stamp);
    }

    /// The grant revision and facts install of the copy held.
    fn stamp(&self) -> Stamp {
        Stamp {
            revision: self.revision(),
            facts: self.held_facts().generation,
        }
    }

    /// The first instant after `now` at which a row the capabilities of
    /// `actor` read may start or stop holding: the next of their instants,
    /// and at the latest the end of `now`'s UTC day, past which their dates
    /// are judged on another day.
    fn settles(&self, actor: Actor<'_>, now: jiff::Timestamp) -> jiff::Timestamp {
        let held = self.held();
        let (performer, provider) = match actor {
            Actor::Client { issuer, client_id } => {
                match held.clients.get(issuer).and_then(|c| c.get(client_id)) {
                    Some(client) => (
                        client.party,
                        client.provider.as_ref().map(|provider| provider.party),
                    ),
                    None => return end_of_day(now),
                }
            }
            Actor::Party(party) => (party, None),
        };
        let facts = self.held_facts();
        let granted = held.capabilities.get(&performer).into_iter().flatten();
        let stated = std::iter::once(performer)
            .chain(provider)
            .filter_map(|party| facts.by_party.get(&party))
            .flat_map(|rows| rows.iter());
        settled(
            granted
                .map(|row| &row.term)
                .chain(stated.map(|row| &row.term)),
            now,
        )
    }
}

/// The first instant after `now` at which one of `terms` may start or stop
/// holding by its instants, and at the latest the end of `now`'s UTC day.
fn settled<'a>(terms: impl Iterator<Item = &'a Term>, now: jiff::Timestamp) -> jiff::Timestamp {
    terms
        .filter_map(|term| term.next_change(now))
        .fold(end_of_day(now), Ord::min)
}

/// The first instant of the UTC day after the one `now` falls on.
fn end_of_day(now: jiff::Timestamp) -> jiff::Timestamp {
    now.to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .tomorrow()
        .and_then(|day| day.to_zoned(jiff::tz::TimeZone::UTC))
        .map_or(now, |day| day.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::store::Dates;

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().expect("an instant")
    }

    fn term(not_before: Option<&str>, not_after: Option<&str>) -> Term {
        Term {
            lifecycle_state: "532".to_owned(),
            role_dates: Some(Dates {
                lower: None,
                upper: None,
            }),
            dates: Some(Dates {
                lower: None,
                upper: None,
            }),
            not_before: not_before.map(at),
            not_after: not_after.map(at),
        }
    }

    /// Kept capabilities hold until the next instant a term starts or stops
    /// holding, and never past the end of the UTC day (#6329).
    #[test]
    fn kept_capabilities_hold_until_a_term_moves() {
        let now = at("2026-12-02T12:00:00Z");
        let midnight = at("2026-12-03T00:00:00Z");
        assert_eq!(settled(std::iter::empty(), now), midnight);
        let starts = term(Some("2026-12-02T13:00:00Z"), None);
        assert_eq!(
            settled([&starts].into_iter(), now),
            at("2026-12-02T13:00:00Z")
        );
        let ends = term(None, Some("2026-12-02T14:00:00Z"));
        let until = settled([&starts, &ends].into_iter(), now);
        assert_eq!(until, at("2026-12-02T13:00:00Z"));
        let ended = term(Some("2026-12-01T00:00:00Z"), Some("2026-12-02T11:00:00Z"));
        assert_eq!(settled([&ended].into_iter(), now), midnight);
        let last = term(None, Some("2026-12-02T14:00:00Z"));
        let until = settled([&last].into_iter(), now);
        assert!(last.holds(
            now.to_zoned(jiff::tz::TimeZone::UTC).date(),
            at("2026-12-02T14:00:00Z")
        ));
        assert!(
            until > at("2026-12-02T14:00:00Z"),
            "the last instant it holds is kept"
        );
        assert!(!last.holds(now.to_zoned(jiff::tz::TimeZone::UTC).date(), until));
    }

    /// One party's capabilities are kept apart by the provider it acts for,
    /// and an entry past its instant is not found.
    #[test]
    fn a_party_is_kept_per_provider() {
        let now = at("2026-12-02T12:00:00Z");
        let party = Uuid::now_v7();
        let mut resolved = Resolved::default();
        let kept = |until| Kept {
            capabilities: Arc::default(),
            until,
        };
        resolved.keep(Actor::Party(party), None, kept(at("2026-12-02T13:00:00Z")));
        resolved.keep(
            Actor::Party(party),
            Some(("hospital", "1")),
            kept(at("2026-12-02T13:00:00Z")),
        );
        resolved.keep(Actor::Party(party), None, kept(at("2026-12-02T12:30:00Z")));
        assert_eq!(resolved.count, 2);
        assert!(resolved.find(Actor::Party(party), None, now).is_some());
        assert!(
            resolved
                .find(Actor::Party(party), Some(("hospital", "1")), now)
                .is_some()
        );
        assert!(
            resolved
                .find(Actor::Party(party), Some(("hospital", "2")), now)
                .is_none()
        );
        assert!(
            resolved
                .find(Actor::Party(party), None, at("2026-12-02T12:30:00Z"))
                .is_none(),
            "an entry is not found at its instant"
        );
    }
}
