// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Steps 7 and 8 over the data a care relationship covers: a caller who
//! holds a live relationship is admitted on care only to the data it covers,
//! and refused `refused-outside-episode` the rest (draft access control,
//! decision procedure, steps 7 and 8; [`crate::access_eval::coverage`]). A
//! justification the caller invokes admits the data no relationship covers
//! when the request states a reason, and refuses them
//! `refused-reason-missing` when it does not
//! ([`crate::access_eval::justification`]).

use openehr_rm::draft::access_control::terminology::{AccessDefault, Refusal};

use super::{Judged, Outcome, Rule, Verdict};
use crate::access_eval::care_tie::Cared;
use crate::access_eval::directive::Permit;
use crate::access_eval::justification::{Invoked, Justified};
use crate::access_eval::scope::Pred;

/// What steps 7 and 8 read beside the judged access.
pub(super) struct Held<'a, 'p> {
    /// The applying permit directives.
    pub(super) permits: &'a [Permit<'p>],
    /// Whether a gate-keeper names the caller.
    pub(super) keeper: &'a Pred,
    /// Whether the caller holds a live care relationship that is a ground.
    pub(super) live: bool,
    /// Whether the policy requires a care relationship of this caller.
    pub(super) care_read: bool,
    /// The default disposition.
    pub(super) default: AccessDefault,
    /// The justification the request invokes.
    pub(super) justified: Option<Invoked>,
}

/// Pushes the refusals of step 7, and of step 8 under a `restricted`
/// default, onto `verdict`, and returns the data the caller's live
/// relationships and its justification cover.
pub(super) fn steps_7_and_8(
    judged: &Judged<'_>,
    held: &Held<'_, '_>,
    verdict: &mut Verdict,
) -> Pred {
    let covering = match judged.ehr.care {
        Cared::Ties(ties) if held.live => {
            crate::access_eval::coverage::covered(judged, ties.iter())
        }
        Cared::Covering if held.live => crate::access_eval::coverage::left(judged, None),
        _ => Pred::of(held.live),
    };
    let uncovered = Pred::negate(Pred::any(vec![covering.clone(), held.keeper.clone()]));
    if uncovered == Pred::False {
        return covering;
    }
    let restricted = held.default == AccessDefault::Restricted;
    if !held.care_read && !restricted {
        return covering;
    }
    let permitted = Pred::any(
        held.permits
            .iter()
            .filter(|permit| !held.care_read || permit.directive.applies_to.is_some())
            .map(|permit| permit.cover.clone())
            .collect(),
    );
    let refused = Pred::all(vec![uncovered, Pred::negate(permitted)]);
    let refusal = match held.justified {
        Some(invoked) if invoked.reasoned => {
            verdict.grounds.justification = Some(Justified {
                at: invoked.at,
                emergency: false,
                cover: refused,
            });
            return Pred::True;
        }
        Some(_) => Refusal::ReasonMissing,
        None if held.live => Refusal::OutsideEpisode,
        None if held.care_read => Refusal::NoCareRelationship,
        None => return covering,
    };
    verdict.rules.push(Rule {
        when: refused,
        then: Outcome::Refused(refusal),
        by: Vec::new(),
    });
    covering
}
