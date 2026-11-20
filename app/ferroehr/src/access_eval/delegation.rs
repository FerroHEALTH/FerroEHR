// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step 3 of the decision procedure for a representative: one a
//! `delegation` directive names reads only what an applying delegation
//! permit covers (EHDS Art. 4(2)). No openEHR spec governs the procedure:
//! our own design (#5609).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Effect, Refusal};

use crate::access_eval::directive::{Reader, kind_and_effect};
use crate::access_eval::procedure::{Outcome, Rule, Verdict};
use crate::access_eval::scope::{Polarity, Pred};

/// Step 3 for a representative: one a `delegation` directive names reads
/// only what an applying delegation permit covers (EHDS Art. 4(2)).
pub(super) fn delegation(
    read: &Reader<'_, '_>,
    directives: &[AccessDirective],
    verdict: &mut Verdict,
) {
    let naming: Vec<(usize, &AccessDirective, Pred)> = directives
        .iter()
        .enumerate()
        .filter(|(_, d)| kind_and_effect(d) == (DirectiveKind::Delegation, Effect::Permit))
        .filter(|(_, d)| d.applies_to.is_some())
        .map(|(at, d)| (at, d, read.actor(d, read.placed(at))))
        .filter(|(_, _, named)| *named != Pred::False)
        .collect();
    if naming.is_empty() {
        return;
    }
    let applying: Vec<(usize, &AccessDirective, Pred)> = naming
        .iter()
        .map(|(at, d, _)| (*at, *d, read.applies(d, Polarity::Permit, read.placed(*at))))
        .filter(|(_, _, applies)| *applies != Pred::False)
        .collect();
    verdict.grounds.permits = applying
        .iter()
        .filter(|(_, _, applies)| *applies == Pred::True)
        .map(|(at, _, _)| *at)
        .collect();
    let covered = applying
        .iter()
        .map(|(at, d, applies)| {
            Pred::all(vec![
                applies.clone(),
                read.cover(d, Effect::Permit, read.placed(*at)),
            ])
        })
        .collect();
    let named = Pred::any(naming.iter().map(|(_, _, named)| named.clone()).collect());
    verdict.rules.push(Rule {
        when: Pred::all(vec![named, Pred::negate(Pred::any(covered))]),
        then: Outcome::Refused(Refusal::ConsentDenied),
        by: naming.iter().map(|(at, _, _)| *at).collect(),
    });
}
