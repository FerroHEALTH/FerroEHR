// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The decision procedure: deny overrides permit, in a fixed step order, with
//! one level of exceptions and one `refused-*` code per refusal.
//!
//! 1. **Opt-out** (EHDS Art. 10): an `opt_out` directive in force binds the
//!    access services; the EHR reads as absent (`refused-opt-out`), unless
//!    its `emergency_override` holds and an access service that may override
//!    invokes an emergency justification without acting for the subject
//!    (Art. 10(2)).
//! 2. **Restriction** (EHDS Art. 8): a `restriction` directive in force binds
//!    professionals, the systems that act for a provider and the access
//!    services; the data it covers read as absent (`refused-restricted`),
//!    unless its `emergency_override` holds and the caller invokes an
//!    emergency justification (Art. 11(5)).
//! 3. **Consent deny**: any other deny directive that applies, with no
//!    exception that applies, refuses (`refused-consent-denied`), lifted the
//!    same way. A representative named by a `delegation` directive reads only
//!    what an applying delegation permit covers (EHDS Art. 4(2)).
//! 4. **Purpose**: where the policy requires one, a request without a
//!    declared purpose is refused (`refused-purpose-missing`) before any rule
//!    is read; which purposes a caller may declare is a rule of step 5. An
//!    emergency justification invoked without a reason is refused here
//!    (`refused-reason-missing`), before any datum is read.
//! 5. **Controller rule** (EHDS Art. 11(3)): a deny rule of the controller's
//!    `ACCESS_POLICY` that applies, with no exception that applies, refuses
//!    whatever the subject's directives permit (`refused-policy-denied`),
//!    unless its `emergency_override` holds and the caller invokes an
//!    emergency justification. A rule that sets purposes applies only to a
//!    request that declares one of them.
//! 6. **Sensitivity**: data carrying a sensitivity, from an `ACCESS_LABEL` or
//!    the controller's classifications, that no applying permit directive or
//!    permit rule names (`refused-sensitivity`). **Confidentiality**: data
//!    whose level, the highest of the EHR's marking, a label's and a
//!    classification's, is above `N` and that no applying permit clears by
//!    naming that level or a higher one (`refused-confidentiality`, answered
//!    as absent at a level the policy conceals).
//! 7. **Care relationship**: where the policy requires one, a professional
//!    needs a live care relationship to the subject, a permit naming them, or
//!    a gate-keeper entry (`refused-no-care-relationship`; BASE Architecture
//!    Overview `master07-security.adoc` §Security Policy, "Time-limitation of
//!    access"). A justification of the policy the caller invokes stands in
//!    for the relationship with a reason, and is refused
//!    `refused-reason-missing` without one
//!    ([`crate::access_eval::justification`]).
//! 8. **Default disposition**: `open` admits, `restricted` admits a permit, a
//!    gate-keeper or a live care relationship, `consent_required` a permit or
//!    a gate-keeper (`refused-consent-required`); an EHR with no settings
//!    takes the server's default.
//!
//! Before step 1 an **exclusion** of the controller
//! (`CONFIDENTIALITY_SETTINGS.exclusions`) that applies refuses the caller it
//! names, a representative and a gate-keeper included (`refused-excluded`),
//! lifted only for a caller who is no representative where its
//! `emergency_override` holds. Before the controller's rules of step 5 a
//! professional in **conflict** with the subject (its `PERSON` is the
//! subject's, or a `related_to` relationship in force joins the two) is
//! refused (`refused-conflict`) unless a conflict rule of the policy allows
//! it; kinship refuses and never grants.
//!
//! The subject reading their own record skips every step (EHDS Art. 3): the
//! patient's own right of access is not the controller's to deny. A caller
//! bound to the subject's `PERSON` reads it as the subject unless it performs
//! a professional `ROLE`, which makes its read a professional's. A
//! confined caller is refused every EHR but its own before step 1 (step 0):
//! the subject, or one whose grants are `patient/` grants alone, every EHR
//! but the one its patient context names (`refused-not-subject`), and a
//! representative every EHR but the one whose subject its recorded
//! representation `PARTY_RELATIONSHIP` names (`refused-not-represented`,
//! EHDS Art. 4(2)). With step 0 as well, a caller admitted under an agreement
//! that conditions its reads on a consent decision, whose credential names no
//! reference to it, is refused every read, query and export
//! (`refused-consent-reference-missing`). On its own EHR a confined professional takes every step,
//! and a representative is held to steps 1 to 3 and 5, the `delegation`
//! directives of that EHR's `EHR_ACCESS` included, and skips 4 and 6 to 8: a
//! controller rule can keep, say, a minor's confidential data from a parent.
//! An emergency justification invoked with a reason lifts steps 6 to 8.
//!
//! A directive permit decides only through steps 6, 7 and 8; a permit rule
//! only through step 6, and a rule's permit exception only inside its deny.
//!
//! The same procedure decides a settings class ([`crate::access_eval::class`])
//! for a query: where it reads a value the class leaves to each EHR, the
//! verdict holds an atom of that fact ([`crate::access_eval::scope::EhrFact`], [`crate::access_eval::scope::Held`])
//! in place of its answer, and a direct read, which knows every value,
//! answers each at once. No openEHR spec governs the procedure: our own
//! design (#5487, #5513, #5588, #5621, #5737, #6077).

use openehr_rm::draft::access_control::access_control::access_conflict_rule::AccessConflictRule;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_justification::AccessJustification;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, Confidentiality, Conflict, DirectiveKind, Effect, ForeignCode,
    OperationClass, Refusal,
};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_eval::care_tie::{Cared, Tie, care_needs, granting};
use crate::access_eval::directive::{Permit, Reader, kind_and_effect, names_sensitivity};
use crate::access_eval::justification::{Invoked, Justified};
use crate::access_eval::scope::{Item, Pred, Scopes};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::facts::CallerFacts;
use crate::service::ehr::access_rule::SettingsCaller;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::versioning::episode::EpisodeKey;

mod confidentiality;
mod covering;
mod narrowing;

/// Who asks, as the procedure reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct Caller {
    /// The kind of caller (`access caller kind`).
    pub kind: CallerKind,
    /// The id of the `PERSON` or `AGENT` the caller is bound to.
    pub party: Option<String>,
    /// The EHRs a [`Self::confined`] caller reaches: the EHR of its SMART
    /// App Launch patient context, or for a representative each EHR whose
    /// subject reference names the `PERSON` its recorded representation
    /// names, which after an identity merge can be more than one; empty for
    /// every other caller, and for a confined caller with no such EHR, whom
    /// every EHR refuses.
    pub own_ehrs: Vec<EhrId>,
    /// Whether the caller's grants on the EHR's data are `patient/` grants
    /// alone, which reach only "data within that patient's EHR" (SMART on
    /// openEHR `master08` §Resource Scopes).
    pub patient_grants_only: bool,
    /// Whether the caller is an access service that may lift an opt-out for
    /// a vital interest, and does not act for the subject (EHDS Art. 10(2)).
    pub opt_out_override: bool,
    /// The ids of the parties the caller acts for or is a member of: its
    /// provider, the `GROUP`s that provider is a member of, its care teams.
    pub acts_for: Vec<String>,
    /// The roles the caller performs and its care teams.
    pub facts: CallerFacts,
    /// Whether the caller holds `admin_role`, the controller acting on the
    /// subject's instruction.
    pub administrator: bool,
    /// The caller as the gate-keeper rule reads it.
    pub settings: SettingsCaller,
    /// Whether the agreement the caller was admitted under conditions its
    /// reads on a consent decision, and its credential names no reference to
    /// one.
    pub lacks_consent_reference: bool,
}

impl Caller {
    /// Whether the caller reaches only [`Self::own_ehrs`]: a subject caller,
    /// a representative, and a caller whose grants are `patient/` grants
    /// alone.
    #[must_use]
    pub fn confined(&self) -> bool {
        matches!(self.kind, CallerKind::Subject | CallerKind::Representative)
            || self.patient_grants_only
    }

    /// Whether the caller reaches a record as a professional: a caller of
    /// kind `professional` that performs a professional `ROLE`. Bound to the
    /// subject's `PERSON`, such a caller is in conflict with the subject, and
    /// any other reads its own record as the subject (#6106, #6077).
    #[must_use]
    pub fn professional_path(&self) -> bool {
        self.kind == CallerKind::Professional && self.facts.performs_a_role()
    }

    /// The refusal of a [`Self::confined`] caller on an EHR other than its
    /// own (step 0).
    #[must_use]
    pub fn not_own(&self) -> Refusal {
        if self.kind == CallerKind::Representative {
            Refusal::NotRepresented
        } else {
            Refusal::NotSubject
        }
    }
}

/// What the caller asks to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked<'a> {
    /// The class of the operation.
    pub operation: OperationClass,
    /// The declared purpose of use, by its code.
    pub purpose: Option<&'a str>,
    /// Whether the request states a reason for a justified access.
    pub reasoned: bool,
    /// The episodes of care the request is narrowed to, sorted; `None` when
    /// it is not narrowed.
    pub episodes: Option<&'a [EpisodeKey]>,
}

/// What the controller's `ACCESS_POLICY` requires of clinical access.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy<'a> {
    /// `care_relationship_required`.
    pub care_required: bool,
    /// `relationship_grace`.
    pub grace: jiff::Span,
    /// `purpose_required`.
    pub purpose_required: bool,
    /// Whether `self_declared_care` is set, which turns the `self_declared`
    /// path on (draft decision procedure, Holding a Care Relationship).
    pub self_declared: bool,
    /// `care_purposes`: the purposes of use for which a care relationship is
    /// a ground; `None` when void.
    pub care_purposes: Option<&'a [DvCodedText]>,
    /// The levels of `concealed_confidentialities`, whose refused records
    /// answer as absent: `R`, then `V`.
    pub concealed: [bool; 2],
    /// `episode_shared`: the data every episode covers.
    pub episode_shared: &'a [AccessDataScope],
    /// `justifications`: the grounds of an access outside the care
    /// relationships.
    pub justifications: &'a [AccessJustification],
}

impl<'a> Policy<'a> {
    /// The requirements of `policy`; none when no version is committed.
    #[must_use]
    pub fn of(policy: Option<&'a AccessPolicy>) -> Self {
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
        let concealed = crate::access_policy::care::CareFacts::of(Some(policy)).concealed();
        Self {
            care_required: policy.care_relationship_required,
            grace,
            purpose_required: policy.purpose_required,
            self_declared: policy.self_declared_care.is_some(),
            care_purposes: policy.care_purposes.as_deref(),
            concealed: [
                concealed.contains(&Confidentiality::Restricted),
                concealed.contains(&Confidentiality::VeryRestricted),
            ],
            episode_shared: policy.episode_shared.as_deref().unwrap_or_default(),
            justifications: policy.justifications.as_deref().unwrap_or_default(),
        }
    }

    /// Whether a care relationship is a ground for an access that declares
    /// `purpose`: one that declares none, or one of `care_purposes` (draft
    /// decision procedure, Terms: a live care relationship).
    #[must_use]
    pub fn admits_care(&self, purpose: Option<&str>) -> bool {
        purpose.is_none_or(|purpose| match self.care_purposes {
            Some(codes) => codes
                .iter()
                .any(|coded| coded.defining_code.code_string == purpose),
            // NOTE: draft BMM ACCESS_POLICY.care_purposes, void means TREAT, ETREAT and CAREMGT.
            None => ["TREAT", "ETREAT", "CAREMGT"].contains(&purpose),
        })
    }

    /// Whether a record refused at `level` answers as absent.
    #[must_use]
    pub const fn conceals(self, level: Confidentiality) -> bool {
        match level {
            Confidentiality::Normal => false,
            Confidentiality::Restricted => self.concealed[0],
            Confidentiality::VeryRestricted => self.concealed[1],
        }
    }
}

/// The controller's rules of `policy` (`ACCESS_POLICY.rules`); none when no
/// version is committed.
#[must_use]
pub fn rules_of(policy: Option<&AccessPolicy>) -> &[AccessDirective] {
    policy
        .and_then(|policy| policy.rules.as_deref())
        .unwrap_or_default()
}

/// The EHR the decision is about.
#[derive(Debug, Clone, Copy)]
pub struct Ehr<'a> {
    /// Its current `EHR_ACCESS` settings; `None` when it carries none.
    pub settings: Option<&'a EhrAccessSettings>,
    /// The server-wide default of an EHR with no settings.
    pub server_default: AccessDefault,
    /// The caller's live care relationships to its subject, stored or
    /// attested by its credential.
    pub care: &'a Cared,
    /// Whether the EHR is one of the [`Caller::own_ehrs`] of a confined caller.
    pub own: bool,
    /// Whether the EHR's `EHR_STATUS.subject` names the `PERSON` the
    /// caller's authenticated identity is bound to ([`Caller::party`]), which
    /// makes the caller the record's person on it.
    pub subject: bool,
    /// The kinships of the `related_to` relationships in force that join the
    /// caller to the subject's `PERSON`; empty when none does.
    pub kinships: &'a [ForeignCode],
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
    /// The positions of the directives the rule rests on: in the settings'
    /// `directives`, or for step 5 in the policy's `rules`; empty for a
    /// requirement of the access policy or the default disposition.
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
    /// The positions of the denies an emergency lifted, each with the items
    /// it covers, so the access record names it only for an item it lifted.
    pub lifted: Vec<(usize, Pred)>,
    /// The positions in the policy's `rules` of the deny rules an emergency
    /// lifted, each with the items it covers, so the access record names it
    /// only for an item it lifted.
    pub lifted_rules: Vec<(usize, Pred)>,
    /// The verdict reads a live care relationship to the subject.
    pub cared: bool,
    /// The relationship the access log names, where the verdict reads one
    /// and its ties are known.
    pub granting: Option<Box<Tie>>,
    /// The verdict reads the caller as a gate-keeper or the controller.
    pub keeper: bool,
    /// The positions in the settings' `exclusions` of the exclusions an
    /// emergency lifted, each with the items it covers.
    pub lifted_exclusions: Vec<(usize, Pred)>,
    /// The conflicts a conflict rule of the policy allowed, which mark the
    /// access (draft decision procedure step 5).
    pub conflicts: Vec<Conflict>,
    /// The justification the verdict rests on, with the data it admitted.
    pub justification: Option<Justified>,
}

/// A directive or a step an emergency justification lifted, which marks the
/// access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lifted {
    /// An opt-out (EHDS Art. 10(2)).
    OptOut,
    /// A restriction (EHDS Art. 11(5)).
    Restriction,
    /// A consent deny.
    Consent,
    /// A deny rule of the controller's policy.
    Rule,
    /// Steps 6 to 8.
    Policy,
    /// An exclusion of the controller.
    Exclusion,
}

/// The decision for one caller on one EHR: the rules in step order, the
/// first that covers an item decides, and otherwise the default answer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Verdict {
    /// The rules, in step order.
    pub rules: Vec<Rule>,
    /// The answer for an item no rule covers.
    pub otherwise: Outcome,
    /// What an emergency justification lifted.
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
    pub policy: Policy<'a>,
    /// The controller's rules (`ACCESS_POLICY.rules`).
    pub rules: &'a [AccessDirective],
    /// The conflicts the controller allows through the professional path
    /// (`ACCESS_POLICY.conflict_rules`); empty allows none.
    pub conflict_rules: &'a [AccessConflictRule],
    /// The EHR.
    pub ehr: Ehr<'a>,
    /// The EHR's labels and the controller's classifications.
    pub scopes: &'a Scopes,
    /// The moment of the request.
    pub now: jiff::Timestamp,
}

/// Decides `judged` by the procedure of this module, narrowed to the
/// episodes the request names.
#[must_use]
pub fn decide(judged: &Judged<'_>) -> Verdict {
    let verdict = procedure(judged);
    match judged.asked.episodes {
        Some(episodes) => narrowing::narrowed(verdict, narrowing::within(judged, episodes)),
        None => verdict,
    }
}

/// The verdict of `judged` by the steps of the procedure.
fn procedure(judged: &Judged<'_>) -> Verdict {
    let mut verdict = Verdict::admit();
    let caller = judged.caller;
    if let Some(refusal) = crate::access_eval::exclusion::standing(judged) {
        verdict.otherwise = Outcome::Refused(refusal);
        return verdict;
    }
    // NOTE: EHDS Art. 3, the subject reads their own record; only the access delay holds it.
    // The party binding is the one source of who that is (#6106), and a professional ROLE
    // makes the read a professional's, held to the conflict step (#6077).
    if caller.kind == CallerKind::Subject || (judged.ehr.subject && !caller.professional_path()) {
        verdict.grounds.subject = true;
        return verdict;
    }
    let (directives, exclusions, held_closed) = lists(judged.ehr.settings);
    let read = Reader { judged };
    let placed = |at: usize| judged.scopes.layout().and_then(|l| l.directives.get(at));
    let invoked = crate::access_eval::justification::invoked(&read);
    let lifts = invoked.is_some_and(Invoked::lifts);
    crate::access_eval::exclusion::excluded(&read, exclusions, lifts, &mut verdict);
    denials(&read, directives, lifts, &mut verdict);
    if caller.kind == CallerKind::Representative {
        crate::access_eval::delegation::delegation(&read, directives, &mut verdict);
        controller_denials(&read, false, &mut verdict);
        return verdict;
    }
    // Step 4: a purpose and an emergency's reason, checked as input before any rule is read
    // (#5737, #6587).
    let missing = if judged.policy.purpose_required && judged.asked.purpose.is_none() {
        Some(Refusal::PurposeMissing)
    } else if invoked.is_some_and(|invoked| invoked.emergency && !invoked.reasoned) {
        Some(Refusal::ReasonMissing)
    } else {
        None
    };
    if let Some(refusal) = missing {
        verdict.rules.push(Rule {
            when: Pred::True,
            then: Outcome::Refused(refusal),
            by: Vec::new(),
        });
        return verdict;
    }
    if let Some(refused) = crate::access_eval::exclusion::conflict(&read, &mut verdict) {
        verdict.rules.push(refused);
        return verdict;
    }
    controller_denials(&read, lifts, &mut verdict);
    if let Some(invoked) = invoked.filter(|invoked| invoked.lifts()) {
        verdict.lifted.push(Lifted::Policy);
        verdict.grounds.justification = Some(Justified {
            at: invoked.at,
            emergency: true,
            cover: Pred::True,
        });
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
    let rule_permits = rule_permits(&read);
    let naming: Vec<&Permit<'_>> = permits.iter().chain(&rule_permits).collect();
    confidentiality::step_6(&read, &naming, &mut verdict);
    let keeper = Pred::any(vec![Pred::of(caller.administrator), read.keeper()]);
    let cared = read.cared();
    let live = cared && !held_closed;
    let default = judged
        .ehr
        .settings
        .map_or(judged.ehr.server_default, EhrAccessSettings::default_access);
    let care_read = judged.policy.care_required && caller.kind == CallerKind::Professional;
    if care_read || default != AccessDefault::Open {
        verdict.grounds.cared = live;
        verdict.grounds.keeper = keeper == Pred::True;
    }
    let needs = care_needs(judged.rules, directives);
    if cared && needs.named {
        verdict.grounds.cared = true;
    }
    if verdict.grounds.cared {
        verdict.grounds.granting =
            granting(judged.ehr.care.ties(), &needs.needs).map(|tie| Box::new(tie.clone()));
    }
    let covering = covering::steps_7_and_8(
        judged,
        &covering::Held {
            permits: &permits,
            keeper: &keeper,
            live,
            care_read,
            default,
            justified: invoked,
        },
        &mut verdict,
    );
    let admitted = match default {
        AccessDefault::Open => Pred::True,
        AccessDefault::Restricted => Pred::any(vec![keeper, covering]),
        AccessDefault::ConsentRequired => keeper,
    };
    disposition(admitted, permits, &mut verdict);
    verdict
}

/// The directives and the exclusions of `settings`, and whether the EHR is
/// held closed; none for an EHR with no settings.
fn lists(settings: Option<&EhrAccessSettings>) -> (&[AccessDirective], &[AccessDirective], bool) {
    match settings {
        Some(EhrAccessSettings::Directives(settings)) => (
            settings.directives.as_deref().unwrap_or_default(),
            settings.exclusions.as_deref().unwrap_or_default(),
            false,
        ),
        Some(EhrAccessSettings::HeldClosed) => (&[], &[], true),
        None => (&[], &[], false),
    }
}

/// Step 8: what the default disposition `admitted` does not admit, a permit
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
/// (opt-out, restriction, consent), or the mark of an emergency
/// justification that lifts it, when `lifts`.
fn denials(
    read: &Reader<'_, '_>,
    directives: &[AccessDirective],
    lifts: bool,
    verdict: &mut Verdict,
) {
    let opt_out_override = lifts && read.judged.caller.opt_out_override;
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
        let (refusal, lifted, lifted_here) = match kind {
            DirectiveKind::OptOut => (
                Outcome::Absent(Refusal::OptOut),
                Lifted::OptOut,
                opt_out_override,
            ),
            DirectiveKind::Restriction => (
                Outcome::Absent(Refusal::Restricted),
                Lifted::Restriction,
                lifts,
            ),
            DirectiveKind::Consent | DirectiveKind::Delegation => (
                Outcome::Refused(Refusal::ConsentDenied),
                Lifted::Consent,
                lifts,
            ),
            // NOTE: draft BMM `Directives_not_controller_rules`, an EHR's directives hold no rule
            // and no exclusion.
            DirectiveKind::ControllerRule | DirectiveKind::Exclusion => continue,
        };
        if directive.emergency_override && lifted_here {
            if binds == Pred::True {
                verdict.lifted.push(lifted);
                verdict
                    .grounds
                    .lifted
                    .push((at, read.cover(directive, Effect::Deny, placed)));
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
        Outcome::Refused(Refusal::Excluded) => 0,
        Outcome::Absent(Refusal::OptOut) => 1,
        Outcome::Absent(_) => 2,
        _ => 3,
    }
}

/// The permit rules of the controller's policy that may bind the caller.
fn rule_permits<'a>(read: &Reader<'_, 'a>) -> Vec<Permit<'a>> {
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

/// Step 5: every deny rule of the controller's policy that binds the
/// caller, or the mark of an emergency justification that lifts it, when
/// `lifts`. A rule binds every caller but the subject, whatever the
/// subject's directives permit.
fn controller_denials(read: &Reader<'_, '_>, lifts: bool, verdict: &mut Verdict) {
    for (at, rule) in read.judged.rules.iter().enumerate() {
        if kind_and_effect(rule).1 != Effect::Deny {
            continue;
        }
        let binds = read.binds(DirectiveKind::ControllerRule, rule, None);
        if binds == Pred::False {
            continue;
        }
        if rule.emergency_override && lifts {
            if binds == Pred::True {
                verdict.lifted.push(Lifted::Rule);
                verdict
                    .grounds
                    .lifted_rules
                    .push((at, read.cover(rule, Effect::Deny, None)));
            }
            continue;
        }
        verdict.rules.push(Rule {
            when: Pred::all(vec![binds, read.cover(rule, Effect::Deny, None)]),
            then: Outcome::Refused(Refusal::PolicyDenied),
            by: vec![at],
        });
    }
}

/// Step 6: data whose sensitivity no applying permit, a directive of the
/// EHR or a rule of the policy, names.
fn sensitivity(read: &Reader<'_, '_>, permits: &[&Permit<'_>], verdict: &mut Verdict) {
    let scopes = read.judged.scopes;
    for code in scopes.sensitivity_codes() {
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
