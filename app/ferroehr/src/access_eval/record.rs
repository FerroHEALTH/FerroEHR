// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The grounds of an access decision as the access record names them.
//!
//! EHDS Annex II 3.2 asks the log of each access for who accessed which data,
//! when and from where (`docs/law/eu/ehds/text.html`); Art. 9 gives the
//! person that log, and Art. 11(5) requires an access to restricted data in
//! the person's vital interest to be logged. The draft `ACCESS_AUTHORISATION`
//! is the shape of one ground (`openehr_rm::draft::access_log`), and FHIR R4
//! `AuditEvent.agent.policy` is "the policy or plan that authorized the
//! activity being recorded", which the `ehds-logging` record writes the
//! versions the grounds name to.
//!
//! A decision names, beside the outcome: each capability and role the caller
//! held (`role`); a live care relationship it read (`treatment
//! relationship`, defined in the `ACCESS_POLICY`); each directive that
//! admitted it, with its `EHR_ACCESS` version uid (`consent`, or
//! `representation` for a delegation); the subject's own access (`subject of
//! care`); and each directive or step an emergency lifted (`emergency`). A
//! refusal names its `refused-*` code, the step that decided it and the
//! directive or policy of that step, the absent answers of steps 1 and 2
//! included, whose response says nothing (EHDS Art. 8), and the exclusion of
//! a `refused-excluded`, which its response never names. An access a
//! conflict rule allowed names it (`conflict allowance`). Which grounds are
//! named, and how a directive is identified, is our own design (#5514,
//! #6077).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::log::Authorisation;
use openehr_rm::draft::access_control::terminology::{
    Confidentiality, DirectiveKind, Refusal, Term as _,
};
use openehr_rm::draft::access_log::access_log::access_authorisation::AccessAuthorisation;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::coded_text;
use openehr_rm::draft::access_log::access_log::access_care_ground::AccessCareGround;

use crate::access_eval::care_tie::Tie;
use crate::access_eval::condition::{Judgement, Noted, RequestAccess};
use crate::access_eval::procedure::{Lifted, Verdict};
use crate::access_eval::scope::{Item, Pred};
use crate::access_log::entry::authorisation;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::care_of::{CarerKind, ProviderKey};
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::system_log::event::{AccessDecision, AuditEvent, EventActionCode, EventOutcome};

/// The name a ground defined in an EHR's settings names them by.
const EHR_ACCESS: &str = "EHR_ACCESS";
/// The name a ground defined in the controller's policy names it by.
const ACCESS_POLICY: &str = "ACCESS_POLICY";

/// The step of the decision procedure that refused
/// ([`crate::access_eval::procedure`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 0, a subject caller on an EHR other than its own, before the
    /// numbered steps.
    Subject,
    /// 0, a representative on an EHR other than the one whose subject it
    /// represents, before the numbered steps.
    Representation,
    /// 0, a read by a caller whose agreement conditions it on a consent
    /// decision it names no reference to, before the numbered steps.
    ConsentReference,
    /// 0, an exclusion of the controller that names the caller, before the
    /// numbered steps.
    Exclusion,
    /// 1, an opt-out (EHDS Art. 10).
    OptOut,
    /// 2, a restriction (EHDS Art. 8).
    Restriction,
    /// 3, a consent deny.
    ConsentDeny,
    /// 4, a purpose the policy requires.
    Purpose,
    /// 5, a conflict of interest no conflict rule of the policy allows.
    Conflict,
    /// 5, a deny rule of the controller's policy.
    ControllerRule,
    /// 6, a sensitivity no applying permit names.
    Sensitivity,
    /// 6, a confidentiality above `N` no applying permit clears.
    Confidentiality,
    /// 7, a care relationship the policy requires.
    CareRelationship,
    /// 8, the default disposition.
    DefaultDisposition,
}

impl Step {
    /// The step that answers `refusal`; each code belongs to one step.
    #[must_use]
    pub const fn of(refusal: Refusal) -> Self {
        match refusal {
            Refusal::OptOut => Self::OptOut,
            Refusal::Restricted => Self::Restriction,
            Refusal::ConsentDenied => Self::ConsentDeny,
            Refusal::PolicyDenied => Self::ControllerRule,
            Refusal::Sensitivity => Self::Sensitivity,
            Refusal::Confidentiality => Self::Confidentiality,
            Refusal::NoCareRelationship => Self::CareRelationship,
            Refusal::PurposeMissing => Self::Purpose,
            Refusal::ConsentRequired => Self::DefaultDisposition,
            Refusal::NotSubject => Self::Subject,
            Refusal::NotRepresented => Self::Representation,
            Refusal::ConsentReferenceMissing => Self::ConsentReference,
            Refusal::Excluded => Self::Exclusion,
            Refusal::Conflict => Self::Conflict,
        }
    }

    /// The step's number in the procedure.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::Subject | Self::Representation | Self::ConsentReference | Self::Exclusion => 0,
            Self::OptOut => 1,
            Self::Restriction => 2,
            Self::ConsentDeny => 3,
            Self::Purpose => 4,
            Self::Conflict | Self::ControllerRule => 5,
            Self::Sensitivity | Self::Confidentiality => 6,
            Self::CareRelationship => 7,
            Self::DefaultDisposition => 8,
        }
    }

    /// The step's name.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Representation => "representation",
            Self::ConsentReference => "consent reference",
            Self::Exclusion => "exclusion",
            Self::OptOut => "opt-out",
            Self::Restriction => "restriction",
            Self::ConsentDeny => "consent deny",
            Self::Purpose => "purpose",
            Self::Conflict => "conflict",
            Self::ControllerRule => "controller rule",
            Self::Sensitivity => "sensitivity",
            Self::Confidentiality => "confidentiality",
            Self::CareRelationship => "care relationship",
            Self::DefaultDisposition => "default disposition",
        }
    }
}

/// A refusal as the access record names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The `refused-*` code.
    pub code: Refusal,
    /// The step that refused.
    pub step: Step,
    /// What decided it: the directives of the step with the `EHR_ACCESS`
    /// version that carries them, or the policy or default of the step.
    pub by: String,
}

impl Refused {
    /// The refusal in words, as the entry's `outcome.description` carries
    /// it: never clinical content.
    #[must_use]
    pub fn description(&self) -> String {
        format!(
            "step {} ({}): {}",
            self.step.number(),
            self.step.rubric(),
            self.by
        )
    }

    /// The `refused-*` code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code.code()
    }
}

/// What the access record names of one decision.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decision {
    /// The refusal; `None` for an admitted access.
    pub refused: Option<Refused>,
    /// The grounds the access rested on.
    pub grounds: Vec<AccessAuthorisation>,
    /// The versions of the policies the grounds are defined in, in order and
    /// without repeats: the `ehds-logging` record's authorisation reference.
    pub policies: Vec<String>,
    /// The confidentiality of the record the access reached, its marking's
    /// level, when it is above `N`: the entry's security label (draft access
    /// control, The Decision Procedure, Refusals).
    pub confidentiality: Option<Confidentiality>,
}

impl Decision {
    /// The decision a stored entry's `authorisations` hold, as every
    /// projection reads it back: the grounds defined in an `EHR_ACCESS` or
    /// the `ACCESS_POLICY`, and their versions; `None` when it holds none.
    #[must_use]
    pub fn of_recorded(authorisations: &[AccessAuthorisation]) -> Option<Box<Self>> {
        let grounds: Vec<AccessAuthorisation> = authorisations
            .iter()
            .filter(|ground| matches!(ground.name.as_deref(), Some(EHR_ACCESS | ACCESS_POLICY)))
            .cloned()
            .collect();
        let decision = Self {
            policies: policies_of(&grounds),
            grounds,
            refused: None,
            confidentiality: None,
        };
        (!decision.grounds.is_empty()).then(|| Box::new(decision))
    }
}

/// The versions `grounds` name, in order and without repeats.
fn policies_of(grounds: &[AccessAuthorisation]) -> Vec<String> {
    let mut policies: Vec<String> = Vec::new();
    for version in grounds.iter().filter_map(|ground| ground.version.as_ref()) {
        if !policies.contains(version) {
            policies.push(version.clone());
        }
    }
    policies
}

impl RequestAccess {
    /// What the access record of this request names for `ehr_id`; with no
    /// EHR, for the EHR the request was first refused on. An EHR a query
    /// served was judged from the per-EHR copy once the query ran, so it
    /// names the grounds a direct read of it names.
    #[must_use]
    pub fn decision(&self, ehr_id: Option<EhrId>) -> Decision {
        let noted = self.noted(ehr_id);
        let judged = ehr_id
            .or(noted.as_ref().and_then(|(ehr_id, _)| *ehr_id))
            .and_then(|ehr_id| self.judgement(ehr_id));
        let mut grounds = self.role_grounds();
        if let Some((_, noted)) = &noted
            && Step::of(noted.refusal) == Step::ControllerRule
        {
            // NOTE: our own design (#5691), a refusing rule is named as the legal obligation the
            // controller's rule states, so its legal grounds reach the record.
            grounds.extend(
                noted
                    .by
                    .iter()
                    .filter_map(|at| self.rule_ground(*at, Authorisation::LegalObligation)),
            );
        }
        let refused = noted.map(|(_, noted)| self.refused(&noted, judged.as_ref()));
        if let Some(judged) = &judged {
            let covering = ehr_id.and_then(|ehr_id| self.lifted_covers().of(ehr_id));
            self.verdict_grounds(judged, refused.is_none(), covering.as_ref(), &mut grounds);
        }
        let confidentiality = settings_of(judged.as_ref())
            .map(EhrAccessSettings::marking)
            .filter(|level| *level > Confidentiality::Normal);
        Decision {
            refused,
            policies: policies_of(&grounds),
            grounds,
            confidentiality,
        }
    }

    /// The legal grounds of the controller rules that refused `ehr_id`; with
    /// no EHR, those of the request's first refusal. Empty for a refusal by
    /// any other step and for an admitted access.
    #[must_use]
    pub fn refusal_grounds(&self, ehr_id: Option<EhrId>) -> Vec<DvCodedText> {
        let Some((_, noted)) = self.noted(ehr_id) else {
            return Vec::new();
        };
        if Step::of(noted.refusal) != Step::ControllerRule {
            return Vec::new();
        }
        noted
            .by
            .iter()
            .filter_map(|at| self.rules().get(*at))
            .flat_map(|rule| rule.legal_grounds.as_deref().unwrap_or_default())
            .cloned()
            .collect()
    }

    /// The capabilities the caller held and the roles it performs.
    fn role_grounds(&self) -> Vec<AccessAuthorisation> {
        let capabilities = self
            .record_facts()
            .capabilities
            .iter()
            .map(|(operation, role)| AccessAuthorisation {
                name: Some((*operation).to_owned()),
                reference: Some(role_reference(*role)),
                ..authorisation(Authorisation::Role)
            });
        let roles = self
            .caller()
            .facts
            .roles()
            .iter()
            .map(|role| AccessAuthorisation {
                name: Some(format!("{}::{}", role.terminology, role.code)),
                ..authorisation(Authorisation::Role)
            });
        capabilities.chain(roles).collect()
    }

    /// The grounds `judged` rests on: those that admitted it when
    /// `admitted`, and each emergency override; when the request admitted
    /// objects of the EHR, only the lifted denies and rules `covering` them
    /// (#5630, #5676).
    fn verdict_grounds(
        &self,
        judged: &Judgement,
        admitted: bool,
        covering: Option<&Covering>,
        grounds: &mut Vec<AccessAuthorisation>,
    ) {
        let verdict = &judged.verdict;
        let found = &verdict.grounds;
        let directives = directives_of((*judged.settings).as_ref());
        let version = judged.version.as_deref();
        let directive = |at: usize, kind: Option<Authorisation>| {
            directives
                .get(at)
                .map(|d| directive_ground(d, at, version, kind))
        };
        if admitted {
            if found.subject {
                grounds.push(authorisation(Authorisation::SubjectOfCare));
            }
            grounds.extend(found.permits.iter().filter_map(|at| directive(*at, None)));
            if found.keeper && !self.caller().administrator {
                grounds.push(AccessAuthorisation {
                    name: Some(EHR_ACCESS.to_owned()),
                    version: version.map(str::to_owned),
                    ..authorisation(Authorisation::Consent)
                });
            }
            if found.cared {
                grounds.push(self.care_authorisation(found.granting.as_deref()));
            }
            // NOTE: draft decision procedure step 5, an access a conflict rule allowed is marked.
            grounds.extend(found.conflicts.iter().map(|conflict| AccessAuthorisation {
                reference: Some(DvIdentifier {
                    issuer: None,
                    assigner: None,
                    id: conflict.code().to_owned(),
                    r#type: Some("conflict".to_owned()),
                }),
                ..self.policy_ground(Authorisation::ConflictAllowance)
            }));
        }
        let exclusions = exclusions_of((*judged.settings).as_ref());
        grounds.extend(found.lifted_exclusions.iter().filter_map(|(at, _)| {
            exclusions
                .get(*at)
                .map(|e| directive_ground(e, *at, version, Some(Authorisation::Emergency)))
        }));
        grounds.extend(
            found
                .lifted
                .iter()
                .map(|(at, _)| *at)
                .filter(|at| covering.is_none_or(|covering| covering.directives.contains(at)))
                .filter_map(|at| directive(at, Some(Authorisation::Emergency))),
        );
        grounds.extend(
            found
                .lifted_rules
                .iter()
                .map(|(at, _)| *at)
                .filter(|at| covering.is_none_or(|covering| covering.rules.contains(at)))
                .filter_map(|at| self.rule_ground(at, Authorisation::Emergency)),
        );
        if verdict.lifted.contains(&Lifted::Policy) {
            grounds.push(self.policy_ground(Authorisation::Emergency));
        }
    }

    /// The treatment relationship ground of a tie `granting` names: for the
    /// `attested` path, the assertion it rests on and the agreement it was
    /// attested under (draft decision procedure, Refusals: "the assertion
    /// and agreement for `attested`"); otherwise the relationship, as a
    /// ground of the access policy in force.
    fn care_authorisation(&self, granting: Option<&Tie>) -> AccessAuthorisation {
        let attested = granting
            .filter(|tie| tie.path == CarePath::Attested)
            .and_then(|_| self.caller().facts.attestation());
        match attested {
            // NOTE: draft access log ACCESS_CARE_GROUND, its carer and provider are 1..1 and an
            // attested relationship names neither here, so the ground carries no care element.
            Some(attestation) => AccessAuthorisation {
                name: Some(attestation.agreement.name.clone()),
                version: Some(attestation.agreement.version.clone()),
                reference: Some(attestation.assertion_ref()),
                ..authorisation(Authorisation::TreatmentRelationship)
            },
            None => AccessAuthorisation {
                care: granting
                    .and_then(|tie| care_ground(tie, &self.party_namespace()))
                    .map(Box::new),
                ..self.policy_ground(Authorisation::TreatmentRelationship)
            },
        }
    }

    /// A ground of `kind` defined in the access policy in force.
    fn policy_ground(&self, kind: Authorisation) -> AccessAuthorisation {
        let version = self.record_facts().policy_version.clone();
        AccessAuthorisation {
            name: version.as_ref().map(|_| ACCESS_POLICY.to_owned()),
            version,
            ..authorisation(kind)
        }
    }

    /// The rules of the access policy in force.
    fn rules(&self) -> &[AccessDirective] {
        crate::access_eval::procedure::rules_of(self.policy.as_deref())
    }

    /// The ground the rule at `at` of the access policy gives, of `kind`.
    fn rule_ground(&self, at: usize, kind: Authorisation) -> Option<AccessAuthorisation> {
        let rule = self.rules().get(at)?;
        Some(AccessAuthorisation {
            reference: Some(identify(rule, at)),
            legal_grounds: rule
                .legal_grounds
                .as_ref()
                .map(|grounds| grounds.iter().cloned().collect()),
            ..self.policy_ground(kind)
        })
    }

    /// The refusal `noted` names, with what decided it.
    fn refused(&self, noted: &Noted, judged: Option<&Judgement>) -> Refused {
        let step = Step::of(noted.refusal);
        let version = judged.and_then(|judged| judged.version.as_deref());
        let carrier = || {
            version.map_or_else(
                || EHR_ACCESS.to_owned(),
                |version| format!("{EHR_ACCESS} {version}"),
            )
        };
        let policy = || {
            self.record_facts().policy_version.as_ref().map_or_else(
                || format!("{ACCESS_POLICY} (none committed)"),
                |version| format!("{ACCESS_POLICY} {version}"),
            )
        };
        let by = if step == Step::Exclusion {
            let exclusions = exclusions_of(settings_of(judged));
            let named: Vec<String> = noted
                .by
                .iter()
                .map(|at| {
                    exclusions
                        .get(*at)
                        .map_or_else(|| position(*at), |e| identify(e, *at))
                        .id
                })
                .collect();
            format!("exclusion {} of {}", named.join(", "), carrier())
        } else if step == Step::ControllerRule {
            let named: Vec<String> = noted
                .by
                .iter()
                .map(|at| {
                    self.rules()
                        .get(*at)
                        .map_or_else(|| position(*at), |rule| identify(rule, *at))
                        .id
                })
                .collect();
            format!("rule {} of {}", named.join(", "), policy())
        } else if noted.by.is_empty() {
            match step {
                Step::Sensitivity => format!("{} and the labels of {}", policy(), carrier()),
                Step::Confidentiality => {
                    match settings_of(judged).map(EhrAccessSettings::marking) {
                        Some(level) if level > Confidentiality::Normal => {
                            format!("the marking {} of {}", level.code(), carrier())
                        }
                        _ => format!("{} and the labels of {}", policy(), carrier()),
                    }
                }
                Step::CareRelationship | Step::Purpose => policy(),
                Step::Conflict => format!("the conflict rules of {}", policy()),
                Step::Subject => "the EHR of the caller's patient context".to_owned(),
                Step::Representation => "the EHR of the subject the caller represents".to_owned(),
                Step::ConsentReference => {
                    "the consent condition of the caller's agreement".to_owned()
                }
                Step::DefaultDisposition if settings_of(judged).is_some() => {
                    format!("the default_access of {}", carrier())
                }
                _ => "the server's default disposition".to_owned(),
            }
        } else {
            let directives = directives_of(settings_of(judged));
            let named: Vec<String> = noted
                .by
                .iter()
                .map(|at| {
                    directives
                        .get(*at)
                        .map_or_else(|| position(*at), |d| identify(d, *at))
                        .id
                })
                .collect();
            format!("directive {} of {}", named.join(", "), carrier())
        };
        Refused {
            code: noted.refusal,
            step,
            by,
        }
    }
}

/// The lifted denies and deny rules whose data scope covered an object the
/// request admitted of one EHR.
#[derive(Debug, Clone, Default)]
pub(crate) struct Covering {
    /// Positions in the settings' `directives`.
    directives: BTreeSet<usize>,
    /// Positions in the policy's `rules`.
    rules: BTreeSet<usize>,
}

/// For each EHR with a lift where the request admitted an object, the lifted
/// denies and deny rules whose data scope covered such an object: the access
/// record names a lift only for the data it lifted (EHDS Art. 11(5) and
/// Art. 9(1); our own design, #5630, #5676).
#[derive(Debug, Default)]
pub(crate) struct LiftedCovers(Mutex<BTreeMap<EhrId, Covering>>);

impl LiftedCovers {
    /// Note which denies and rules `verdict` lifted on `ehr_id` cover the
    /// admitted `item`; the EHR itself is no object, and a request naming no
    /// EHR names no lift, so neither notes anything.
    pub(crate) fn note(&self, ehr_id: Option<EhrId>, verdict: &Verdict, item: &Item<'_>) {
        let Some(ehr_id) = ehr_id else {
            return;
        };
        let found = &verdict.grounds;
        if found.lifted.is_empty() && found.lifted_rules.is_empty()
            || !matches!(item, Item::Object(_))
        {
            return;
        }
        let covering = |lifted: &[(usize, Pred)]| -> Vec<usize> {
            lifted
                .iter()
                .filter(|(_, cover)| cover.holds(item))
                .map(|(at, _)| *at)
                .collect()
        };
        let mut covers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let covered = covers.entry(ehr_id).or_default();
        covered.directives.extend(covering(&found.lifted));
        covered.rules.extend(covering(&found.lifted_rules));
    }

    /// What was noted for `ehr_id`; `None` when the request admitted no
    /// object of it, so the record names every lift of the EHR's verdict.
    pub(crate) fn of(&self, ehr_id: EhrId) -> Option<Covering> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ehr_id)
            .cloned()
    }
}

/// Stamp `event` with the decision `access` gave the EHR it names.
///
/// The record carries the grounds, the policies they are defined in, and the
/// refusal, which marks the access refused whatever its status: the `404` of
/// steps 1 and 2 says nothing (EHDS Art. 8), the record says all.
pub fn stamp(event: &mut AuditEvent, access: &RequestAccess) {
    // NOTE: our own design, a record whose EHR is no UUID names no EHR the
    // request judged, which is legitimately absent.
    let ehr_id = event
        .ehr_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .map(EhrId);
    // NOTE: our own design (#5589, #5962), a read names the subject its own read found in the
    // store, while a write keeps the one its transaction reads after the write.
    if matches!(
        event.action,
        EventActionCode::Read | EventActionCode::Execute
    ) {
        event.judged_subject = ehr_id.and_then(|ehr_id| access.subject_read(ehr_id));
    }
    let decision = access.decision(ehr_id);
    if decision.refused.is_some() {
        event.decision = AccessDecision::Refused;
        // NOTE: EHDS Art. 9(1), a request answered refused that named its EHR outside its path,
        // such as a query's scope, is recorded against that EHR; an answered query's served
        // record names it already (#5639, #5611).
        if event.ehr_id.is_none()
            && event.outcome != EventOutcome::Success
            && let Some((Some(refused_on), _)) = access.noted(None)
        {
            event.ehr_id = Some(refused_on.to_string());
        }
    }
    event.decided = Some(Box::new(decision));
}

/// The directives of `settings`; none for an EHR with no settings or held
/// closed.
fn directives_of(settings: Option<&EhrAccessSettings>) -> &[AccessDirective] {
    match settings {
        Some(EhrAccessSettings::Directives(settings)) => {
            settings.directives.as_deref().unwrap_or_default()
        }
        Some(EhrAccessSettings::HeldClosed) | None => &[],
    }
}

/// The controller's exclusions in `settings`; none for an EHR with no
/// settings or held closed.
fn exclusions_of(settings: Option<&EhrAccessSettings>) -> &[AccessDirective] {
    match settings {
        Some(EhrAccessSettings::Directives(settings)) => {
            settings.exclusions.as_deref().unwrap_or_default()
        }
        Some(EhrAccessSettings::HeldClosed) | None => &[],
    }
}

/// The settings `judged` was judged on.
fn settings_of(judged: Option<&Judgement>) -> Option<&EhrAccessSettings> {
    judged.and_then(|judged| (*judged.settings).as_ref())
}

/// The ground the directive at `at` gives, of `kind`, or by its directive
/// kind: `representation` for a delegation, `consent` otherwise.
fn directive_ground(
    directive: &AccessDirective,
    at: usize,
    version: Option<&str>,
    kind: Option<Authorisation>,
) -> AccessAuthorisation {
    let kind = kind.unwrap_or_else(|| {
        if DirectiveKind::read_coded(&directive.kind) == Ok(DirectiveKind::Delegation) {
            Authorisation::Representation
        } else {
            Authorisation::Consent
        }
    });
    AccessAuthorisation {
        name: Some(EHR_ACCESS.to_owned()),
        version: version.map(str::to_owned),
        reference: Some(identify(directive, at)),
        legal_grounds: directive
            .legal_grounds
            .as_ref()
            .map(|grounds| grounds.iter().cloned().collect()),
        ..authorisation(kind)
    }
}

/// How the record identifies the directive at `at`: its uid, else its first
/// identifier in another system, else its position, from 1.
fn identify(directive: &AccessDirective, at: usize) -> DvIdentifier {
    if let Some(uid) = &directive.uid {
        return DvIdentifier {
            issuer: None,
            assigner: None,
            id: uid.value().to_owned(),
            r#type: Some("uid".to_owned()),
        };
    }
    directive
        .identifiers
        .as_ref()
        .and_then(|identifiers| identifiers.iter().next().cloned())
        .unwrap_or_else(|| position(at))
}

/// The identifier of the directive at `at` by its position, from 1.
fn position(at: usize) -> DvIdentifier {
    DvIdentifier {
        issuer: None,
        assigner: None,
        id: at.saturating_add(1).to_string(),
        r#type: Some("position".to_owned()),
    }
}

/// The care relationship `tie` rests on as the access log names it: its
/// role, its path, its carer, the version that holds it, whether grace
/// admitted the access, the carer it covers for and its provider (draft
/// access control decision procedure, Refusals); `None` for a tie read
/// without its relationship. The carer is named under `namespace`, the one
/// this CDR names its parties under.
fn care_ground(tie: &Tie, namespace: &str) -> Option<AccessCareGround> {
    let grant = tie.grant.as_deref()?;
    let identifier = |key: &ProviderKey, r#type: Option<&str>| DvIdentifier {
        issuer: Some(key.issuer.clone()),
        assigner: None,
        id: key.id.clone(),
        r#type: r#type.map(str::to_owned),
    };
    Some(AccessCareGround {
        role: coded_text(&tie.role.terminology, &tie.role.code, &tie.role.code),
        path: tie.path.coded(),
        carer: PartyRef {
            namespace: namespace.to_owned(),
            r#type: grant
                .carer_kind
                .map_or("PARTY", CarerKind::rm_type)
                .to_owned(),
            // NOTE: BASE HIER_OBJECT_ID, a UUID always reads as one.
            id: ObjectId::HierObjectId(HierObjectId::new(grant.carer.to_string()).ok()?),
        },
        relationship: grant
            .version
            .as_deref()
            .and_then(|version| version.parse::<ObjectVersionId>().ok()),
        in_grace: grant.in_grace,
        covers_for: grant.covers_for.as_ref().map(|key| identifier(key, None)),
        provider: identifier(&grant.provider, Some("ORGANISATION")),
    })
}

/// The identifier of the `ROLE` container whose `CAPABILITY` grants a
/// capability.
fn role_reference(role: Uuid) -> DvIdentifier {
    DvIdentifier {
        issuer: None,
        assigner: None,
        id: role.to_string(),
        r#type: Some("ROLE".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

    /// The versions the stored entry's grounds name come back as the
    /// policies every projection writes after the data permit, and a ground
    /// defined elsewhere names none.
    #[test]
    fn the_stored_entry_gives_back_the_policies_of_its_grounds() {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        "dr-b".clone_into(&mut event.user_id);
        event.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
        event.permit = Some("hdab-nl/2026/0042".to_owned());
        let ground = |name: &str, version: &str| AccessAuthorisation {
            name: Some(name.to_owned()),
            version: Some(version.to_owned()),
            ..authorisation(Authorisation::Consent)
        };
        let grounds = [
            ground(EHR_ACCESS, "ehr-access::cdr::2"),
            ground("agreement-of-the-region", "4"),
            ground(ACCESS_POLICY, "policy::cdr::3"),
            ground(EHR_ACCESS, "ehr-access::cdr::2"),
        ];
        event.decided = Some(Box::new(Decision {
            refused: None,
            policies: policies_of(&grounds),
            grounds: grounds.to_vec(),
            confidentiality: None,
        }));
        let retention = crate::access_log::retention::stated(
            None,
            &crate::system_log::retention::LogRetention::default(),
        );
        let entry =
            crate::access_log::entry::entry_of(&event, None, "cdr", retention).expect("an entry");
        let stored = crate::access_log::recorded::stored(&entry, 1).expect("it reads back");
        assert_eq!(
            stored.event.policies(),
            ["hdab-nl/2026/0042", "ehr-access::cdr::2", "policy::cdr::3"]
        );
    }

    /// Every refusal code belongs to the step that answers it.
    #[test]
    fn each_refusal_belongs_to_its_step() {
        let steps: Vec<(u8, &str)> = Refusal::ALL
            .iter()
            .map(|refusal| (Step::of(*refusal).number(), refusal.code()))
            .collect();
        for (number, code) in steps {
            let expected = match code {
                "refused-opt-out" => 1,
                "refused-restricted" => 2,
                "refused-consent-denied" => 3,
                "refused-purpose-missing" => 4,
                "refused-policy-denied" | "refused-conflict" => 5,
                "refused-sensitivity" | "refused-confidentiality" => 6,
                "refused-no-care-relationship" => 7,
                "refused-consent-required" => 8,
                "refused-not-subject"
                | "refused-not-represented"
                | "refused-consent-reference-missing"
                | "refused-excluded" => 0,
                other => panic!("an unknown refusal code {other}"),
            };
            assert_eq!(number, expected, "{code}");
        }
    }
}
