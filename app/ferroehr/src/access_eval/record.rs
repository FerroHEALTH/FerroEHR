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
//! included, whose response says nothing (EHDS Art. 8). Which grounds are
//! named, and how a directive is identified, is our own design (#5514).

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Refusal, Term as _};
use openehr_rm::draft::access_log::access_log::access_authorisation::AccessAuthorisation;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use uuid::Uuid;

use crate::access_eval::condition::{Judgement, Noted, RequestAccess};
use crate::access_eval::procedure::Lifted;
use crate::access_log::entry::authorisation;
use crate::ids::EhrId;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::system_log::event::{AccessDecision, AuditEvent};

/// The name a ground defined in an EHR's settings names them by.
const EHR_ACCESS: &str = "EHR_ACCESS";
/// The name a ground defined in the controller's policy names it by.
const ACCESS_POLICY: &str = "ACCESS_POLICY";

/// The step of the decision procedure that refused
/// ([`crate::access_eval::procedure`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 1, an opt-out (EHDS Art. 10).
    OptOut,
    /// 2, a restriction (EHDS Art. 8).
    Restriction,
    /// 3, a consent deny.
    ConsentDeny,
    /// 4, a sensitivity the caller is not cleared for.
    Sensitivity,
    /// 5, a care relationship the policy requires.
    CareRelationship,
    /// 6, a purpose the policy requires.
    Purpose,
    /// 7, the default disposition.
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
            Refusal::Sensitivity => Self::Sensitivity,
            Refusal::NoCareRelationship => Self::CareRelationship,
            Refusal::PurposeMissing | Refusal::PurposeNotPermitted => Self::Purpose,
            Refusal::ConsentRequired => Self::DefaultDisposition,
        }
    }

    /// The step's number in the procedure.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::OptOut => 1,
            Self::Restriction => 2,
            Self::ConsentDeny => 3,
            Self::Sensitivity => 4,
            Self::CareRelationship => 5,
            Self::Purpose => 6,
            Self::DefaultDisposition => 7,
        }
    }

    /// The step's name.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::OptOut => "opt-out",
            Self::Restriction => "restriction",
            Self::ConsentDeny => "consent deny",
            Self::Sensitivity => "sensitivity",
            Self::CareRelationship => "care relationship",
            Self::Purpose => "purpose",
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
}

impl Decision {
    /// The decision a stored record names by the versions of its policies
    /// alone, as the store reads it back; `None` when it names none.
    #[must_use]
    pub fn of_policies(policies: Vec<String>) -> Option<Box<Self>> {
        (!policies.is_empty()).then(|| {
            Box::new(Self {
                policies,
                ..Self::default()
            })
        })
    }
}

impl RequestAccess {
    /// What the access record of this request names for `ehr_id`; with no
    /// EHR, for the EHR the request was first refused on.
    #[must_use]
    pub fn decision(&self, ehr_id: Option<EhrId>) -> Decision {
        let noted = self.noted(ehr_id);
        let judged = ehr_id
            .or(noted.as_ref().map(|(ehr_id, _)| *ehr_id))
            .and_then(|ehr_id| self.judgement(ehr_id));
        let mut grounds = self.role_grounds();
        let refused = noted.map(|(_, noted)| self.refused(&noted, judged.as_ref()));
        if let Some(judged) = &judged {
            self.verdict_grounds(judged, refused.is_none(), &mut grounds);
        }
        let mut policies: Vec<String> = Vec::new();
        for version in grounds.iter().filter_map(|ground| ground.version.as_ref()) {
            if !policies.contains(version) {
                policies.push(version.clone());
            }
        }
        Decision {
            refused,
            grounds,
            policies,
        }
    }

    /// The capabilities and roles the caller held.
    fn role_grounds(&self) -> Vec<AccessAuthorisation> {
        let capabilities = self
            .record_facts()
            .capabilities
            .iter()
            .map(|(operation, role)| AccessAuthorisation {
                name: Some((*operation).to_owned()),
                reference: Some(role_reference(*role)),
                ..authorisation("role")
            });
        let roles = self.caller().roles.iter().map(|role| AccessAuthorisation {
            name: Some(role.clone()),
            ..authorisation("role")
        });
        capabilities.chain(roles).collect()
    }

    /// The grounds `judged` rests on: those that admitted it when
    /// `admitted`, and each emergency override.
    fn verdict_grounds(
        &self,
        judged: &Judgement,
        admitted: bool,
        grounds: &mut Vec<AccessAuthorisation>,
    ) {
        let verdict = &judged.verdict;
        let found = &verdict.grounds;
        let directives = directives_of((*judged.settings).as_ref());
        let version = judged.version.as_deref();
        let directive = |at: usize, kind: Option<&'static str>| {
            directives
                .get(at)
                .map(|d| directive_ground(d, at, version, kind))
        };
        if admitted {
            if found.subject {
                grounds.push(authorisation("subject of care"));
            }
            grounds.extend(found.permits.iter().filter_map(|at| directive(*at, None)));
            if found.keeper && !self.caller().administrator {
                grounds.push(AccessAuthorisation {
                    name: Some(EHR_ACCESS.to_owned()),
                    version: version.map(str::to_owned),
                    ..authorisation("consent")
                });
            }
            if found.cared {
                grounds.push(self.policy_ground("treatment relationship"));
            }
        }
        grounds.extend(
            found
                .lifted
                .iter()
                .filter_map(|at| directive(*at, Some("emergency"))),
        );
        if verdict.lifted.contains(&Lifted::Policy) {
            grounds.push(self.policy_ground("emergency"));
        }
    }

    /// A ground of `kind` defined in the access policy in force.
    fn policy_ground(&self, kind: &str) -> AccessAuthorisation {
        let version = self.record_facts().policy_version.clone();
        AccessAuthorisation {
            name: version.as_ref().map(|_| ACCESS_POLICY.to_owned()),
            version,
            ..authorisation(kind)
        }
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
        let by = if noted.by.is_empty() {
            match step {
                Step::Sensitivity => format!("{} and the labels of {}", policy(), carrier()),
                Step::CareRelationship | Step::Purpose => policy(),
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
    let decision = access.decision(ehr_id);
    if decision.refused.is_some() {
        event.decision = AccessDecision::Refused;
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
    kind: Option<&'static str>,
) -> AccessAuthorisation {
    let kind = kind.unwrap_or_else(|| {
        if DirectiveKind::read_coded(&directive.kind) == Ok(DirectiveKind::Delegation) {
            "representation"
        } else {
            "consent"
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
                "refused-sensitivity" => 4,
                "refused-no-care-relationship" => 5,
                "refused-purpose-missing" | "refused-purpose-not-permitted" => 6,
                "refused-consent-required" => 7,
                other => panic!("an unknown refusal code {other}"),
            };
            assert_eq!(number, expected, "{code}");
        }
    }
}
