// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template layer of a demographic write: a party of a mapped template, or
//! a `PARTY_RELATIONSHIP` of the mapped membership or acts-for archetype, is
//! validated against its operational template.
//!
//! The shared party mapping (SCIM, just-in-time creation and the login
//! binding) names the templates its `PERSON`s and `GROUP`s are written under
//! and the archetype of a membership. A write of such a node takes the
//! template-driven archetype-conformance pass the COMPOSITION commit takes,
//! with the same `553|incomplete|` relaxation; every other party keeps RM
//! validation only, so parties created through the demographic API under other
//! templates keep working. Which nodes are validated is our own design: no
//! openEHR spec governs it.
//!
//! Spec oracles:
//! - RM common `master06-change_control_package.adoc` §Incomplete Content:
//!   "If being committed in the `complete` state, such data would be treated
//!   as invalid by the repository and rejected by the API"; incomplete data
//!   "respects the same template and archetype(s), but with all existence and
//!   cardinality lower limits set to zero"; "Transitioning from `incomplete`
//!   to `complete` states requires any missing fields to be populated". RM
//!   Release 1.1.0 carries the rule (SPECRM-97, `master00-amendment_record.adoc`)
//!   and 1.2.0 keeps it, so one rule serves both `spec_profile`s.
//! - RM demographic `master02-demographic_package.adoc` §Archetyping: a party
//!   is "a completely archetypable structure", rooted at its own class.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::sync::Arc;

use openehr_base::validate::InvariantViolation;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::ValidationMessage;
use serde_json::Value;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::demographic::validate::{party_check, relationship_check};
use crate::templates::identity::canonical_key;
use crate::versioning::Kind;

/// The RM class of a membership.
const RELATIONSHIP: &str = "PARTY_RELATIONSHIP";

/// The demographic definitions the shared party mapping writes under.
///
/// Each entry is an id the definition store resolves: an OPT 1.4 template id,
/// or the id of an ADL 2 template or archetype, which the store compiles to
/// its OPT2. A party is of a mapped template when its
/// `archetype_details.template_id` names a person or group entry; a
/// `PARTY_RELATIONSHIP` is a membership when its `archetype_details.archetype_id`
/// is the root archetype of the membership definition. Ids compare in their
/// BASE §Composite Identifiers and Case canonical form, as the store keys them.
#[derive(Debug, Clone, Default)]
pub struct MappedPartyTemplates {
    persons: Vec<String>,
    role: Option<String>,
    group: Option<String>,
    membership: Option<String>,
    policy: Vec<String>,
    acts_for: Option<String>,
}

impl MappedPartyTemplates {
    /// Adds a template a professional's `PERSON` may be written under.
    #[must_use]
    pub fn with_person_template(mut self, template_id: impl Into<String>) -> Self {
        self.persons.push(template_id.into());
        self
    }

    /// Sets the template a professional `ROLE` is written under.
    #[must_use]
    pub fn with_role_template(mut self, template_id: impl Into<String>) -> Self {
        self.role = Some(template_id.into());
        self
    }

    /// Sets the template a provisioned `GROUP` is written under.
    #[must_use]
    pub fn with_group_template(mut self, template_id: impl Into<String>) -> Self {
        self.group = Some(template_id.into());
        self
    }

    /// Sets the archetype of a group membership `PARTY_RELATIONSHIP`.
    #[must_use]
    pub fn with_membership_archetype(mut self, archetype_id: impl Into<String>) -> Self {
        self.membership = Some(archetype_id.into());
        self
    }

    /// Adds a template a policy party, the system `AGENT` or a policy
    /// `ROLE`, is written under.
    #[must_use]
    pub fn with_policy_template(mut self, template_id: impl Into<String>) -> Self {
        self.policy.push(template_id.into());
        self
    }

    /// Sets the archetype of an acts-for `PARTY_RELATIONSHIP`.
    #[must_use]
    pub fn with_acts_for_archetype(mut self, archetype_id: impl Into<String>) -> Self {
        self.acts_for = Some(archetype_id.into());
        self
    }

    /// Whether `template_id` is one of the party templates.
    fn names(&self, template_id: &str) -> bool {
        let key = canonical_key(template_id);
        self.persons
            .iter()
            .chain(&self.role)
            .chain(&self.group)
            .chain(&self.policy)
            .any(|mapped| canonical_key(mapped) == key)
    }
}

/// How a demographic body fits its template, when it fits at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateFit {
    /// Every constraint holds: the body may commit `532|complete|`.
    Complete,
    /// Only the relaxed reading holds, with every existence and cardinality
    /// lower limit at zero: the body may commit `553|incomplete|`, and each
    /// violation names a path the `complete` reading refuses.
    Incomplete(Vec<InvariantViolation>),
}

impl FerroEhrService {
    /// Install the demographic definitions of the shared party mapping.
    ///
    /// Without it no party is validated against a template, which is the
    /// posture of a deployment without party binding.
    #[must_use]
    pub fn with_mapped_party_templates(mut self, templates: MappedPartyTemplates) -> Self {
        self.party_templates = Arc::new(templates);
        self
    }

    /// Install the part of the shared party mapping the binding side table is
    /// derived under ([`crate::party_binding::store`]).
    ///
    /// Without it no party has a row, so every login is unbound.
    #[must_use]
    pub fn with_binding_mapping(mut self, mapping: crate::party_binding::BindingMapping) -> Self {
        self.binding = Arc::new(mapping);
        self
    }

    /// The party source of the login binding, reading the side table on this
    /// service's party pool and naming each party under `namespace`.
    #[must_use]
    pub fn party_source(
        &self,
        namespace: impl Into<String>,
    ) -> crate::party_binding::source::LocalPartySource {
        crate::party_binding::source::LocalPartySource::new(
            self.demographic_pool.clone(),
            namespace,
        )
        .with_mapping(Arc::clone(&self.binding))
    }

    /// Rebuild the binding side table from the party versions alone, in one
    /// transaction on the party pool.
    ///
    /// # Errors
    /// [`ServiceError`] when the party pool fails.
    pub async fn refill_binding_table(&self) -> Result<(), ServiceError> {
        let mut tx = self.demographic_pool.begin().await?;
        crate::party_binding::store::refill(&mut tx, &self.binding).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Judge a demographic body against the definition `template_id` names,
    /// under both lifecycle readings of RM common master06 §Incomplete Content.
    ///
    /// The template layer only: the RM class invariants are the commit's own
    /// gate. SCIM and just-in-time creation call this before they choose the
    /// lifecycle state of a write.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) naming each failing path when
    /// the body is wrong even with every lower limit at zero, or is not of the
    /// template's root class; the [`Self::web_template`] refusals when the
    /// definition does not resolve.
    pub async fn party_template_fit(
        &self,
        body: &Value,
        template_id: &str,
    ) -> Result<TemplateFit, ServiceError> {
        let declared = body.get("_type").and_then(Value::as_str);
        fit(declared, body, &*self.web_template(template_id).await?)
    }

    /// The commit check of a demographic version on the raw-body CONTRIBUTION
    /// lane: the full RM check including the decode, which the direct routes
    /// make at their typed door, then the template gate.
    ///
    /// # Errors
    /// The [`party_check`] or [`relationship_check`] refusal, then the
    /// [`Self::mapped_template_gate`] one.
    pub(crate) async fn demographic_commit_check(
        &self,
        kind: Kind,
        data: &Value,
        incomplete: bool,
    ) -> Result<(), ServiceError> {
        if kind == Kind::PartyRelationship {
            relationship_check(data, incomplete)?;
        } else {
            party_check(kind.as_str(), data, incomplete)?;
        }
        self.mapped_template_gate(kind.as_str(), data, incomplete)
            .await
    }

    /// The template gate of a demographic commit of `rm_type`: a mapped node is
    /// refused unless it fits its template under the commit's lifecycle state,
    /// and every other node passes untouched.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) when a mapped node is wrong,
    /// or is missing data under a `complete` commit; the
    /// [`Self::web_template`] refusals when its definition does not resolve.
    pub(crate) async fn mapped_template_gate(
        &self,
        rm_type: &str,
        body: &Value,
        incomplete: bool,
    ) -> Result<(), ServiceError> {
        let Some(wt) = self.mapped_definition(rm_type, body).await? else {
            return Ok(());
        };
        match fit(Some(rm_type), body, &wt)? {
            TemplateFit::Complete => Ok(()),
            TemplateFit::Incomplete(_) if incomplete => Ok(()),
            TemplateFit::Incomplete(missing) => Err(refusal(missing)),
        }
    }

    /// The definition a mapped node is validated against, `None` for a node
    /// the mapping does not name.
    async fn mapped_definition(
        &self,
        rm_type: &str,
        body: &Value,
    ) -> Result<Option<Arc<WebTemplate>>, ServiceError> {
        if rm_type == RELATIONSHIP {
            let mapped = self.party_templates.membership.iter();
            for archetype in mapped.chain(&self.party_templates.acts_for) {
                let wt = self.web_template(archetype).await?;
                let is_mapped = body
                    .pointer("/archetype_details/archetype_id/value")
                    .and_then(Value::as_str)
                    .zip(wt.tree.node_id.as_deref())
                    .is_some_and(|(id, root)| canonical_key(id) == canonical_key(root));
                if is_mapped {
                    return Ok(Some(wt));
                }
            }
            return Ok(None);
        }
        match body
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
        {
            Some(template_id) if self.party_templates.names(template_id) => {
                Ok(Some(self.web_template(template_id).await?))
            }
            _ => Ok(None),
        }
    }
}

/// Judge `body` against `wt` under the relaxed and the full reading.
///
/// `declared` is the body's RM class, `None` for a root whose `_type` is
/// legitimately absent (canonical JSON requires it only on polymorphic slots),
/// which is then read as the template's root class.
fn fit(
    declared: Option<&str>,
    body: &Value,
    wt: &WebTemplate,
) -> Result<TemplateFit, ServiceError> {
    let root = wt
        .tree
        .rm_type
        .split('<')
        .next()
        .unwrap_or(&wt.tree.rm_type);
    if let Some(rm_type) = declared.filter(|rm_type| *rm_type != root) {
        return Err(refusal(vec![InvariantViolation::at(
            "/",
            format!(
                "template {} is rooted at {root}, so it cannot describe a {rm_type}",
                wt.template_id
            ),
        )]));
    }
    let wrong = openehr_sdt::flat::validation::validate_archetype_conformance_incomplete(body, wt);
    if !wrong.is_empty() {
        return Err(refusal(violations(wrong)));
    }
    let missing = openehr_sdt::flat::validation::validate_archetype_conformance(body, wt);
    Ok(if missing.is_empty() {
        TemplateFit::Complete
    } else {
        TemplateFit::Incomplete(violations(missing))
    })
}

fn violations(messages: Vec<ValidationMessage>) -> Vec<InvariantViolation> {
    messages
        .into_iter()
        .map(|m| InvariantViolation::at(m.path, m.message))
        .collect()
}

/// The `422` of a template refusal, counted under the `template` pass the
/// COMPOSITION commit counts its own under.
fn refusal(violations: Vec<InvariantViolation>) -> ServiceError {
    crate::telemetry::metrics::metrics()
        .validation_failures
        .add(
            u64::try_from(violations.len()).unwrap_or(u64::MAX),
            &[opentelemetry::KeyValue::new("pass", "template")],
        );
    ServiceError::ValidationFailed(violations)
}
