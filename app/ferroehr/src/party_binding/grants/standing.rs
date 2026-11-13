// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The standing of a client, a delegate actor and a scopeless subject, read
//! from the policy grants.
//!
//! `ferrohealth-smart` asks it while it classifies a token, and it answers
//! from this replica's copy of the grant rows.
//!
//! Each question names an identifier its issuer issued, and the answer is the
//! grants of the `ACTOR` carrying that identifier as a `client_id` under that
//! issuer: [`Operation::SignInPersons`] makes a client first party,
//! [`Operation::ActForPersons`] a delegate (a client, or the RFC 8693 actor a
//! person's token names in `act`), and [`Operation::AccessWithoutScope`]
//! admits a system caller whose token carries no resource scope. The copy is
//! read again only when a notification or a lost listener marked it stale,
//! so a question adds no transaction; a store that cannot answer is a
//! [`StandingError`], which refuses the token with `503`. SMART App Launch
//! 2.2.0 and RFC 8693 §4.1 leave both lists to the resource server, and no
//! openEHR spec gives a capability an authorization meaning: our own design.

use std::sync::Arc;

use ferrohealth_smart::kind::Actor as TokenActor;
use ferrohealth_smart::kind::standing::{Answer, ClientStanding, Issued, Standing, StandingError};

use crate::party_binding::grants::capabilities::{Actor, Capabilities};
use crate::party_binding::grants::live::LiveGrants;
use crate::party_binding::grants::operation::Operation;

/// The standing questions answered from the policy grants of `grants`.
#[derive(Debug, Clone)]
pub struct GrantStanding {
    /// This replica's copy of the grant rows.
    grants: Arc<LiveGrants>,
}

impl GrantStanding {
    /// The standing read from `grants`.
    #[must_use]
    pub fn new(grants: Arc<LiveGrants>) -> Self {
        Self { grants }
    }

    /// The grants in force now of the `ACTOR` carrying the client identifier
    /// `id` of `issuer`, after reading the rows when the copy is stale.
    async fn of(&self, issuer: &str, id: &str) -> Result<Capabilities, StandingError> {
        self.grants
            .refresh_if_stale()
            .await
            .map_err(|error| StandingError::new(error.into_inner()))?;
        Ok(self.grants.capabilities(
            Actor::Client {
                issuer,
                client_id: id,
            },
            jiff::Timestamp::now(),
        ))
    }
}

/// The standing `held` gives a client: first party under a
/// [`Operation::SignInPersons`] grant, else a delegate under an
/// [`Operation::ActForPersons`] grant.
fn standing_of(held: &Capabilities) -> ClientStanding {
    if held.holds(Operation::SignInPersons) {
        ClientStanding::FirstParty
    } else if held.holds(Operation::ActForPersons) {
        ClientStanding::Delegate
    } else {
        ClientStanding::Unlisted
    }
}

impl Standing for GrantStanding {
    fn client<'a>(&'a self, client: Issued<'a>) -> Answer<'a, ClientStanding> {
        Box::pin(async move { Ok(standing_of(&self.of(client.issuer, client.id).await?)) })
    }

    fn delegate<'a>(&'a self, _issuer: &'a str, actor: &'a TokenActor) -> Answer<'a, bool> {
        Box::pin(async move {
            Ok(self
                .of(&actor.issuer, &actor.sub)
                .await?
                .holds(Operation::ActForPersons))
        })
    }

    fn scopeless<'a>(&'a self, subject: Issued<'a>) -> Answer<'a, bool> {
        Box::pin(async move {
            Ok(self
                .of(subject.issuer, subject.id)
                .await?
                .holds(Operation::AccessWithoutScope))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::capabilities::Grant;

    fn holding(operations: &[Operation]) -> Capabilities {
        Capabilities::new(
            None,
            operations
                .iter()
                .map(|operation| Grant::new(*operation, uuid::Uuid::nil(), None))
                .collect(),
        )
    }

    #[test]
    fn a_first_party_grant_outranks_a_delegate_grant() {
        assert_eq!(standing_of(&holding(&[])), ClientStanding::Unlisted);
        assert_eq!(
            standing_of(&holding(&[Operation::ActForPersons])),
            ClientStanding::Delegate
        );
        assert_eq!(
            standing_of(&holding(&[
                Operation::ActForPersons,
                Operation::SignInPersons
            ])),
            ClientStanding::FirstParty
        );
        assert_eq!(
            standing_of(&holding(&[Operation::AccessWithoutScope])),
            ClientStanding::Unlisted
        );
    }
}
