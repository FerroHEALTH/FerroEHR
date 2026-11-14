// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The strict check every `ACCESS_POLICY` passes before it is stored.
//!
//! It runs the canonical-JSON reader of the generated draft type, which refuses an
//! undeclared, missing or duplicate member and an empty optional list, then
//! the class invariants of the draft BMM
//! ([`check_policy`]), the root rules of a
//! `LOCATABLE` and of the `controller`, and that each sensitivity and
//! category a classification names, and each legal ground of a restriction of
//! the access log's subject view, comes from the jurisdiction's terminology
//! ([`ForeignCode`]).
//!
//! The draft package is FerroEHR's change proposal and no openEHR release:
//! our own design over our own model (#5512).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the policy body arrives as the canonical JSON \
              the strict typed reader decodes"
)]

use serde_json::Value;

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
    /// A sensitivity or category of a classification, or a legal ground of a
    /// log restriction, is coded in FerroEHR's own terminology, which holds
    /// none.
    #[error(transparent)]
    NotForeign(#[from] NotForeign),
    /// The controller is not an `ORGANISATION`.
    #[error("the controller is a {0:?} reference; the controller is an ORGANISATION")]
    Controller(String),
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
            .iter()
            .chain(data.sensitivities.as_deref().unwrap_or_default())
        {
            ForeignCode::read(ForeignKind::Sensitivity, sensitivity)?;
        }
        for category in data.categories.as_deref().unwrap_or_default() {
            ForeignCode::read(ForeignKind::Category, category)?;
        }
    }
    for restriction in policy.log_restrictions.as_deref().unwrap_or_default() {
        for ground in &restriction.legal_grounds {
            ForeignCode::read(ForeignKind::LegalGround, ground)?;
        }
    }
    if policy.controller.r#type != CONTROLLER_TYPE {
        return Err(PolicyDefect::Controller(policy.controller.r#type.clone()));
    }
    Ok(policy)
}
