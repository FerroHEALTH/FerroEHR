// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Steps 7 and 8 over the data a care relationship covers: a caller who
//! holds a live relationship is admitted on care only to the data it covers,
//! and refused `refused-outside-episode` the rest (draft access control,
//! decision procedure, steps 7 and 8; [`crate::access_eval::coverage`]).

use openehr_rm::draft::access_control::terminology::{AccessDefault, Refusal};

use super::{Judged, Outcome, Rule, Verdict};
use crate::access_eval::care_tie::Cared;
use crate::access_eval::directive::Permit;
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
}

/// Pushes the refusals of step 7, and of step 8 under a `restricted`
/// default, onto `verdict`, and returns the data the caller's live
/// relationships cover.
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
    let refusal = if held.live {
        Refusal::OutsideEpisode
    } else {
        Refusal::NoCareRelationship
    };
    if held.care_read {
        let naming = held
            .permits
            .iter()
            .filter(|permit| permit.directive.applies_to.is_some())
            .map(|permit| permit.cover.clone())
            .collect();
        verdict.rules.push(Rule {
            when: Pred::all(vec![uncovered, Pred::negate(Pred::any(naming))]),
            then: Outcome::Refused(refusal),
            by: Vec::new(),
        });
    } else if held.default == AccessDefault::Restricted && held.live {
        let permitted = Pred::any(
            held.permits
                .iter()
                .map(|permit| permit.cover.clone())
                .collect(),
        );
        verdict.rules.push(Rule {
            when: Pred::all(vec![uncovered, Pred::negate(permitted)]),
            then: Outcome::Refused(refusal),
            by: Vec::new(),
        });
    }
    covering
}
