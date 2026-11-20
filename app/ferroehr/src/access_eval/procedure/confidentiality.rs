// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step 6 of the decision procedure for the confidentiality of the data
//! (draft access control, The Decision Procedure, step 6). Our own design over
//! that text (#6057).

use openehr_rm::draft::access_control::terminology::Refusal;

use super::{Outcome, Rule, Verdict};
use crate::access_eval::directive::{Permit, Reader, clears_confidentiality};
use crate::access_eval::scope::{Polarity, Pred};

/// Step 6: data whose confidentiality is above `N` that no applying permit,
/// a directive of the EHR or a rule of the policy, clears; the highest level
/// is judged first, so a datum is refused under its own level.
pub(super) fn step(read: &Reader<'_, '_>, permits: &[&Permit<'_>], verdict: &mut Verdict) {
    let scopes = read.judged.scopes;
    for level in scopes.levels().into_iter().rev() {
        let clearing = permits
            .iter()
            .filter(|permit| clears_confidentiality(permit.directive, level))
            .map(|permit| permit.cover.clone())
            .collect();
        let refusal = Refusal::Confidentiality;
        verdict.rules.push(Rule {
            when: Pred::all(vec![
                scopes.at_least(level, Polarity::Deny),
                Pred::negate(Pred::any(clearing)),
            ]),
            then: if read.judged.policy.conceals(level) {
                Outcome::Absent(refusal)
            } else {
                Outcome::Refused(refusal)
            },
            by: Vec::new(),
        });
    }
}
