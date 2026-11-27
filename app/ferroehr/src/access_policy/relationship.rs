// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's bounds on writing a care relationship: its validity, the
//! `ORGANISATION` units it may name and the writers a marked record needs.
//!
//! Draft access control decision procedure, Writing Relationships: a care
//! relationship written without an end takes the `_default_period_` of the
//! first `ACCESS_POLICY._relationship_validity_` entry that covers its role
//! and provider type, one longer than that entry's `_max_period_` is refused,
//! one no entry covers must be written with an end, and an entry with neither
//! period lets it stay open; an `ORGANISATION` is a target only when
//! `_team_targets_` names it, and never the controller; a relationship to a
//! record marked above `N` is written only by a writer
//! `_relationship_writers_` names for that level. The checks here are pure:
//! the commit reads the facts they take inside its transaction, in the
//! crate's `party_binding::grants::caller::bounds`. Our own design over our
//! own model.

use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::access_control::access_relationship_validity::AccessRelationshipValidity;
use openehr_rm::draft::access_control::terminology::{Confidentiality, ForeignCode, ForeignKind};
use openehr_rm::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use uuid::Uuid;

use crate::party_binding::derive::container;
use crate::party_binding::grants::caller::care_of::CarerKind;
use crate::service::ehr::access_rule::party_key;

/// What a care relationship states for its bounds: its role, the types of
/// its provider and the dates its `time_validity` holds.
#[derive(Debug, Clone, Copy)]
pub struct Written<'a> {
    /// The `care_role`.
    pub role: &'a ForeignCode,
    /// The types the provider's `ORGANISATION` names for itself.
    pub provider_types: &'a [ForeignCode],
    /// The first date it holds; `None` when unbounded.
    pub lower: Option<jiff::civil::Date>,
    /// The first date it no longer holds; `None` when written without an
    /// end.
    pub upper: Option<jiff::civil::Date>,
    /// The date of the commit, the start of a relationship written without
    /// one.
    pub today: jiff::civil::Date,
}

impl Written<'_> {
    /// The first date it holds, the commit's date when it names none.
    fn start(&self) -> jiff::civil::Date {
        self.lower.unwrap_or(self.today)
    }
}

/// Why a care relationship's validity is refused at write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidityRefusal {
    /// It has no end, and no entry lets it stay open or gives it a default
    /// period.
    #[error(
        "a care relationship in the role {role} must be written with an end: no relationship_validity \
         entry of the access policy covers its role and provider type with a default period"
    )]
    EndRequired {
        /// The role, as `terminology::code`.
        role: String,
    },
    /// It is longer than the maximum period of the entry that covers it.
    #[error(
        "a care relationship in the role {role} runs past {until}, the end the maximum period \
         {max} of the access policy allows from its start"
    )]
    PastMaximum {
        /// The role, as `terminology::code`.
        role: String,
        /// The entry's `max_period`.
        max: String,
        /// The first date past the maximum.
        until: jiff::civil::Date,
    },
    /// A period of the entry does not add to a date.
    #[error("the period {0} of the access policy does not add to a date")]
    Period(String),
}

/// The end a care relationship takes from the policy `policy`, `None` when
/// it keeps the dates it was written with: an end of its own, or no end that
/// the covering entry lets stay open.
///
/// # Errors
/// [`ValidityRefusal`] when it has no end and none may be given, when it is
/// longer than the covering entry allows, and when a period does not add.
pub fn effective_end(
    policy: Option<&AccessPolicy>,
    written: &Written<'_>,
) -> Result<Option<jiff::civil::Date>, ValidityRefusal> {
    let role = format!("{}::{}", written.role.terminology, written.role.code);
    let Some(entry) = covering(policy, written) else {
        return match written.upper {
            Some(_) => Ok(None),
            None => Err(ValidityRefusal::EndRequired { role }),
        };
    };
    let start = written.start();
    if let Some(max) = &entry.max_period {
        let until = add(start, max)?;
        if written.upper.is_some_and(|upper| upper > until) {
            return Err(ValidityRefusal::PastMaximum {
                role,
                max: max.value.clone(),
                until,
            });
        }
    }
    match (written.upper, &entry.default_period, &entry.max_period) {
        (Some(_), ..) | (None, None, None) => Ok(None),
        (None, Some(default), _) => add(start, default).map(Some),
        (None, None, Some(_)) => Err(ValidityRefusal::EndRequired { role }),
    }
}

/// Why a care relationship with no `episode` is refused at write.
///
/// The first `ACCESS_POLICY._relationship_validity_` entry covering its role
/// and provider type marks `_episode_required_` (draft access control
/// decision procedure, Writing Relationships).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "a care relationship in the role {role} names an episode: the relationship_validity entry of \
     the access policy that covers its role and provider type marks episode_required"
)]
pub struct EpisodeRequired {
    /// The role, as `terminology::code`.
    pub role: String,
}

/// Judges whether the care relationship `written`, naming an episode when
/// `episode`, may be written without one under `policy`.
///
/// # Errors
/// [`EpisodeRequired`] when it names none and the covering entry requires one.
pub fn episode(
    policy: Option<&AccessPolicy>,
    written: &Written<'_>,
    episode: bool,
) -> Result<(), EpisodeRequired> {
    let required =
        covering(policy, written).is_some_and(|entry| entry.episode_required == Some(true));
    if required && !episode {
        return Err(EpisodeRequired {
            role: format!("{}::{}", written.role.terminology, written.role.code),
        });
    }
    Ok(())
}

/// Why a self-declared care relationship is refused at write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelfDeclaredRefusal {
    /// The policy sets no `self_declared_care`.
    #[error(
        "a self_declared care relationship is refused: the access policy sets no \
         self_declared_care"
    )]
    Off,
    /// Its target is not the writer's own `PERSON`.
    #[error(
        "a self_declared care relationship is written by the professional it names: its target \
         is the writer's own PERSON"
    )]
    NotTheWriter,
    /// It ends after the period `self_declared_care` allows from its start,
    /// or has no end.
    #[error(
        "a self_declared care relationship ends within {max} of its start, by {until} at the \
         latest"
    )]
    TooLong {
        /// The policy's `self_declared_care`.
        max: String,
        /// The first date past that period.
        until: jiff::civil::Date,
    },
    /// The period does not add to a date.
    #[error(transparent)]
    Period(#[from] ValidityRefusal),
}

/// Judges a self-declared care relationship `written` against `policy`.
///
/// `end` is the end it holds from then on, `(kind, carer)` its target and
/// `writer` the party that wrote it: the policy sets `self_declared_care`,
/// the target is the writer's own `PERSON`, and it ends within that period
/// of its start (draft access control decision procedure, Writing
/// Relationships).
///
/// # Errors
/// [`SelfDeclaredRefusal`] naming the condition it fails.
pub fn self_declared(
    policy: Option<&AccessPolicy>,
    written: &Written<'_>,
    end: Option<jiff::civil::Date>,
    (kind, carer): (CarerKind, Uuid),
    writer: Option<Uuid>,
) -> Result<(), SelfDeclaredRefusal> {
    let max = policy
        .and_then(|policy| policy.self_declared_care.as_ref())
        .ok_or(SelfDeclaredRefusal::Off)?;
    if kind != CarerKind::Person || writer != Some(carer) {
        return Err(SelfDeclaredRefusal::NotTheWriter);
    }
    let until = add(written.start(), max)?;
    match end {
        Some(end) if end <= until => Ok(()),
        _ => Err(SelfDeclaredRefusal::TooLong {
            max: max.value.clone(),
            until,
        }),
    }
}

/// The first entry of `policy` that covers the role and a provider type of
/// `written`.
fn covering<'a>(
    policy: Option<&'a AccessPolicy>,
    written: &Written<'_>,
) -> Option<&'a AccessRelationshipValidity> {
    let entries = policy?.relationship_validity.as_deref()?;
    entries.iter().find(|entry| {
        let role = entry.roles.as_deref().is_none_or(|roles| {
            roles.iter().any(|coded| {
                ForeignCode::read(ForeignKind::CareRole, coded).is_ok_and(|c| c == *written.role)
            })
        });
        let kind = entry.organisation_types.as_deref().is_none_or(|types| {
            types.iter().any(|coded| {
                ForeignCode::read(ForeignKind::OrganisationType, coded)
                    .is_ok_and(|c| written.provider_types.contains(&c))
            })
        });
        role && kind
    })
}

/// `start` plus `period`.
///
/// # Errors
/// [`ValidityRefusal::Period`] when the period does not read as an ISO 8601
/// duration or does not add to a date.
pub fn add(
    start: jiff::civil::Date,
    period: &DvDuration,
) -> Result<jiff::civil::Date, ValidityRefusal> {
    period
        .value
        .parse::<jiff::Span>()
        .ok()
        .and_then(|span| start.checked_add(span).ok())
        .ok_or_else(|| ValidityRefusal::Period(period.value.clone()))
}

/// Why the target of a care relationship is refused at write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetRefusal {
    /// The target is an `ORGANISATION` the policy's `team_targets` does not
    /// name.
    #[error(
        "the target ORGANISATION {0} is no unit the access policy's team_targets names; a care \
         team is a GROUP"
    )]
    NotATeamTarget(Uuid),
    /// The target is the controller, or the provider whose care it is.
    #[error(
        "the target ORGANISATION {0} is the controller or the relationship's provider, which is \
         never a target"
    )]
    Controller(Uuid),
}

/// Refuses the target `target` of kind `kind` under `policy`: an
/// `ORGANISATION` the policy's `team_targets` does not name, the policy's
/// controller and `provider`, the container of the relationship's provider.
///
/// # Errors
/// The [`TargetRefusal`] of an `ORGANISATION` target that is no unit the
/// policy allows.
pub fn target(
    policy: Option<&AccessPolicy>,
    kind: CarerKind,
    target: Uuid,
    provider: Uuid,
) -> Result<(), TargetRefusal> {
    if kind != CarerKind::Organisation {
        return Ok(());
    }
    let controller = policy
        .and_then(|p| party_key(&p.controller))
        .and_then(|k| container(&k));
    if controller == Some(target) || provider == target {
        return Err(TargetRefusal::Controller(target));
    }
    let named = policy
        .and_then(|p| p.team_targets.as_deref())
        .unwrap_or_default()
        .iter()
        .filter_map(|unit| party_key(unit).and_then(|key| container(&key)))
        .any(|unit| unit == target);
    if named {
        Ok(())
    } else {
        Err(TargetRefusal::NotATeamTarget(target))
    }
}

/// What the policy says of a writer of a care relationship to a record at
/// one confidentiality level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterVerdict {
    /// The record is marked `N`: any authorised writer writes.
    Unrestricted,
    /// A writer `relationship_writers` names for the level writes, and the
    /// write is recorded for review.
    Review,
    /// No writer the policy names for the level: refused.
    Refused,
}

/// The verdict on a writer of a relationship to a record at `level`, where
/// `names` says whether an actor scope names the writer.
#[must_use]
pub fn writer(
    policy: Option<&AccessPolicy>,
    level: Confidentiality,
    names: impl Fn(&AccessActorScope) -> bool,
) -> WriterVerdict {
    if level == Confidentiality::Normal {
        return WriterVerdict::Unrestricted;
    }
    let listed = policy
        .and_then(|p| p.relationship_writers.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(|entry| {
            entry
                .confidentialities
                .iter()
                .any(|coded| Confidentiality::read(coded).is_ok_and(|at| at == level))
        })
        .any(|entry| entry.applies_to.iter().any(&names));
    if listed {
        WriterVerdict::Review
    } else {
        WriterVerdict::Refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str = "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7";
    const WARD: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";

    /// A policy with `extra` members.
    fn policy(extra: &str) -> AccessPolicy {
        let body = format!(
            r#"{{ "_type": "ACCESS_POLICY",
                "name": {{ "_type": "DV_TEXT", "value": "policy" }},
                "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
                "controller": {{ "_type": "PARTY_REF", "namespace": "demographic",
                    "type": "ORGANISATION",
                    "id": {{ "_type": "HIER_OBJECT_ID", "value": "{CONTROLLER}" }} }},
                "care_relationship_required": true,
                "purpose_required": false{extra} }}"#
        );
        openehr_its::json::from_canonical_json(&body).expect("the policy should decode")
    }

    fn term(terminology: &str, code: &str) -> String {
        format!(
            r#"{{ "_type": "DV_CODED_TEXT", "value": "{code}",
                "defining_code": {{ "terminology_id": {{ "value": "{terminology}" }},
                    "code_string": "{code}" }} }}"#
        )
    }

    fn duration(value: &str) -> String {
        format!(r#"{{ "_type": "DV_DURATION", "value": "{value}" }}"#)
    }

    fn date(text: &str) -> jiff::civil::Date {
        text.parse().expect("a date")
    }

    fn code(terminology: &str, code: &str) -> ForeignCode {
        ForeignCode {
            terminology: terminology.to_owned(),
            code: code.to_owned(),
        }
    }

    /// A relationship in `role` of a provider of `types`, from 2026-11-01
    /// until `upper`, written on 2026-11-20.
    fn written<'a>(
        role: &'a ForeignCode,
        types: &'a [ForeignCode],
        upper: Option<&str>,
    ) -> Written<'a> {
        Written {
            role,
            provider_types: types,
            lower: Some(date("2026-11-01")),
            upper: upper.map(date),
            today: date("2026-11-20"),
        }
    }

    /// Entries for a hospital's attending role (`P30D`, at most `P180D`), a
    /// general practice's (open), and every other role (at most `P1Y`, no
    /// default).
    fn bounded() -> AccessPolicy {
        policy(&format!(
            r#", "relationship_validity": [
                {{ "_type": "ACCESS_RELATIONSHIP_VALIDITY", "roles": [{att}],
                   "organisation_types": [{hosp}],
                   "default_period": {d30}, "max_period": {d180} }},
                {{ "_type": "ACCESS_RELATIONSHIP_VALIDITY", "organisation_types": [{gp}] }},
                {{ "_type": "ACCESS_RELATIONSHIP_VALIDITY", "max_period": {y1} }}]"#,
            att = term("HL7_ParticipationType", "ATND"),
            hosp = term("urn:example:org-types", "hospital"),
            gp = term("urn:example:org-types", "gp"),
            d30 = duration("P30D"),
            d180 = duration("P180D"),
            y1 = duration("P1Y"),
        ))
    }

    #[test]
    fn the_first_covering_entry_that_marks_episode_required_refuses_a_relationship_with_none() {
        let policy = policy(&format!(
            r#", "relationship_validity": [
                {{ "_type": "ACCESS_RELATIONSHIP_VALIDITY",
                   "organisation_types": [{hosp}], "episode_required": true }},
                {{ "_type": "ACCESS_RELATIONSHIP_VALIDITY", "episode_required": false }}]"#,
            hosp = term("urn:example:org-types", "hospital"),
        ));
        let role = code("HL7_ParticipationType", "ATND");
        let hospital = [code("urn:example:org-types", "hospital")];
        let gp = [code("urn:example:org-types", "gp")];
        let at_hospital = written(&role, &hospital, Some("2026-12-01"));
        assert_eq!(
            episode(Some(&policy), &at_hospital, false),
            Err(EpisodeRequired {
                role: "HL7_ParticipationType::ATND".to_owned()
            })
        );
        assert_eq!(episode(Some(&policy), &at_hospital, true), Ok(()));
        assert_eq!(
            episode(
                Some(&policy),
                &written(&role, &gp, Some("2026-12-01")),
                false
            ),
            Ok(()),
            "the first entry covering a GP's relationship requires none"
        );
        assert_eq!(episode(None, &at_hospital, false), Ok(()), "no policy");
    }

    #[test]
    fn an_open_relationship_no_entry_covers_is_refused_and_a_closed_one_kept() {
        let role = code("HL7_ParticipationType", "ATND");
        for policy in [None, Some(policy(""))] {
            assert_eq!(
                effective_end(policy.as_ref(), &written(&role, &[], None)),
                Err(ValidityRefusal::EndRequired {
                    role: "HL7_ParticipationType::ATND".to_owned()
                })
            );
            assert_eq!(
                effective_end(policy.as_ref(), &written(&role, &[], Some("2030-01-01"))),
                Ok(None),
                "a written end with no entry covering it stands"
            );
        }
    }

    #[test]
    fn an_open_relationship_takes_the_default_period_of_the_first_covering_entry() {
        let policy = bounded();
        let role = code("HL7_ParticipationType", "ATND");
        let hospital = [code("urn:example:org-types", "hospital")];
        assert_eq!(
            effective_end(Some(&policy), &written(&role, &hospital, None)),
            Ok(Some(date("2026-12-01"))),
            "from its start, 2026-11-01, plus P30D"
        );
        let mut from_today = written(&role, &hospital, None);
        from_today.lower = None;
        assert_eq!(
            effective_end(Some(&policy), &from_today),
            Ok(Some(date("2026-12-20"))),
            "no start: from the commit's date"
        );
        let gp = [code("urn:example:org-types", "gp")];
        assert_eq!(
            effective_end(Some(&policy), &written(&role, &gp, None)),
            Ok(None),
            "an entry with neither period lets it stay open"
        );
        let other = code("HL7_ParticipationType", "CON");
        assert!(
            matches!(
                effective_end(Some(&policy), &written(&other, &hospital, None)),
                Err(ValidityRefusal::EndRequired { .. })
            ),
            "a maximum with no default needs a written end"
        );
    }

    #[test]
    fn a_relationship_past_the_maximum_is_refused() {
        let policy = bounded();
        let role = code("HL7_ParticipationType", "ATND");
        let hospital = [code("urn:example:org-types", "hospital")];
        assert_eq!(
            effective_end(
                Some(&policy),
                &written(&role, &hospital, Some("2027-04-30"))
            ),
            Ok(None),
            "exactly P180D from 2026-11-01"
        );
        assert_eq!(
            effective_end(
                Some(&policy),
                &written(&role, &hospital, Some("2027-05-01"))
            ),
            Err(ValidityRefusal::PastMaximum {
                role: "HL7_ParticipationType::ATND".to_owned(),
                max: "P180D".to_owned(),
                until: date("2027-04-30"),
            })
        );
    }

    #[test]
    fn an_organisation_target_needs_team_targets_and_is_never_the_controller() {
        let ward = Uuid::parse_str(WARD).expect("a uuid");
        let controller = Uuid::parse_str(CONTROLLER).expect("a uuid");
        let other = Uuid::now_v7();
        let provider = Uuid::now_v7();
        for kind in [CarerKind::Person, CarerKind::Role, CarerKind::Group] {
            assert_eq!(target(None, kind, other, provider), Ok(()), "{kind:?}");
        }
        assert_eq!(
            target(Some(&policy("")), CarerKind::Organisation, ward, provider),
            Err(TargetRefusal::NotATeamTarget(ward)),
            "void team_targets names no unit"
        );
        let units = policy(&format!(
            r#", "team_targets": [{{ "_type": "PARTY_REF", "namespace": "demographic",
                "type": "ORGANISATION",
                "id": {{ "_type": "HIER_OBJECT_ID", "value": "{WARD}" }} }}]"#
        ));
        assert_eq!(
            target(Some(&units), CarerKind::Organisation, ward, provider),
            Ok(())
        );
        assert_eq!(
            target(Some(&units), CarerKind::Organisation, ward, ward),
            Err(TargetRefusal::Controller(ward)),
            "the provider whose care it is"
        );
        assert_eq!(
            target(Some(&units), CarerKind::Organisation, controller, provider),
            Err(TargetRefusal::Controller(controller))
        );
        assert_eq!(
            target(Some(&units), CarerKind::Organisation, other, provider),
            Err(TargetRefusal::NotATeamTarget(other))
        );
    }

    #[test]
    fn a_marked_record_needs_a_writer_the_policy_names_for_its_level() {
        let writers = policy(&format!(
            r#", "relationship_writers": [{{ "_type": "ACCESS_RELATIONSHIP_WRITER",
                "applies_to": [{{ "_type": "ACCESS_ACTOR_SCOPE", "role": {} }}],
                "confidentialities": [{}] }}]"#,
            term("urn:example:roles", "adt"),
            term("HL7_Confidentiality", "R")
        ));
        let adt = |scope: &AccessActorScope| {
            scope
                .role
                .as_ref()
                .is_some_and(|role| role.defining_code.code_string == "adt")
        };
        let nobody = |_: &AccessActorScope| false;
        for policy in [None, Some(&writers)] {
            assert_eq!(
                writer(policy, Confidentiality::Normal, nobody),
                WriterVerdict::Unrestricted
            );
        }
        assert_eq!(
            writer(Some(&writers), Confidentiality::Restricted, adt),
            WriterVerdict::Review
        );
        assert_eq!(
            writer(Some(&writers), Confidentiality::Restricted, nobody),
            WriterVerdict::Refused
        );
        assert_eq!(
            writer(Some(&writers), Confidentiality::VeryRestricted, adt),
            WriterVerdict::Refused,
            "named for R only"
        );
        assert_eq!(
            writer(None, Confidentiality::VeryRestricted, adt),
            WriterVerdict::Refused,
            "void relationship_writers names none"
        );
    }

    #[test]
    fn a_self_declared_relationship_needs_the_switch_its_writer_and_a_short_end() {
        let role = code("HL7_ParticipationType", "PART");
        let switched = policy(&format!(r#", "self_declared_care": {}"#, duration("P2D")));
        let writer = Uuid::parse_str(WARD).expect("a uuid");
        let target = (CarerKind::Person, writer);
        let short = written(&role, &[], Some("2026-11-03"));
        let end = Some(date("2026-11-03"));
        assert_eq!(
            self_declared(None, &short, end, target, Some(writer)),
            Err(SelfDeclaredRefusal::Off)
        );
        assert_eq!(
            self_declared(Some(&switched), &short, end, target, Some(writer)),
            Ok(())
        );
        assert_eq!(
            self_declared(Some(&switched), &short, end, target, None),
            Err(SelfDeclaredRefusal::NotTheWriter),
            "a writer that names no party"
        );
        assert_eq!(
            self_declared(
                Some(&switched),
                &short,
                end,
                (CarerKind::Group, writer),
                Some(writer)
            ),
            Err(SelfDeclaredRefusal::NotTheWriter),
            "a team is never self-declared"
        );
        let long = written(&role, &[], Some("2026-11-04"));
        assert!(matches!(
            self_declared(
                Some(&switched),
                &long,
                Some(date("2026-11-04")),
                target,
                Some(writer)
            ),
            Err(SelfDeclaredRefusal::TooLong { .. })
        ));
        let open = written(&role, &[], None);
        assert!(matches!(
            self_declared(Some(&switched), &open, None, target, Some(writer)),
            Err(SelfDeclaredRefusal::TooLong { .. })
        ));
    }
}
