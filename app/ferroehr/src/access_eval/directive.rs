// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How the decision procedure ([`crate::access_eval::procedure`]) reads a
//! directive: whether it binds the caller, applies to the request now, names
//! the caller in an actor scope, and which items it covers less its
//! exceptions, for one EHR or, through its layout, a settings class. No
//! openEHR spec governs the reading: our own design (#5513, #5588).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    CallerKind, DirectiveKind, Effect, ForeignCode, ForeignKind, OperationClass, Relationship, Term,
};

use crate::access_eval::class::{DirectiveLayout, keeper};
use crate::access_eval::procedure::Judged;
use crate::access_eval::scope::{EhrFact, Polarity, Pred, within};
use crate::service::ehr::access_projection::{Validity, validity};
use crate::service::ehr::access_rule::party_key;

/// A consent permit that may bind the caller: its position in the settings'
/// `directives`, the directive, whether it binds the caller whatever the
/// facts, and the items it covers where it binds.
pub(super) struct Permit<'a> {
    pub(super) at: usize,
    pub(super) directive: &'a AccessDirective,
    pub(super) certain: bool,
    pub(super) cover: Pred,
}

/// Whether a data scope of `directive` names the sensitivity `code`.
pub(super) fn names_sensitivity(directive: &AccessDirective, code: &ForeignCode) -> bool {
    directive
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|scope| scope.sensitivities.as_deref().unwrap_or_default())
        .any(|coded| ForeignCode::read(ForeignKind::Sensitivity, coded).as_ref() == Ok(code))
}

/// The kind and effect of `directive`; settings that hold a code outside its
/// group are held closed before they reach the evaluator, so one that does
/// not read is read as the consent deny it would refuse as.
pub(super) fn kind_and_effect(directive: &AccessDirective) -> (DirectiveKind, Effect) {
    (
        DirectiveKind::read_coded(&directive.kind).unwrap_or(DirectiveKind::Consent),
        Effect::read_coded(&directive.effect).unwrap_or(Effect::Deny),
    )
}

/// Reads directives for one decision.
pub(super) struct Reader<'j, 'a> {
    pub(super) judged: &'j Judged<'a>,
}

impl Reader<'_, '_> {
    /// Where the facts of the directive at `at` sit, for a settings class.
    pub(super) fn placed(&self, at: usize) -> Option<&DirectiveLayout> {
        self.judged
            .scopes
            .layout()
            .and_then(|layout| layout.directives.get(at))
    }

    /// Whether the caller is a gate-keeper of the EHR.
    pub(super) fn keeper(&self) -> Pred {
        let caller = &self.judged.caller.settings;
        match self.judged.scopes.layout() {
            Some(layout) => keeper(layout, caller),
            None => Pred::of(caller.is_gate_keeper(self.judged.ehr.settings)),
        }
    }

    /// Whether a directive of `kind` binds the caller: the kind binds the
    /// caller's kind, its `applies_to` names the caller, and it applies to
    /// the request now.
    pub(super) fn binds(
        &self,
        kind: DirectiveKind,
        directive: &AccessDirective,
        placed: Option<&DirectiveLayout>,
    ) -> Pred {
        let caller = self.judged.caller.kind;
        // NOTE: our own design under EHDS Art. 8 and 10: a restriction binds whoever reaches
        // the data for a provider, an opt-out the access services, a delegation a representative.
        let bindable = match kind {
            DirectiveKind::OptOut => caller == CallerKind::AccessService,
            DirectiveKind::Restriction => matches!(
                caller,
                CallerKind::Professional | CallerKind::System | CallerKind::AccessService
            ),
            DirectiveKind::Consent => caller != CallerKind::Subject,
            DirectiveKind::Delegation => caller == CallerKind::Representative,
        };
        if !bindable {
            return Pred::False;
        }
        let polarity = match kind_and_effect(directive).1 {
            Effect::Deny => Polarity::Deny,
            Effect::Permit => Polarity::Permit,
        };
        self.applies(directive, polarity, placed)
    }

    /// Whether `directive` applies to the request now: in force, naming the
    /// caller, the operation and the purpose.
    pub(super) fn applies(
        &self,
        directive: &AccessDirective,
        polarity: Polarity,
        placed: Option<&DirectiveLayout>,
    ) -> Pred {
        let asked = self.judged.asked;
        let in_force = match directive.time_validity.as_ref().map(validity) {
            None => Pred::True,
            Some(Validity::Within { from, until }) => match placed {
                Some(placed) if self.judged.scopes.layout().is_some() => {
                    let from = from.zip(placed.from).map(|(b, at)| (at, b.included));
                    let until = until.zip(placed.until).map(|(b, at)| (at, b.included));
                    if from.is_none() && until.is_none() {
                        Pred::True
                    } else {
                        Pred::Ehr(EhrFact::Within { from, until })
                    }
                }
                _ => Pred::of(within(from, until, self.judged.now)),
            },
            Some(Validity::Unreadable) => Pred::of(polarity.unreadable()),
        };
        let operation = directive.operations.as_deref().is_none_or(|operations| {
            operations.iter().any(|coded| {
                OperationClass::read_coded(coded)
                    .map_or(polarity.unreadable(), |op| op == asked.operation)
            })
        });
        let purpose = directive.purposes.as_deref().is_none_or(|purposes| {
            asked.purpose.map_or(polarity.unreadable(), |declared| {
                purposes
                    .iter()
                    .any(|coded| coded.defining_code.code_string == declared)
            })
        });
        let actor = if directive.applies_to.is_none() {
            Pred::True
        } else {
            self.actor(directive, placed)
        };
        Pred::all(vec![in_force, Pred::of(operation && purpose), actor])
    }

    /// Whether an actor scope of `directive` names the caller: every
    /// attribute a scope sets holds.
    pub(super) fn actor(
        &self,
        directive: &AccessDirective,
        placed: Option<&DirectiveLayout>,
    ) -> Pred {
        let caller = self.judged.caller;
        let class = self.judged.scopes.layout().is_some();
        let scopes =
            directive
                .applies_to
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(at, scope)| {
                    let party = match scope.party.as_ref() {
                        None => Pred::True,
                        Some(party) => {
                            match placed.and_then(|p| p.parties.get(at).copied().flatten()) {
                                Some(fact) if class => Pred::Ehr(EhrFact::Party(fact)),
                                _ => Pred::of(party_key(party).is_some_and(|key| {
                                    caller.party.as_ref() == Some(&key)
                                        || caller.acts_for.contains(&key)
                                })),
                            }
                        }
                    };
                    let role = scope.role.as_ref().is_none_or(|role| {
                        caller
                            .roles
                            .iter()
                            .any(|held| held.eq_ignore_ascii_case(&role.defining_code.code_string))
                    });
                    let relationship = scope.relationship.as_ref().is_none_or(|coded| {
                        match Relationship::read_coded(coded) {
                            Ok(Relationship::Care) => self.judged.ehr.cared,
                            Ok(Relationship::Representative | Relationship::Proxy) => {
                                caller.kind == CallerKind::Representative
                            }
                            Err(_) => false,
                        }
                    });
                    let kind = scope
                        .caller_kind
                        .as_ref()
                        .is_none_or(|coded| CallerKind::read_coded(coded) == Ok(caller.kind));
                    Pred::all(vec![party, Pred::of(role && relationship && kind)])
                })
                .collect();
        Pred::any(scopes)
    }

    /// The items `directive`, of `effect`, covers: its data less what its
    /// applying exceptions, of the opposite effect, cover.
    pub(super) fn cover(
        &self,
        directive: &AccessDirective,
        effect: Effect,
        placed: Option<&DirectiveLayout>,
    ) -> Pred {
        let (own, theirs) = match effect {
            Effect::Deny => (Polarity::Deny, Polarity::Permit),
            Effect::Permit => (Polarity::Permit, Polarity::Deny),
        };
        let scopes = self.judged.scopes;
        let excepted = directive
            .exceptions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(at, exception)| {
                let placed = placed.and_then(|p| p.exceptions.get(at));
                Pred::all(vec![
                    self.applies(exception, theirs, placed),
                    scopes.data(
                        exception.data.as_deref().unwrap_or_default(),
                        theirs,
                        placed.map(|p| p.data.as_slice()),
                    ),
                ])
            })
            .collect();
        Pred::all(vec![
            scopes.data(
                directive.data.as_deref().unwrap_or_default(),
                own,
                placed.map(|p| p.data.as_slice()),
            ),
            Pred::negate(Pred::any(excepted)),
        ])
    }
}
