// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decision procedure: deny overrides permit, in a fixed step order, with
//! one level of exceptions and one `refused-*` code per refusal.
//!
//! 1. **Opt-out** (EHDS Art. 10): an `opt_out` directive in force binds the
//!    access services; the EHR reads as absent (`refused-opt-out`), unless
//!    its `emergency_override` holds and an access service that may override
//!    declares an emergency purpose without acting for the subject
//!    (Art. 10(2)).
//! 2. **Restriction** (EHDS Art. 8): a `restriction` directive in force binds
//!    professionals, the systems that act for a provider and the access
//!    services; the data it covers read as absent (`refused-restricted`),
//!    unless its `emergency_override` holds and the break-glass role declares
//!    an emergency purpose (Art. 11(5)).
//! 3. **Consent deny**: any other deny directive that applies, with no
//!    exception that applies, refuses (`refused-consent-denied`), lifted the
//!    same way. A representative named by a `delegation` directive reads only
//!    what an applying delegation permit covers (EHDS Art. 4(2)).
//! 4. **Sensitivity**: data carrying a sensitivity, from an `ACCESS_LABEL` or
//!    the controller's classifications, that the caller's capabilities do not
//!    clear and no applying permit names (`refused-sensitivity`).
//! 5. **Care relationship**: where the policy requires one, a professional
//!    needs a live care relationship to the subject, a permit naming them, or
//!    a gate-keeper entry (`refused-no-care-relationship`; BASE Architecture
//!    Overview `master07-security.adoc` §Security Policy, "Time-limitation of
//!    access").
//! 6. **Purpose**: where the policy requires one, a declared purpose the
//!    caller's capabilities permit (`refused-purpose-missing`,
//!    `refused-purpose-not-permitted`).
//! 7. **Default disposition**: `open` admits, `restricted` admits a permit, a
//!    gate-keeper or a live care relationship, `consent_required` a permit or
//!    a gate-keeper (`refused-consent-required`); an EHR with no settings
//!    takes the server's default.
//!
//! The subject reading their own record skips every step (EHDS Art. 3), and
//! every other EHR refuses them (`refused-consent-required`): a subject
//! caller is the subject of the one EHR its patient context names. A
//! representative is held to steps 1 to 3 and skips 4 to 7. A declared
//! emergency by the break-glass role lifts steps 4 to 7.
//!
//! The same procedure decides a settings class ([`crate::access_eval::class`])
//! for a query: where it reads a value the class leaves to each EHR, the
//! verdict holds an atom of that fact ([`EhrFact`], [`crate::access_eval::scope::Held`])
//! in place of its answer, and a direct read, which knows every value,
//! answers each at once. No openEHR spec governs the procedure: our own
//! design (#5487, #5513, #5588).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, DirectiveKind, Effect, ForeignCode, ForeignKind, OperationClass,
    Refusal, Relationship, Term,
};

use crate::access_eval::class::{DirectiveLayout, keeper};
use crate::access_eval::scope::{EhrFact, Item, Polarity, Pred, Scopes, within};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CallerFacts;
use crate::service::ehr::access_projection::{Validity, validity};
use crate::service::ehr::access_rule::{SettingsCaller, party_key};
use crate::service::ehr::access_settings::EhrAccessSettings;

/// Who asks, as the procedure reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct Caller {
    /// The kind of caller (`access caller kind`).
    pub kind: CallerKind,
    /// The id of the `PERSON` or `AGENT` the caller is bound to.
    pub party: Option<String>,
    /// The EHR whose subject a [`CallerKind::Subject`] caller is: the EHR of
    /// its SMART App Launch patient context; `None` for every other caller,
    /// and for a subject caller with no such EHR, whom every EHR refuses.
    pub own_ehr: Option<EhrId>,
    /// The ids of the parties the caller acts for or is a member of: its
    /// provider, the `GROUP`s that provider is a member of, its care teams.
    pub acts_for: Vec<String>,
    /// The functional roles the caller holds, upper-case.
    pub roles: Vec<String>,
    /// The caller's clearances, permitted purposes and care teams.
    pub facts: CallerFacts,
    /// Whether the caller holds `admin_role`, the controller acting on the
    /// subject's instruction.
    pub administrator: bool,
    /// The caller as the gate-keeper rule reads it.
    pub settings: SettingsCaller,
}

/// What the caller asks to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked<'a> {
    /// The class of the operation.
    pub operation: OperationClass,
    /// The declared purpose of use, by its code.
    pub purpose: Option<&'a str>,
    /// What the caller declared in an emergency.
    pub emergency: Emergency,
}

/// An emergency as the caller declared it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Emergency {
    /// The request declares one of `[audit] emergency_purpose_codes`.
    pub declared: bool,
    /// The caller holds `[authz.rbac] emergency_access_role`.
    pub break_glass: bool,
    /// The caller is an access service that may override an opt-out for a
    /// vital interest, and does not act for the subject (EHDS Art. 10(2)).
    pub opt_out_override: bool,
}

impl Emergency {
    /// Whether the break-glass role declares an emergency purpose.
    #[must_use]
    pub const fn break_glass(self) -> bool {
        self.declared && self.break_glass
    }
}

/// What the controller's `ACCESS_POLICY` requires of clinical access.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy {
    /// `care_relationship_required`.
    pub care_required: bool,
    /// `relationship_grace`.
    pub grace: jiff::Span,
    /// `purpose_required`.
    pub purpose_required: bool,
}

impl Policy {
    /// The requirements of `policy`; none when no version is committed.
    #[must_use]
    pub fn of(
        policy: Option<
            &openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy,
        >,
    ) -> Self {
        let Some(policy) = policy else {
            return Self::default();
        };
        // NOTE: our own design, a grace that does not read as an ISO 8601 duration admits
        // no access after a relationship ends.
        let grace = policy
            .relationship_grace
            .as_ref()
            .and_then(|grace| grace.value.parse::<jiff::Span>().ok())
            .unwrap_or_default();
        Self {
            care_required: policy.care_relationship_required,
            grace,
            purpose_required: policy.purpose_required,
        }
    }
}

/// The EHR the decision is about.
#[derive(Debug, Clone, Copy)]
pub struct Ehr<'a> {
    /// Its current `EHR_ACCESS` settings; `None` when it carries none.
    pub settings: Option<&'a EhrAccessSettings>,
    /// The server-wide default of an EHR with no settings.
    pub server_default: AccessDefault,
    /// Whether the caller has a live care relationship to its subject.
    pub cared: bool,
    /// Whether the EHR is the [`Caller::own_ehr`] of a subject caller.
    pub own: bool,
}

/// What one step answers for the items it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// The item may be read.
    Admit,
    /// The item reads as absent (`404`), refused under the code.
    Absent(Refusal),
    /// The item is refused (`403`) under the code.
    Refused(Refusal),
}

/// One step's rule: the items it covers, what it answers for them, and the
/// directives it rests on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    /// The items covered.
    pub when: Pred,
    /// The answer.
    pub then: Outcome,
    /// The positions in the settings' `directives` of the directives the rule
    /// rests on; empty for a rule of the access policy or the default
    /// disposition.
    pub by: Vec<usize>,
}

/// What a verdict rests on beside its rules, which the access record names
/// (EHDS Annex II 3.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Grounds {
    /// The subject reads their own record (EHDS Art. 3).
    pub subject: bool,
    /// The positions in the settings' `directives` of the permits that bind
    /// the caller.
    pub permits: Vec<usize>,
    /// The positions of the denies an emergency lifted.
    pub lifted: Vec<usize>,
    /// The verdict reads a live care relationship to the subject.
    pub cared: bool,
    /// The verdict reads the caller as a gate-keeper or the controller.
    pub keeper: bool,
}

/// A directive or a step an emergency lifted, which marks the access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lifted {
    /// An opt-out (EHDS Art. 10(2)).
    OptOut,
    /// A restriction (EHDS Art. 11(5)).
    Restriction,
    /// A consent deny.
    Consent,
    /// Steps 4 to 7, by the break-glass role.
    Policy,
}

/// The decision for one caller on one EHR: the rules in step order, the
/// first that covers an item decides, and otherwise the default answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Verdict {
    /// The rules, in step order.
    pub rules: Vec<Rule>,
    /// The answer for an item no rule covers.
    pub otherwise: Outcome,
    /// What an emergency lifted.
    pub lifted: Vec<Lifted>,
    /// What the verdict rests on beside its rules.
    pub grounds: Grounds,
}

impl Verdict {
    /// A verdict admitting everything.
    #[must_use]
    pub fn admit() -> Self {
        Self {
            rules: Vec::new(),
            otherwise: Outcome::Admit,
            lifted: Vec::new(),
            grounds: Grounds::default(),
        }
    }

    /// The verdict without what it rests on, so verdicts that answer alike
    /// compare equal and one SQL condition serves them all.
    #[must_use]
    pub fn bare(mut self) -> Self {
        self.grounds = Grounds::default();
        for rule in &mut self.rules {
            rule.by.clear();
        }
        self
    }

    /// The rule that decides `item`; `None` when no rule covers it and the
    /// default answer stands.
    #[must_use]
    pub fn rule_for(&self, item: &Item<'_>) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.when.holds(item))
    }

    /// The answer for `item`.
    #[must_use]
    pub fn outcome(&self, item: &Item<'_>) -> Outcome {
        self.rule_for(item).map_or(self.otherwise, |rule| rule.then)
    }

    /// Whether the answer depends on the item.
    #[must_use]
    pub fn item_dependent(&self) -> bool {
        self.rules.iter().any(|rule| rule.when.constant().is_none())
    }

    /// Whether some item could be admitted.
    #[must_use]
    pub fn admits_any(&self) -> bool {
        self.reaches() != Pred::False
    }

    /// Where some item could be admitted, as a predicate over the facts of
    /// the EHR alone: constant for one EHR's own settings, and over its facts
    /// for a settings class.
    #[must_use]
    pub fn reaches(&self) -> Pred {
        let mut reaches = Pred::of(self.otherwise == Outcome::Admit);
        for rule in self.rules.iter().rev() {
            reaches = if rule.then == Outcome::Admit {
                Pred::any(vec![rule.when.maybe(), reaches])
            } else {
                Pred::all(vec![Pred::negate(rule.when.always()), reaches])
            };
        }
        reaches
    }

    /// The items admitted, as one predicate.
    #[must_use]
    pub fn admitted(&self) -> Pred {
        let mut admitted = if self.otherwise == Outcome::Admit {
            Pred::True
        } else {
            Pred::False
        };
        for rule in self.rules.iter().rev() {
            let then = if rule.then == Outcome::Admit {
                rule.when.clone()
            } else {
                Pred::False
            };
            admitted = Pred::any(vec![
                then,
                Pred::all(vec![Pred::negate(rule.when.clone()), admitted]),
            ]);
        }
        admitted
    }
}

/// The inputs of one decision.
#[derive(Debug, Clone, Copy)]
pub struct Judged<'a> {
    /// Who asks.
    pub caller: &'a Caller,
    /// What they ask.
    pub asked: Asked<'a>,
    /// What the controller requires.
    pub policy: Policy,
    /// The EHR.
    pub ehr: Ehr<'a>,
    /// The EHR's labels and the controller's classifications.
    pub scopes: &'a Scopes,
    /// The moment of the request.
    pub now: jiff::Timestamp,
}

/// Decides `judged` by the procedure of this module.
#[must_use]
pub fn decide(judged: &Judged<'_>) -> Verdict {
    let mut verdict = Verdict::admit();
    let caller = judged.caller;
    // NOTE: EHDS Art. 3, the subject reads their own record; only the access delay holds it.
    if caller.kind == CallerKind::Subject {
        if judged.ehr.own {
            verdict.grounds.subject = true;
        } else {
            // NOTE: our own design (#5586), SMART master08 §Resource Scopes confines a patient
            // context to "data within that patient's EHR", so every other EHR refuses it.
            verdict.otherwise = Outcome::Refused(Refusal::ConsentRequired);
        }
        return verdict;
    }
    let (directives, held_closed) = match judged.ehr.settings {
        Some(EhrAccessSettings::Directives(settings)) => {
            (settings.directives.as_deref().unwrap_or_default(), false)
        }
        Some(EhrAccessSettings::HeldClosed) => (&[][..], true),
        None => (&[][..], false),
    };
    let read = Reader { judged };
    let placed = |at: usize| judged.scopes.layout().and_then(|l| l.directives.get(at));
    denials(&read, directives, &mut verdict);
    if caller.kind == CallerKind::Representative {
        delegation(&read, directives, &mut verdict);
        return verdict;
    }
    if judged.asked.emergency.break_glass() {
        verdict.lifted.push(Lifted::Policy);
        return verdict;
    }
    let permits: Vec<Permit<'_>> = directives
        .iter()
        .enumerate()
        .filter(|(_, d)| kind_and_effect(d) == (DirectiveKind::Consent, Effect::Permit))
        .filter_map(|(at, d)| {
            let binds = read.binds(DirectiveKind::Consent, d, placed(at));
            (binds != Pred::False).then(|| Permit {
                at,
                directive: d,
                certain: binds == Pred::True,
                cover: Pred::all(vec![binds, read.cover(d, Effect::Permit, placed(at))]),
            })
        })
        .collect();
    verdict.grounds.permits = permits
        .iter()
        .filter(|permit| permit.certain)
        .map(|permit| permit.at)
        .collect();
    sensitivity(&read, &permits, &mut verdict);
    let keeper = Pred::any(vec![Pred::of(caller.administrator), read.keeper()]);
    let live = judged.ehr.cared && !held_closed;
    let default = judged
        .ehr
        .settings
        .map_or(judged.ehr.server_default, EhrAccessSettings::default_access);
    let care_read = judged.policy.care_required && caller.kind == CallerKind::Professional;
    if care_read || default != AccessDefault::Open {
        verdict.grounds.cared = live;
        verdict.grounds.keeper = keeper == Pred::True;
    }
    // Step 5: a care relationship.
    let uncared = Pred::negate(Pred::any(vec![Pred::of(live), keeper.clone()]));
    if care_read && uncared != Pred::False {
        let naming = permits
            .iter()
            .filter(|permit| permit.directive.applies_to.is_some())
            .map(|permit| permit.cover.clone())
            .collect();
        verdict.rules.push(Rule {
            when: Pred::all(vec![uncared, Pred::negate(Pred::any(naming))]),
            then: Outcome::Refused(Refusal::NoCareRelationship),
            by: Vec::new(),
        });
    }
    // Step 6: a purpose.
    if judged.policy.purpose_required {
        let refused = match judged.asked.purpose {
            None => Some(Refusal::PurposeMissing),
            Some(purpose) if !caller.facts.purposes().iter().any(|p| p.code == purpose) => {
                Some(Refusal::PurposeNotPermitted)
            }
            Some(_) => None,
        };
        if let Some(refusal) = refused {
            verdict.rules.push(Rule {
                when: Pred::True,
                then: Outcome::Refused(refusal),
                by: Vec::new(),
            });
        }
    }
    // Step 7: the default disposition.
    let admitted = match default {
        AccessDefault::Open => Pred::True,
        AccessDefault::Restricted => Pred::any(vec![keeper, Pred::of(live)]),
        AccessDefault::ConsentRequired => keeper,
    };
    disposition(admitted, permits, &mut verdict);
    verdict
}

/// Step 7: what the default disposition `admitted` does not admit, a permit
/// may.
fn disposition(admitted: Pred, permits: Vec<Permit<'_>>, verdict: &mut Verdict) {
    if admitted == Pred::True {
        return;
    }
    if admitted != Pred::False {
        verdict.rules.push(Rule {
            when: admitted,
            then: Outcome::Admit,
            by: Vec::new(),
        });
    }
    verdict.rules.push(Rule {
        by: permits.iter().map(|permit| permit.at).collect(),
        when: Pred::any(permits.into_iter().map(|permit| permit.cover).collect()),
        then: Outcome::Admit,
    });
    verdict.otherwise = Outcome::Refused(Refusal::ConsentRequired);
}

/// Steps 1 to 3: every deny directive that binds the caller, in step order
/// (opt-out, restriction, consent), or the mark of an emergency that lifts
/// it.
fn denials(read: &Reader<'_, '_>, directives: &[AccessDirective], verdict: &mut Verdict) {
    let emergency = read.judged.asked.emergency;
    for (at, directive) in directives.iter().enumerate() {
        let placed = read.placed(at);
        let (kind, effect) = kind_and_effect(directive);
        if effect != Effect::Deny {
            continue;
        }
        let binds = read.binds(kind, directive, placed);
        if binds == Pred::False {
            continue;
        }
        let (refusal, lifted, lifts) = match kind {
            DirectiveKind::OptOut => (
                Outcome::Absent(Refusal::OptOut),
                Lifted::OptOut,
                emergency.declared && emergency.opt_out_override,
            ),
            DirectiveKind::Restriction => (
                Outcome::Absent(Refusal::Restricted),
                Lifted::Restriction,
                emergency.break_glass(),
            ),
            DirectiveKind::Consent | DirectiveKind::Delegation => (
                Outcome::Refused(Refusal::ConsentDenied),
                Lifted::Consent,
                emergency.break_glass(),
            ),
        };
        if directive.emergency_override && lifts {
            if binds == Pred::True {
                verdict.lifted.push(lifted);
                verdict.grounds.lifted.push(at);
            }
        } else {
            verdict.rules.push(Rule {
                when: Pred::all(vec![binds, read.cover(directive, Effect::Deny, placed)]),
                then: refusal,
                by: vec![at],
            });
        }
    }
    verdict.rules.sort_by_key(|rule| step(rule.then));
}

/// The step a deny of steps 1 to 3 belongs to.
const fn step(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Absent(Refusal::OptOut) => 1,
        Outcome::Absent(_) => 2,
        _ => 3,
    }
}

/// Step 3 for a representative: one a `delegation` directive names reads
/// only what an applying delegation permit covers (EHDS Art. 4(2)).
fn delegation(read: &Reader<'_, '_>, directives: &[AccessDirective], verdict: &mut Verdict) {
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

/// Step 4: data whose sensitivity the caller is not cleared for, unless an
/// applying permit names that sensitivity.
fn sensitivity(read: &Reader<'_, '_>, permits: &[Permit<'_>], verdict: &mut Verdict) {
    let scopes = read.judged.scopes;
    for code in scopes.sensitivity_codes() {
        if read.judged.caller.facts.clears(&code) {
            continue;
        }
        let named = permits
            .iter()
            .filter(|permit| names_sensitivity(permit.directive, &code))
            .map(|permit| permit.cover.clone())
            .collect();
        verdict.rules.push(Rule {
            when: Pred::all(vec![
                scopes.sensitive(&code),
                Pred::negate(Pred::any(named)),
            ]),
            then: Outcome::Refused(Refusal::Sensitivity),
            by: Vec::new(),
        });
    }
}

/// A consent permit that may bind the caller: its position in the settings'
/// `directives`, the directive, whether it binds the caller whatever the
/// facts, and the items it covers where it binds.
struct Permit<'a> {
    at: usize,
    directive: &'a AccessDirective,
    certain: bool,
    cover: Pred,
}

/// Whether a data scope of `directive` names the sensitivity `code`.
fn names_sensitivity(directive: &AccessDirective, code: &ForeignCode) -> bool {
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
fn kind_and_effect(directive: &AccessDirective) -> (DirectiveKind, Effect) {
    (
        DirectiveKind::read_coded(&directive.kind).unwrap_or(DirectiveKind::Consent),
        Effect::read_coded(&directive.effect).unwrap_or(Effect::Deny),
    )
}

/// Reads directives for one decision.
struct Reader<'j, 'a> {
    judged: &'j Judged<'a>,
}

impl Reader<'_, '_> {
    /// Where the facts of the directive at `at` sit, for a settings class.
    fn placed(&self, at: usize) -> Option<&DirectiveLayout> {
        self.judged
            .scopes
            .layout()
            .and_then(|layout| layout.directives.get(at))
    }

    /// Whether the caller is a gate-keeper of the EHR.
    fn keeper(&self) -> Pred {
        let caller = &self.judged.caller.settings;
        match self.judged.scopes.layout() {
            Some(layout) => keeper(layout, caller),
            None => Pred::of(caller.is_gate_keeper(self.judged.ehr.settings)),
        }
    }

    /// Whether a directive of `kind` binds the caller: the kind binds the
    /// caller's kind, its `applies_to` names the caller, and it applies to
    /// the request now.
    fn binds(
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
    fn applies(
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
    fn actor(&self, directive: &AccessDirective, placed: Option<&DirectiveLayout>) -> Pred {
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
    fn cover(
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
