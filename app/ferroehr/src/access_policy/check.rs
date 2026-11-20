// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The strict check every `ACCESS_POLICY` passes before it is stored.
//!
//! It runs the canonical-JSON reader of the generated draft type, which refuses an
//! undeclared, missing or duplicate member and an empty optional list, then
//! the class invariants of the draft BMM
//! ([`check_policy`]), the root rules of a
//! `LOCATABLE` and of the `controller`, and that each sensitivity and
//! category a classification names, each role, purpose, sensitivity,
//! category and legal ground a controller rule names, each role and
//! organisation type a conflict rule names, each legal ground
//! of a restriction of the access log's subject view, each provision of
//! the access log retention, and each care role, organisation type and actor
//! scope of the bounds on care relationships, comes from the jurisdiction's
//! terminology ([`ForeignCode`]); that each period of `relationship_validity`
//! adds to a date and `team_targets` does not name the controller; and that
//! the access log retention is one the audit trail keeps records under
//! ([`crate::access_policy::retention`]).
//!
//! The draft package is FerroEHR's change proposal and no openEHR release:
//! our own design over our own model (#5512).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the policy body arrives as the canonical JSON \
              the strict typed reader decodes"
)]

use serde_json::Value;

use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::invariants::check_policy;
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind, NotForeign};
use openehr_rm::draft::invariants::Violations;

/// The RM class an access policy archetype constrains.
const RM_CLASS: &str = "ACCESS_POLICY";

/// The `PARTY_REF.type` of the controller.
const CONTROLLER_TYPE: &str = "ORGANISATION";

/// Why a body is no access policy FerroEHR stores.
#[derive(Debug, thiserror::Error)]
pub enum PolicyDefect {
    /// The body is not a canonical-JSON `ACCESS_POLICY`.
    #[error("the body is not a canonical-JSON ACCESS_POLICY")]
    NotAPolicy(#[source] openehr_its::json::JsonParseError),
    /// The policy breaks invariants of the draft BMM.
    #[error(transparent)]
    Invariants(#[from] Violations),
    /// The root `archetype_node_id` is not the archetype id
    /// `archetype_details` names.
    #[error(
        "the root archetype_node_id {node_id:?} is not the archetype id archetype_details names \
         ({archetype_id:?})"
    )]
    RootNodeId {
        /// The root `archetype_node_id`.
        node_id: String,
        /// `archetype_details.archetype_id.value`.
        archetype_id: String,
    },
    /// `archetype_details` names an archetype of another RM class.
    #[error("archetype_details names {0:?}, which is no archetype of an ACCESS_POLICY")]
    ArchetypeClass(String),
    /// A sensitivity or category of a classification, a role, purpose,
    /// sensitivity, category or legal ground of a controller rule, or a legal
    /// ground of a log restriction, is coded in the draft access terminology,
    /// which holds none.
    #[error(transparent)]
    NotForeign(#[from] NotForeign),
    /// A period of the access log retention is no whole number of years or
    /// days.
    #[error(transparent)]
    RetentionPeriod(#[from] crate::access_policy::retention::PeriodForm),
    /// The access log retention breaks a bound it declares, as
    /// `ehds-logging` counts calendar years and days.
    #[error(transparent)]
    Retention(#[from] crate::system_log::retention::RetentionDefect),
    /// The controller is not an `ORGANISATION`.
    #[error("the controller is a {0:?} reference; the controller is an ORGANISATION")]
    Controller(String),
    /// A period of `relationship_validity` does not add to a date.
    #[error(transparent)]
    RelationshipPeriod(#[from] crate::access_policy::relationship::ValidityRefusal),
    /// `team_targets` names the controller, which is never a target.
    #[error("team_targets names the controller, which is never the target of a care relationship")]
    TeamTargetController,
    /// The policy sets an attribute no access decision of this server reads
    /// yet.
    #[error("ACCESS_POLICY.{0} is not decided by this server, so it is not stored")]
    Undecided(String),
}

/// `body` read and checked as an access policy.
///
/// # Errors
/// The first [`PolicyDefect`] of the body; invariants are reported together.
pub fn checked(body: &Value) -> Result<AccessPolicy, PolicyDefect> {
    let policy: AccessPolicy =
        openehr_its::json::from_canonical_value(body).map_err(PolicyDefect::NotAPolicy)?;
    check_policy(&policy)?;
    if let Some(details) = &policy.archetype_details {
        let archetype_id = &details.archetype_id.value;
        if policy.archetype_node_id != *archetype_id {
            return Err(PolicyDefect::RootNodeId {
                node_id: policy.archetype_node_id.clone(),
                archetype_id: archetype_id.clone(),
            });
        }
        if details.archetype_id.rm_entity() != RM_CLASS {
            return Err(PolicyDefect::ArchetypeClass(archetype_id.clone()));
        }
    }
    for classification in policy.classifications.as_deref().unwrap_or_default() {
        let data = &classification.data;
        for sensitivity in classification
            .sensitivities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .chain(data.sensitivities.as_deref().unwrap_or_default())
        {
            ForeignCode::read(ForeignKind::Sensitivity, sensitivity)?;
        }
        for category in data.categories.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::Category, category)?;
        }
    }
    for rule in policy.rules.as_deref().unwrap_or_default() {
        rule_codes(rule)?;
        for exception in rule.exceptions.as_deref().unwrap_or_default() {
            rule_codes(exception)?;
        }
    }
    for rule in policy.conflict_rules.as_deref().unwrap_or_default() {
        for scope in rule.applies_to.as_deref().unwrap_or_default() {
            scope_codes(scope)?;
        }
    }
    relationship_codes(&policy)?;
    for restriction in policy.log_restrictions.as_deref().unwrap_or_default() {
        for ground in &restriction.legal_grounds {
            ForeignCode::read(ForeignKind::LegalGround, ground)?;
        }
    }
    if let Some(retention) = &policy.log_retention {
        for bound in retention
            .floors
            .iter()
            .flatten()
            .chain(retention.ceilings.iter().flatten())
        {
            ForeignCode::read(ForeignKind::LegalGround, &bound.ground)?;
        }
    }
    crate::access_policy::retention::log_retention(policy.log_retention.as_ref(), None)?
        .policy()?;
    if policy.controller.r#type != CONTROLLER_TYPE {
        return Err(PolicyDefect::Controller(policy.controller.r#type.clone()));
    }
    if let Some(path) = crate::access_policy::undecided::in_policy(&policy) {
        return Err(PolicyDefect::Undecided(path));
    }
    Ok(policy)
}

/// Reads every care role and organisation type of `relationship_validity`
/// and every scope of `relationship_writers`, none of which the draft access
/// terminology holds, checks that each period adds to a date, and refuses
/// the controller among `team_targets`.
///
/// # Errors
/// [`PolicyDefect::NotForeign`] for a code of the draft access terminology,
/// [`PolicyDefect::RelationshipPeriod`] for a period that does not add, and
/// [`PolicyDefect::TeamTargetController`].
fn relationship_codes(policy: &AccessPolicy) -> Result<(), PolicyDefect> {
    // NOTE: our own design, a period is judged by adding it to a date, as each write adds it.
    let probe = jiff::civil::date(2000, 1, 1);
    for entry in policy.relationship_validity.as_deref().unwrap_or_default() {
        for role in entry.roles.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::CareRole, role)?;
        }
        for kind in entry.organisation_types.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::OrganisationType, kind)?;
        }
        for period in [&entry.default_period, &entry.max_period]
            .into_iter()
            .flatten()
        {
            crate::access_policy::relationship::add(probe, period)?;
        }
    }
    for writer in policy.relationship_writers.as_deref().unwrap_or_default() {
        for scope in &writer.applies_to {
            scope_codes(scope)?;
        }
    }
    let controller = crate::service::ehr::access_rule::party_key(&policy.controller);
    let names_controller = policy
        .team_targets
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|target| crate::service::ehr::access_rule::party_key(target) == controller);
    if controller.is_some() && names_controller {
        return Err(PolicyDefect::TeamTargetController);
    }
    Ok(())
}

/// Reads every role, organisation type, purpose, sensitivity, category and legal ground `rule`
/// names, none of which the draft access terminology holds.
///
/// # Errors
/// [`NotForeign`] for the first one coded in the draft access terminology.
fn rule_codes(rule: &AccessDirective) -> Result<(), NotForeign> {
    for scope in rule.applies_to.as_deref().unwrap_or_default() {
        scope_codes(scope)?;
    }
    for purpose in rule.purposes.as_deref().unwrap_or_default() {
        ForeignCode::read(ForeignKind::Purpose, purpose)?;
    }
    for data in rule.data.as_deref().unwrap_or_default() {
        for sensitivity in data.sensitivities.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::Sensitivity, sensitivity)?;
        }
        for category in data.categories.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::Category, category)?;
        }
    }
    for ground in rule.legal_grounds.as_deref().unwrap_or_default() {
        ForeignCode::read(ForeignKind::LegalGround, ground)?;
    }
    Ok(())
}

/// Reads the role and organisation type `scope` names, neither of which the
/// draft access terminology holds.
///
/// # Errors
/// [`NotForeign`] for the first one coded in the draft access terminology.
fn scope_codes(scope: &AccessActorScope) -> Result<(), NotForeign> {
    if let Some(role) = &scope.role {
        ForeignCode::read(ForeignKind::Role, role)?;
    }
    if let Some(kind) = &scope.organisation_type {
        ForeignCode::read(ForeignKind::OrganisationType, kind)?;
    }
    Ok(())
}
