// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The steps of the decision procedure that bar one caller from one record
//! whatever else admits it: its standing and the controller's exclusion
//! (step 0), and a conflict of interest (step 5).
//!
//! An exclusion is a deny of kind `exclusion` in
//! `CONFIDENTIALITY_SETTINGS.exclusions`, naming the parties it keeps out by
//! `applies_to`; it binds a representative and a gate-keeper and never the
//! subject (draft access control package, Exclusions; 45 CFR
//! 164.502(g)(5)). A professional is in conflict with the subject when its
//! own `PERSON` is the subject's, or a `related_to` relationship in force
//! joins the two, and a conflict rule of the policy alone lets it through;
//! kinship refuses and never grants (draft decision procedure, Terms and
//! step 5). No openEHR spec governs either step: our own design (#6077).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    CallerKind, Conflict, DirectiveKind, Effect, ForeignCode, OperationClass, Refusal, Term,
};

use crate::access_eval::directive::Reader;
use crate::access_eval::procedure::{Judged, Lifted, Outcome, Rule, Verdict};
use crate::access_eval::scope::Pred;

/// Step 0, exclusion: every exclusion of the controller that binds the
/// caller, or the mark of an emergency that lifts it. A permit of the
/// subject or of a representative never lifts one, and an emergency only
/// where its `emergency_override` holds and the caller is no representative.
pub(super) fn excluded(
    read: &Reader<'_, '_>,
    exclusions: &[AccessDirective],
    verdict: &mut Verdict,
) {
    let judged = read.judged;
    let lifts =
        judged.asked.emergency.break_glass() && judged.caller.kind != CallerKind::Representative;
    for (at, exclusion) in exclusions.iter().enumerate() {
        let placed = read.placed_exclusion(at);
        let binds = read.binds(DirectiveKind::Exclusion, exclusion, placed);
        if binds == Pred::False {
            continue;
        }
        let cover = read.cover(exclusion, Effect::Deny, placed);
        if exclusion.emergency_override && lifts {
            if binds == Pred::True {
                verdict.lifted.push(Lifted::Exclusion);
                verdict.grounds.lifted_exclusions.push((at, cover));
            }
            continue;
        }
        verdict.rules.push(Rule {
            when: Pred::all(vec![binds, cover]),
            then: Outcome::Refused(Refusal::Excluded),
            by: vec![at],
        });
    }
}

/// Step 5, conflict: the refusal of a professional in conflict with the
/// subject that no conflict rule of the policy allows, or `None`, with each
/// conflict a rule allowed noted in the grounds. On its own record a caller
/// reaches this step only on the professional path
/// ([`Caller::professional_path`](crate::access_eval::procedure::Caller::professional_path)).
pub(super) fn conflict(read: &Reader<'_, '_>, verdict: &mut Verdict) -> Option<Rule> {
    let judged = read.judged;
    if judged.caller.kind != CallerKind::Professional {
        return None;
    }
    let mut conflicts: Vec<(Conflict, Option<&ForeignCode>)> = judged
        .ehr
        .kinships
        .iter()
        .map(|kinship| (Conflict::Related, Some(kinship)))
        .collect();
    if judged.ehr.subject {
        conflicts.push((Conflict::SelfAccess, None));
    }
    if conflicts.is_empty() {
        return None;
    }
    let allowances = judged.conflict_rules;
    let allowed = |(conflict, kinship): &(Conflict, Option<&ForeignCode>)| {
        allowances.iter().any(|rule| {
            conflict.is(&rule.conflict)
                && read.allows(rule.applies_to.as_deref())
                && kinship.is_none_or(|kinship| {
                    rule.kinships.as_deref().is_none_or(|codes| {
                        codes.iter().any(|coded| {
                            coded.defining_code.terminology_id.value == kinship.terminology
                                && coded.defining_code.code_string == kinship.code
                        })
                    })
                })
        })
    };
    if conflicts.iter().all(allowed) {
        let mut noted: Vec<Conflict> = conflicts.iter().map(|(conflict, _)| *conflict).collect();
        noted.sort_unstable();
        noted.dedup();
        verdict.grounds.conflicts = noted;
        return None;
    }
    Some(Rule {
        when: Pred::True,
        then: Outcome::Refused(Refusal::Conflict),
        by: Vec::new(),
    })
}

/// Step 0: the refusal of a caller whose standing reaches no data of the
/// EHR, whatever its directives permit.
pub(super) fn standing(judged: &Judged<'_>) -> Option<Refusal> {
    let caller = judged.caller;
    // NOTE: our own design (#5586, #5606, #5609), SMART master08 §Resource Scopes and EHDS
    // Art. 4(2) hold a patient context or a representative to one EHR, so every other refuses.
    if caller.confined() && !judged.ehr.own {
        return Some(caller.not_own());
    }
    // NOTE: our own design (#5614), a read under an agreement conditioned on consent names
    // the consent decision, so a caller naming none is refused whatever the EHR permits.
    let reads = matches!(
        judged.asked.operation,
        OperationClass::Read | OperationClass::Query | OperationClass::Export
    );
    (caller.lacks_consent_reference && reads).then_some(Refusal::ConsentReferenceMissing)
}
