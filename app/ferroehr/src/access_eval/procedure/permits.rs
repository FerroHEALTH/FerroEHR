// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The permits the procedure reads at steps 6 to 8: the consent permits of
//! the EHR's directives and the permit rules of the controller's policy that
//! may bind the caller.

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Effect};

use crate::access_eval::directive::{Permit, Reader, kind_and_effect};
use crate::access_eval::scope::Pred;

/// The consent permits among `directives` that may bind the caller.
pub(super) fn consent_permits<'a>(
    read: &Reader<'_, '_>,
    directives: &'a [AccessDirective],
) -> Vec<Permit<'a>> {
    directives
        .iter()
        .enumerate()
        .filter(|(_, d)| kind_and_effect(d) == (DirectiveKind::Consent, Effect::Permit))
        .filter_map(|(at, d)| {
            let placed = read.placed(at);
            let binds = read.binds(DirectiveKind::Consent, d, placed);
            (binds != Pred::False).then(|| Permit {
                at,
                directive: d,
                certain: binds == Pred::True,
                cover: Pred::all(vec![binds, read.cover(d, Effect::Permit, placed)]),
            })
        })
        .collect()
}

/// The permit rules of the controller's policy that may bind the caller.
pub(super) fn rule_permits<'a>(read: &Reader<'_, 'a>) -> Vec<Permit<'a>> {
    read.judged
        .rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| kind_and_effect(rule).1 == Effect::Permit)
        .filter_map(|(at, rule)| {
            let binds = read.binds(DirectiveKind::ControllerRule, rule, None);
            (binds != Pred::False).then(|| Permit {
                at,
                directive: rule,
                certain: binds == Pred::True,
                cover: Pred::all(vec![binds, read.cover(rule, Effect::Permit, None)]),
            })
        })
        .collect()
}
