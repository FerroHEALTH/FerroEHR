// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The strict check every `ACCESS_POLICY` passes before it is stored.
//!
//! It runs the strict canonical-JSON reader of the draft type, the class
//! invariants of the draft BMM ([`check_policy`]) and the root rules, and
//! requires every coded value a rule names to come from the jurisdiction's
//! terminology ([`ForeignCode`]), `time_zone` to name an IANA zone, and the
//! access log retention to be one the audit trail keeps records under
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
    /// `time_zone` names no zone of the IANA time zone database.
    #[error("time_zone {0:?} names no zone of the IANA time zone database")]
    TimeZone(String),
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
    for purpose in policy.care_purposes.iter().flatten() {
        ForeignCode::read(ForeignKind::Purpose, purpose)?;
    }
    if let Some(zone) = &policy.time_zone
        && jiff::tz::TimeZone::get(zone).is_err()
    {
        return Err(PolicyDefect::TimeZone(zone.clone()));
    }
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

/// Reads the role, the organisation type and the care roles `scope` names,
/// none of which the draft access terminology holds.
///
/// # Errors
/// [`NotForeign`] for the first one coded in the draft access terminology.
fn scope_codes(scope: &AccessActorScope) -> Result<(), NotForeign> {
    if let Some(role) = &scope.role {
        ForeignCode::read(ForeignKind::Role, role)?;
    }
    for role in scope.care_roles.iter().flatten() {
        ForeignCode::read(ForeignKind::CareRole, role)?;
    }
    if let Some(kind) = &scope.organisation_type {
        ForeignCode::read(ForeignKind::OrganisationType, kind)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A coded value of `terminology`.
    fn coded(terminology: &str, code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "terminology_id": { "value": terminology },
                    "code_string": code } })
    }

    /// A policy over the default archetype with `extra` members.
    fn policy(extra: &Value) -> Value {
        let mut body = json!({ "_type": "ACCESS_POLICY",
            "name": { "_type": "DV_TEXT", "value": "policy" },
            "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
            "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.2.0",
                "archetype_id": { "_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-ACCESS_POLICY.controller.v1" } },
            "controller": { "_type": "PARTY_REF", "namespace": "demographic",
                "type": "ORGANISATION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
            "care_relationship_required": true,
            "purpose_required": false });
        if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            body.extend(extra.clone());
        }
        body
    }

    /// A rule permitting a caller in care in `care_roles` by `care_path`.
    fn care_rule(role: &Value, path: &str) -> Value {
        json!({ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id1",
            "name": { "_type": "DV_TEXT", "value": "rule" },
            "kind": coded("openehr_access", "controller_rule"),
            "effect": coded("HL7_ConsentProvisionType", "permit"),
            "emergency_override": false,
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "relationship": coded("openehr_access", "care"),
                "care_roles": [role],
                "care_path": coded("openehr_access", path) }] })
    }

    /// Care roles, a care path, care purposes and a time zone are decided,
    /// so a policy setting them is stored (#6075).
    #[test]
    fn care_roles_a_path_care_purposes_and_a_time_zone_are_stored() {
        let body = policy(&json!({
            "rules": [care_rule(&coded("HL7_ParticipationType", "RESP"), "direct")],
            "care_purposes": [coded("HL7_ActReason", "TREAT")],
            "time_zone": "Europe/Amsterdam" }));
        let stored = checked(&body).expect("the policy is stored");
        assert_eq!(stored.time_zone.as_deref(), Some("Europe/Amsterdam"));
    }

    #[test]
    fn a_time_zone_the_database_does_not_hold_is_refused() {
        let body = policy(&json!({ "time_zone": "Europe/Atlantis" }));
        assert!(
            matches!(checked(&body), Err(PolicyDefect::TimeZone(zone)) if zone == "Europe/Atlantis")
        );
    }

    #[test]
    fn a_care_role_or_care_purpose_of_the_access_terminology_is_refused() {
        for body in [
            policy(&json!({ "rules": [care_rule(&coded("openehr_access", "care"), "team")] })),
            policy(&json!({ "care_purposes": [coded("openehr_access", "care")] })),
        ] {
            assert!(
                matches!(checked(&body), Err(PolicyDefect::NotForeign(_))),
                "{body}"
            );
        }
    }
}
