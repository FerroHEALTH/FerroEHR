// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The class invariants of the draft access control, checked over the
//! generated types of [`crate::draft::access_control`].
//!
//! Each invariant is the BMM's own, named as the BMM names it
//! (`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`).
//! The draft is FerroEHR's change proposal and no openEHR release, so these
//! checks are our own design over our own model (#5506).
//!
//! The `*_valid` invariants that ask an optional list to be absent or
//! non-empty are not checked here: the generated types hold each such list
//! as an `Option<NonEmptyVec<_>>`, so the reader refuses an empty one.
//!
//! The RM data values the classes hold keep their own invariants wherever
//! they are used: each `DV_DATE_TIME` and `DV_DURATION` is checked by its
//! `Value_valid`, through the BASE validator the RM types use (RM Data Types,
//! `DV_DATE_TIME` and `DV_DURATION` §Invariants, #5629).
//!
//! The terminology invariants name `Terminology_id_access`, the draft
//! `openehr_access` terminology, whose groups take an HL7 code where one
//! means the same: a coded value holds when
//! [`crate::draft::access_control::terminology`] reads it as a term of the
//! invariant's group.

use crate::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use crate::draft::access_control::access_control::access_classification::AccessClassification;
use crate::draft::access_control::access_control::access_conflict_rule::AccessConflictRule;
use crate::draft::access_control::access_control::access_data_scope::AccessDataScope;
use crate::draft::access_control::access_control::access_directive::AccessDirective;
use crate::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use crate::draft::access_control::access_control::access_log_restriction::AccessLogRestriction;
use crate::draft::access_control::access_control::access_log_retention::AccessLogRetention;
use crate::draft::access_control::access_control::access_policy::AccessPolicy;
use crate::draft::access_control::access_control::access_relationship_validity::AccessRelationshipValidity;
use crate::draft::access_control::access_control::access_relationship_writer::AccessRelationshipWriter;
use crate::draft::access_control::terminology::log::AccessorKind;
use crate::draft::access_control::terminology::{
    AccessDefault, CallerKind, Conflict, DirectiveKind, Effect, Relationship,
    TERMINOLOGY_ID_CONFIDENTIALITY, Term, ValueSetBinding, Withheld,
};
use crate::draft::invariants::{Check, Violations};
use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use crate::v1_2::data_types::quantity::dv_interval::DvInterval;
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
use crate::v1_2::validate::{valid_iso8601_date_time, valid_iso8601_duration};

/// `ACCESS_DIRECTIVE.is_deny`: the effect is the `deny` code.
fn is_deny(directive: &AccessDirective) -> bool {
    Effect::Deny.is(&directive.effect)
}

/// `ACCESS_DIRECTIVE.is_restriction`: the kind is the `restriction` code.
fn is_restriction(directive: &AccessDirective) -> bool {
    DirectiveKind::Restriction.is(&directive.kind)
}

/// `ACCESS_DIRECTIVE.is_opt_out`: the kind is the `opt_out` code.
fn is_opt_out(directive: &AccessDirective) -> bool {
    DirectiveKind::OptOut.is(&directive.kind)
}

/// `ACCESS_DIRECTIVE.is_controller_rule`: the kind is the `controller_rule`
/// code.
fn is_controller_rule(directive: &AccessDirective) -> bool {
    DirectiveKind::ControllerRule.is(&directive.kind)
}

/// Whether `directive` and each of its exceptions are controller rules, all
/// of them when `rules` and none of them otherwise.
fn all_rules_or_none(directive: &AccessDirective, rules: bool) -> bool {
    is_controller_rule(directive) == rules
        && directive
            .exceptions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(|e| is_controller_rule(e) == rules)
}

/// Whether `coded`, when present, is a term of the group `T`.
fn absent_or<T: Term>(coded: Option<&DvCodedText>) -> bool {
    coded.is_none_or(|coded| T::read_coded(coded).is_ok())
}

/// Checks every invariant of an `ACCESS_DIRECTIVE_SETTINGS` and of the
/// directives it holds.
///
/// # Errors
/// [`Violations`] naming each invariant the settings break.
pub fn check_settings(settings: &AccessDirectiveSettings) -> Result<(), Violations> {
    let mut check = Check::default();
    check.hold(
        AccessDefault::read_coded(&settings.default_access).is_ok(),
        "ACCESS_DIRECTIVE_SETTINGS",
        "Default_access_valid",
    );
    let directives = settings.directives.as_deref().unwrap_or_default();
    check.hold(
        directives.iter().filter(|d| is_opt_out(d)).count() <= 1,
        "ACCESS_DIRECTIVE_SETTINGS",
        "Opt_out_unique",
    );
    check.hold(
        directives.iter().all(|d| all_rules_or_none(d, false)),
        "ACCESS_DIRECTIVE_SETTINGS",
        "Directives_not_controller_rules",
    );
    for directive in directives {
        directive_rules(&mut check, directive);
    }
    check.finish()
}

/// Checks every invariant of an `ACCESS_POLICY` and of the classifications,
/// rules and catalogue entries it holds.
///
/// # Errors
/// [`Violations`] naming each invariant the policy breaks.
pub fn check_policy(policy: &AccessPolicy) -> Result<(), Violations> {
    let mut check = Check::default();
    // NOTE: RM Common IM, LOCATABLE `Archetyped_valid`: a node is an archetype
    // root exactly when it carries `archetype_details`.
    check.hold(
        policy.archetype_details.is_some(),
        "ACCESS_POLICY",
        "Is_archetype_root",
    );
    if let Some(grace) = &policy.relationship_grace {
        duration(&mut check, grace);
    }
    check.hold(
        policy
            .relationship_grace
            .as_ref()
            .is_none_or(|grace| grace.magnitude().is_some_and(|seconds| seconds >= 0.0)),
        "ACCESS_POLICY",
        "Grace_valid",
    );
    let catalogue = policy.catalogue.as_deref().unwrap_or_default();
    check.hold(
        catalogue.iter().all(|d| d.identifiers.is_some()),
        "ACCESS_POLICY",
        "Catalogue_identified",
    );
    check.hold(
        catalogue.iter().all(|d| all_rules_or_none(d, false)),
        "ACCESS_POLICY",
        "Catalogue_not_controller_rules",
    );
    let rules = policy.rules.as_deref().unwrap_or_default();
    check.hold(
        rules.iter().all(|r| all_rules_or_none(r, true)),
        "ACCESS_POLICY",
        "Rules_kind",
    );
    for classification in policy.classifications.as_deref().unwrap_or_default() {
        classification_rules(&mut check, classification);
    }
    for directive in catalogue.iter().chain(rules) {
        directive_rules(&mut check, directive);
    }
    for restriction in policy.log_restrictions.as_deref().unwrap_or_default() {
        log_restriction_rules(&mut check, restriction);
    }
    if let Some(retention) = &policy.log_retention {
        log_retention_rules(&mut check, retention);
    }
    care_rules(&mut check, policy);
    check.finish()
}

/// The invariants of the attributes that say how care relationships are
/// read: `Self_declared_valid`, `Concealed_valid`, and those of each
/// `ACCESS_CONFLICT_RULE` and `ACCESS_VALUE_SET`.
fn care_rules(check: &mut Check, policy: &AccessPolicy) {
    if let Some(period) = &policy.self_declared_care {
        duration(check, period);
    }
    check.hold(
        policy
            .self_declared_care
            .as_ref()
            .is_none_or(|period| period.magnitude().is_some_and(|seconds| seconds > 0.0)),
        "ACCESS_POLICY",
        "Self_declared_valid",
    );
    check.hold(
        policy
            .concealed_confidentialities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(|level| {
                level.defining_code.terminology_id.value == TERMINOLOGY_ID_CONFIDENTIALITY
            }),
        "ACCESS_POLICY",
        "Concealed_valid",
    );
    for rule in policy.conflict_rules.as_deref().unwrap_or_default() {
        conflict_rule_rules(check, rule);
    }
    for value_set in policy.value_sets.as_deref().unwrap_or_default() {
        check.hold(
            ValueSetBinding::read_coded(&value_set.binding).is_ok(),
            "ACCESS_VALUE_SET",
            "Binding_valid",
        );
    }
    for entry in policy.relationship_validity.as_deref().unwrap_or_default() {
        relationship_validity_rules(check, entry);
    }
    for writer in policy.relationship_writers.as_deref().unwrap_or_default() {
        relationship_writer_rules(check, writer);
    }
    check.hold(
        policy
            .team_targets
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(|target| target.r#type == "ORGANISATION"),
        "ACCESS_POLICY",
        "Team_targets_valid",
    );
}

/// The `ACCESS_RELATIONSHIP_VALIDITY` invariants, comparing durations by RM
/// `DV_DURATION.magnitude`.
fn relationship_validity_rules(check: &mut Check, entry: &AccessRelationshipValidity) {
    const CLASS: &str = "ACCESS_RELATIONSHIP_VALIDITY";
    let positive = |period: Option<&DvDuration>| {
        period.is_none_or(|period| period.magnitude().is_some_and(|seconds| seconds > 0.0))
    };
    for period in [&entry.default_period, &entry.max_period]
        .into_iter()
        .flatten()
    {
        duration(check, period);
    }
    check.hold(
        positive(entry.default_period.as_ref()),
        CLASS,
        "Default_valid",
    );
    check.hold(positive(entry.max_period.as_ref()), CLASS, "Max_valid");
    check.hold(
        match (&entry.default_period, &entry.max_period) {
            (Some(default), Some(max)) => default
                .magnitude()
                .zip(max.magnitude())
                .is_some_and(|(default, max)| default <= max),
            _ => true,
        },
        CLASS,
        "Default_within_max",
    );
    check.hold(
        entry.default_period.is_none() || entry.max_period.is_some(),
        CLASS,
        "Bounded_together",
    );
}

/// The `ACCESS_RELATIONSHIP_WRITER` invariants and those of the actor
/// scopes it names.
fn relationship_writer_rules(check: &mut Check, writer: &AccessRelationshipWriter) {
    check.hold(
        writer.confidentialities.iter().all(|level| {
            level.defining_code.terminology_id.value == TERMINOLOGY_ID_CONFIDENTIALITY
        }),
        "ACCESS_RELATIONSHIP_WRITER",
        "Confidentialities_valid",
    );
    for scope in &writer.applies_to {
        actor_scope_rules(check, scope);
    }
}

fn conflict_rule_rules(check: &mut Check, rule: &AccessConflictRule) {
    let conflict = Conflict::read_coded(&rule.conflict);
    check.hold(conflict.is_ok(), "ACCESS_CONFLICT_RULE", "Conflict_valid");
    check.hold(
        rule.kinships.is_none() || conflict == Ok(Conflict::Related),
        "ACCESS_CONFLICT_RULE",
        "Kinships_valid",
    );
    for scope in rule.applies_to.as_deref().unwrap_or_default() {
        actor_scope_rules(check, scope);
    }
}

/// The `ACCESS_LOG_RETENTION` invariants, comparing durations by RM
/// `DV_DURATION.magnitude`; a duration with no magnitude is refused by its
/// `Value_valid`, and no comparison over it holds.
fn log_retention_rules(check: &mut Check, retention: &AccessLogRetention) {
    const CLASS: &str = "ACCESS_LOG_RETENTION";
    // NOTE: Regulation (EU) 2025/327 Art. 9(2), "at least three years from each date of access".
    const UNION_FLOOR: &str = "P3Y";
    let seconds = |value: &DvDuration| value.magnitude();
    let floors = retention.floors.as_deref().unwrap_or_default();
    let ceilings = retention.ceilings.as_deref().unwrap_or_default();
    for bound in floors.iter().chain(ceilings) {
        duration(check, &bound.period);
        check.hold(
            seconds(&bound.period).is_some_and(|s| s > 0.0),
            "ACCESS_RETENTION_BOUND",
            "Period_valid",
        );
    }
    let period = retention.period.as_ref();
    if let Some(period) = period {
        duration(check, period);
    }
    let period_seconds = period.and_then(seconds);
    let at_least = |floor: &DvDuration| {
        period.is_none()
            || period_seconds
                .zip(seconds(floor))
                .is_some_and(|(p, f)| p >= f)
    };
    check.hold(
        retention.in_union || !floors.is_empty(),
        CLASS,
        "Floor_outside_union",
    );
    let union_floor = DvDuration {
        normal_status: None,
        normal_range: None,
        other_reference_ranges: None,
        magnitude_status: None,
        accuracy: None,
        accuracy_is_percent: None,
        value: UNION_FLOOR.to_owned(),
    };
    check.hold(
        !retention.in_union || at_least(&union_floor),
        CLASS,
        "Period_union",
    );
    check.hold(
        floors.iter().all(|floor| at_least(&floor.period)),
        CLASS,
        "Period_floors",
    );
    check.hold(
        ceilings.is_empty() || period.is_some(),
        CLASS,
        "Ceilings_need_period",
    );
    check.hold(
        period.is_none()
            || ceilings.iter().all(|ceiling| {
                period_seconds
                    .zip(seconds(&ceiling.period))
                    .is_some_and(|(p, c)| p <= c)
            }),
        CLASS,
        "Period_ceilings",
    );
    check.hold(
        ceilings.iter().all(|ceiling| {
            floors.iter().all(|floor| {
                seconds(&ceiling.period)
                    .zip(seconds(&floor.period))
                    .is_some_and(|(c, f)| c >= f)
            })
        }),
        CLASS,
        "Ceilings_floors",
    );
}

fn log_restriction_rules(check: &mut Check, restriction: &AccessLogRestriction) {
    check.hold(
        restriction
            .accessor_kinds
            .iter()
            .all(|kind| AccessorKind::read_coded(kind).is_ok()),
        "ACCESS_LOG_RESTRICTION",
        "Accessor_kinds_valid",
    );
    check.hold(
        restriction
            .withheld
            .iter()
            .all(|withheld| Withheld::read_coded(withheld).is_ok()),
        "ACCESS_LOG_RESTRICTION",
        "Withheld_valid",
    );
}

fn directive_rules(check: &mut Check, directive: &AccessDirective) {
    check.hold(
        DirectiveKind::read_coded(&directive.kind).is_ok(),
        "ACCESS_DIRECTIVE",
        "Kind_valid",
    );
    check.hold(
        Effect::read_coded(&directive.effect).is_ok(),
        "ACCESS_DIRECTIVE",
        "Effect_valid",
    );
    let deny = is_deny(directive);
    check.hold(
        !is_restriction(directive) || deny,
        "ACCESS_DIRECTIVE",
        "Restriction_denies",
    );
    check.hold(
        !is_opt_out(directive) || (deny && directive.data.is_none()),
        "ACCESS_DIRECTIVE",
        "Opt_out_denies_whole_record",
    );
    let exceptions = directive.exceptions.as_deref().unwrap_or_default();
    check.hold(
        exceptions
            .iter()
            .all(|e| is_deny(e) != deny && e.exceptions.is_none()),
        "ACCESS_DIRECTIVE",
        "Exceptions_opposite",
    );
    if let Some(validity) = &directive.time_validity {
        interval(check, validity);
    }
    if let Some(instructed) = &directive.instructed {
        date_time(check, instructed);
    }
    for scope in directive.applies_to.as_deref().unwrap_or_default() {
        actor_scope_rules(check, scope);
    }
    for scope in directive.data.as_deref().unwrap_or_default() {
        data_scope_rules(check, scope);
    }
    for exception in exceptions {
        directive_rules(check, exception);
    }
}

fn actor_scope_rules(check: &mut Check, scope: &AccessActorScope) {
    check.hold(
        scope.party.is_some()
            || scope.role.is_some()
            || scope.organisation_type.is_some()
            || scope.relationship.is_some()
            || scope.caller_kind.is_some(),
        "ACCESS_ACTOR_SCOPE",
        "Not_empty",
    );
    check.hold(
        absent_or::<Relationship>(scope.relationship.as_ref()),
        "ACCESS_ACTOR_SCOPE",
        "Relationship_valid",
    );
    check.hold(
        absent_or::<CallerKind>(scope.caller_kind.as_ref()),
        "ACCESS_ACTOR_SCOPE",
        "Caller_kind_valid",
    );
}

/// `DV_DATE_TIME.Value_valid`: the value is an ISO 8601 date-time.
fn date_time(check: &mut Check, value: &DvDateTime) {
    check.hold(
        valid_iso8601_date_time(&value.value),
        "DV_DATE_TIME",
        "Value_valid",
    );
}

/// `DV_DURATION.Value_valid`: the value is an ISO 8601 duration.
fn duration(check: &mut Check, value: &DvDuration) {
    check.hold(
        valid_iso8601_duration(&value.value),
        "DV_DURATION",
        "Value_valid",
    );
}

/// Each limit of `interval` by [`date_time`].
fn interval(check: &mut Check, interval: &DvInterval<DvDateTime>) {
    for limit in [&interval.lower, &interval.upper].into_iter().flatten() {
        date_time(check, limit);
    }
}

fn data_scope_rules(check: &mut Check, scope: &AccessDataScope) {
    if let Some(recorded) = &scope.recorded {
        interval(check, recorded);
    }
    check.hold(
        scope.objects.is_some()
            || scope.templates.is_some()
            || scope.archetypes.is_some()
            || scope.categories.is_some()
            || scope.sensitivities.is_some()
            || scope.origins.is_some()
            || scope.recorded.is_some(),
        "ACCESS_DATA_SCOPE",
        "Not_empty",
    );
}

fn classification_rules(check: &mut Check, classification: &AccessClassification) {
    check.hold(
        classification.data.objects.is_none(),
        "ACCESS_CLASSIFICATION",
        "No_objects",
    );
    data_scope_rules(check, &classification.data);
}
