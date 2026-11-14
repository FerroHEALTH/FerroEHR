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
//! The subject reading their own record skips every step (EHDS Art. 3); a
//! representative is held to steps 1 to 3 and skips 4 to 7. A declared
//! emergency by the break-glass role lifts steps 4 to 7. No openEHR spec
//! governs the procedure: our own design (#5487, #5513).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, DirectiveKind, Effect, ForeignCode, ForeignKind, OperationClass,
    Refusal, Relationship, Term,
};

use crate::access_eval::scope::{Item, Polarity, Pred, Scopes, within};
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
    /// The ids of the parties the caller acts for or is a member of: its
    /// provider, its care teams.
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
        policy: Option<&openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy>,
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

/// One step's rule: the items it covers and what it answers for them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    /// The items covered.
    pub when: Pred,
    /// The answer.
    pub then: Outcome,
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
}

impl Verdict {
    /// A verdict admitting everything.
    #[must_use]
    pub fn admit() -> Self {
        Self {
            rules: Vec::new(),
            otherwise: Outcome::Admit,
            lifted: Vec::new(),
        }
    }

    /// The answer for `item`.
    #[must_use]
    pub fn outcome(&self, item: &Item<'_>) -> Outcome {
        self.rules
            .iter()
            .find(|rule| rule.when.holds(item))
            .map_or(self.otherwise, |rule| rule.then)
    }

    /// Whether the answer depends on the item.
    #[must_use]
    pub fn item_dependent(&self) -> bool {
        self.rules.iter().any(|rule| rule.when.constant().is_none())
    }

    /// Whether some item could be admitted.
    #[must_use]
    pub fn admits_any(&self) -> bool {
        for rule in &self.rules {
            match rule.when.constant() {
                Some(true) => return rule.then == Outcome::Admit,
                Some(false) => {}
                None if rule.then == Outcome::Admit => return true,
                None => {}
            }
        }
        self.otherwise == Outcome::Admit
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
                Pred::all(vec![Pred::not(rule.when.clone()), admitted]),
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
    let emergency = judged.asked.emergency;
    for directive in directives {
        let (kind, effect) = read.kind_and_effect(directive);
        if effect != Effect::Deny || !read.binds(kind, directive) {
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
            verdict.lifted.push(lifted);
        } else {
            verdict.rules.push(Rule {
                when: read.cover(directive, Effect::Deny),
                then: refusal,
            });
        }
    }
    // Steps 1 to 3 put their rules in step order: opt-out, restriction, consent.
    verdict.rules.sort_by_key(|rule| step(rule.then));
    if caller.kind == CallerKind::Representative {
        delegation(&read, directives, &mut verdict);
        return verdict;
    }
    if emergency.break_glass() {
        verdict.lifted.push(Lifted::Policy);
        return verdict;
    }
    let permits: Vec<(&AccessDirective, Pred)> = directives
        .iter()
        .filter(|d| read.kind_and_effect(d) == (DirectiveKind::Consent, Effect::Permit))
        .filter(|d| read.binds(DirectiveKind::Consent, d))
        .map(|d| (d, read.cover(d, Effect::Permit)))
        .collect();
    sensitivity(&read, &permits, &mut verdict);
    let keeper = caller.administrator || caller.settings.is_gate_keeper(judged.ehr.settings);
    let live = judged.ehr.cared && !held_closed;
    // Step 5: a care relationship.
    if judged.policy.care_required && caller.kind == CallerKind::Professional && !(live || keeper)
    {
        let naming = permits
            .iter()
            .filter(|(d, _)| d.applies_to.is_some())
            .map(|(_, cover)| cover.clone())
            .collect();
        verdict.rules.push(Rule {
            when: Pred::not(Pred::any(naming)),
            then: Outcome::Refused(Refusal::NoCareRelationship),
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
            });
        }
    }
    // Step 7: the default disposition.
    let default = judged
        .ehr
        .settings
        .map_or(judged.ehr.server_default, EhrAccessSettings::default_access);
    let admitted = match default {
        AccessDefault::Open => true,
        AccessDefault::Restricted => keeper || live,
        AccessDefault::ConsentRequired => keeper,
    };
    if !admitted {
        verdict.rules.push(Rule {
            when: Pred::any(permits.into_iter().map(|(_, cover)| cover).collect()),
            then: Outcome::Admit,
        });
        verdict.otherwise = Outcome::Refused(Refusal::ConsentRequired);
    }
    verdict
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
    let naming: Vec<&AccessDirective> = directives
        .iter()
        .filter(|d| read.kind_and_effect(d) == (DirectiveKind::Delegation, Effect::Permit))
        .filter(|d| d.applies_to.is_some() && read.actor(d))
        .collect();
    if naming.is_empty() {
        return;
    }
    let covered = naming
        .iter()
        .filter(|d| read.applies(d, Polarity::Permit))
        .map(|d| read.cover(d, Effect::Permit))
        .collect();
    verdict.rules.push(Rule {
        when: Pred::not(Pred::any(covered)),
        then: Outcome::Refused(Refusal::ConsentDenied),
    });
}

/// Step 4: data whose sensitivity the caller is not cleared for, unless an
/// applying permit names that sensitivity.
fn sensitivity(read: &Reader<'_, '_>, permits: &[(&AccessDirective, Pred)], verdict: &mut Verdict) {
    let scopes = read.judged.scopes;
    for code in scopes.sensitivity_codes() {
        if read.judged.caller.facts.clears(&code) {
            continue;
        }
        let named = permits
            .iter()
            .filter(|(d, _)| names_sensitivity(d, &code))
            .map(|(_, cover)| cover.clone())
            .collect();
        verdict.rules.push(Rule {
            when: Pred::all(vec![scopes.sensitive(&code), Pred::not(Pred::any(named))]),
            then: Outcome::Refused(Refusal::Sensitivity),
        });
    }
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

/// Reads directives for one decision.
struct Reader<'j, 'a> {
    judged: &'j Judged<'a>,
}

impl Reader<'_, '_> {
    /// The kind and effect of `directive`; settings that hold a code outside
    /// its group are held closed before they reach the evaluator, so one
    /// that does not read is read as the consent deny it would refuse as.
    fn kind_and_effect(&self, directive: &AccessDirective) -> (DirectiveKind, Effect) {
        (
            DirectiveKind::read_coded(&directive.kind).unwrap_or(DirectiveKind::Consent),
            Effect::read_coded(&directive.effect).unwrap_or(Effect::Deny),
        )
    }

    /// Whether a directive of `kind` binds the caller: the kind binds the
    /// caller's kind, its `applies_to` names the caller, and it applies to
    /// the request now.
    fn binds(&self, kind: DirectiveKind, directive: &AccessDirective) -> bool {
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
        let polarity = match self.kind_and_effect(directive).1 {
            Effect::Deny => Polarity::Deny,
            Effect::Permit => Polarity::Permit,
        };
        bindable && self.applies(directive, polarity)
    }

    /// Whether `directive` applies to the request now: in force, naming the
    /// caller, the operation and the purpose.
    fn applies(&self, directive: &AccessDirective, polarity: Polarity) -> bool {
        let asked = self.judged.asked;
        let in_force = match directive.time_validity.as_ref().map(validity) {
            None => true,
            Some(Validity::Within { from, until }) => within(from, until, self.judged.now),
            Some(Validity::Unreadable) => polarity.unreadable(),
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
        in_force && operation && purpose && (directive.applies_to.is_none() || self.actor(directive))
    }

    /// Whether an actor scope of `directive` names the caller: every
    /// attribute a scope sets holds.
    fn actor(&self, directive: &AccessDirective) -> bool {
        let caller = self.judged.caller;
        directive
            .applies_to
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|scope| {
                let party = scope.party.as_ref().is_none_or(|party| {
                    party_key(party).is_some_and(|key| {
                        caller.party.as_ref() == Some(&key) || caller.acts_for.contains(&key)
                    })
                });
                let role = scope.role.as_ref().is_none_or(|role| {
                    caller
                        .roles
                        .iter()
                        .any(|held| held.eq_ignore_ascii_case(&role.defining_code.code_string))
                });
                let relationship =
                    scope
                        .relationship
                        .as_ref()
                        .is_none_or(|coded| match Relationship::read_coded(coded) {
                            Ok(Relationship::Care) => self.judged.ehr.cared,
                            Ok(Relationship::Representative | Relationship::Proxy) => {
                                caller.kind == CallerKind::Representative
                            }
                            Err(_) => false,
                        });
                let kind = scope
                    .caller_kind
                    .as_ref()
                    .is_none_or(|coded| CallerKind::read_coded(coded) == Ok(caller.kind));
                party && role && relationship && kind
            })
    }

    /// The items `directive`, of `effect`, covers: its data less what its
    /// applying exceptions, of the opposite effect, cover.
    fn cover(&self, directive: &AccessDirective, effect: Effect) -> Pred {
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
            .filter(|exception| self.applies(exception, theirs))
            .map(|exception| scopes.data(exception.data.as_deref().unwrap_or_default(), theirs))
            .collect();
        Pred::all(vec![
            scopes.data(directive.data.as_deref().unwrap_or_default(), own),
            Pred::not(Pred::any(excepted)),
        ])
    }
}
