// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An access narrowed to episodes of care: what it reaches, and the verdict
//! that refuses the rest. No openEHR spec governs the narrowing: our own
//! design (draft access control, ITS-REST, Queries and the Caseload; #6417).

use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::Refusal;

use super::{Judged, Outcome, Rule, Verdict};
use crate::access_eval::scope::{Polarity, Pred};
use crate::versioning::episode::EpisodeKey;

/// The items an access narrowed to `episodes` reaches: the EHR, the data
/// placed in none of them or in one, and the data `episode_shared` names
/// (draft access control, ITS-REST, Queries and the Caseload).
pub(super) fn within(judged: &Judged<'_>, episodes: &[EpisodeKey]) -> Pred {
    let shared = judged.policy.episode_shared;
    // NOTE: draft BMM ACCESS_POLICY.episode_shared, void means no data are shared, where an
    // empty data list elsewhere names the whole EHR.
    let shared = if shared.is_empty() {
        Pred::False
    } else {
        judged.scopes.data(shared, Polarity::Permit, None)
    };
    Pred::any(vec![Pred::InEpisodes(Arc::from(episodes)), shared])
}

/// `verdict` with every item outside `within` refused
/// `refused-outside-episode` where the verdict would admit it, and every other
/// answer kept.
pub(super) fn narrowed(mut verdict: Verdict, within: Pred) -> Verdict {
    for rule in &mut verdict.rules {
        if rule.then == Outcome::Admit {
            rule.when = Pred::all(vec![rule.when.clone(), within.clone()]);
        }
    }
    if verdict.otherwise == Outcome::Admit {
        verdict.rules.push(Rule {
            when: Pred::negate(within),
            then: Outcome::Refused(Refusal::OutsideEpisode),
            by: Vec::new(),
        });
    }
    verdict
}
