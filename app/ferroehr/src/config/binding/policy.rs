// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The mapping of the policy parties.
//!
//! Its sections are `[authz.party_binding.agent]`,
//! `[authz.party_binding.acts_for]` and the four policy `ROLE` sections,
//! `system_client_role`, `data_migration_role`, `federation_partner_role` and
//! `privileged_role`.
//!
//! They name the definitions a system's `AGENT`, the `ROLE`s whose
//! `CAPABILITY.credentials` hold a grant, and the acts-for
//! `PARTY_RELATIONSHIP` are written under (RM demographic
//! `master02-demographic_package.adoc`). A party of one of them is a policy
//! party, which only an administrator writes
//! ([`crate::party_binding::protect`]). No openEHR spec governs a policy
//! mapping: our own design.

use serde::{Deserialize, Serialize};

use crate::config::binding::person::{RoleSection, Selector};
use crate::party_binding::policy::PolicyRole;
use crate::party_binding::{BindingMapping, IdentifierSelector, SelectorError};
use crate::service::demographic::template::MappedPartyTemplates;

/// `[authz.party_binding.agent]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct AgentSection {
    /// An operational template rooted at `AGENT`.
    pub template_id: String,
    /// The client identifier: the `DV_IDENTIFIER` whose issuer is the token
    /// issuer and whose id is the OAuth 2.0 `client_id`.
    pub client: Selector,
}

/// `[authz.party_binding.acts_for]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct ActsForSection {
    /// A `PARTY_RELATIONSHIP` archetype whose `source` is a system's `AGENT`
    /// and whose `target` is the provider's `ORGANISATION`.
    pub archetype_id: String,
}

/// The policy sections of `[authz.party_binding]`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicySections<'a> {
    /// `agent`.
    pub agent: Option<&'a AgentSection>,
    /// `acts_for`.
    pub acts_for: Option<&'a ActsForSection>,
    /// `system_client_role`.
    pub system_client_role: Option<&'a RoleSection>,
    /// `data_migration_role`.
    pub data_migration_role: Option<&'a RoleSection>,
    /// `federation_partner_role`.
    pub federation_partner_role: Option<&'a RoleSection>,
    /// `privileged_role`.
    pub privileged_role: Option<&'a RoleSection>,
}

impl<'a> PolicySections<'a> {
    /// Each policy `ROLE` section that is set, with its kind and its key.
    #[must_use]
    pub fn roles(&self) -> Vec<(PolicyRole, String, &'a RoleSection)> {
        PolicyRole::ALL
            .into_iter()
            .filter_map(|kind| {
                let section = match kind {
                    PolicyRole::SystemClient => self.system_client_role,
                    PolicyRole::DataMigration => self.data_migration_role,
                    PolicyRole::FederationPartner => self.federation_partner_role,
                    PolicyRole::Privileged => self.privileged_role,
                }?;
                Some((
                    kind,
                    format!("authz.party_binding.{}_role", kind.as_str()),
                    section,
                ))
            })
            .collect()
    }

    /// Whether no policy section is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agent.is_none() && self.acts_for.is_none() && self.roles().is_empty()
    }

    /// Pushes the refusals a configuration can be judged by alone, each
    /// naming its key.
    pub fn errors(&self, errors: &mut Vec<String>) {
        if let Some(agent) = self.agent {
            if agent.template_id.trim().is_empty() {
                errors.push("authz.party_binding.agent.template_id is empty".to_owned());
            }
            if let Err(e) = client(agent) {
                errors.push(format!("authz.party_binding.agent.client: {e}"));
            }
        }
        if let Some(acts_for) = self.acts_for
            && acts_for.archetype_id.trim().is_empty()
        {
            errors.push("authz.party_binding.acts_for.archetype_id is empty".to_owned());
        }
        let mut seen: Vec<(String, String)> = Vec::new();
        for (_, key, role) in self.roles() {
            let id = role.template_id.trim();
            if id.is_empty() {
                errors.push(format!("{key}.template_id is empty"));
                continue;
            }
            let canonical = crate::templates::identity::canonical_key(id);
            if let Some((first, _)) = seen.iter().find(|(_, other)| *other == canonical) {
                errors.push(format!(
                    "{key} and {first} name one template {id:?}: each kind of grant has its own"
                ));
            }
            seen.push((key, canonical));
        }
    }

    /// Returns `mapping` naming the policy parties.
    ///
    /// # Errors
    /// [`SelectorError`] when the client selector does not compile, which
    /// [`Self::errors`] reports at boot.
    pub fn apply(&self, mut mapping: BindingMapping) -> Result<BindingMapping, SelectorError> {
        if let Some(agent) = self.agent {
            mapping = mapping.with_agent(&agent.template_id, client(agent)?);
        }
        for (kind, _, role) in self.roles() {
            mapping = mapping.with_policy_role(kind, &role.template_id);
        }
        if let Some(acts_for) = self.acts_for {
            mapping = mapping.with_acts_for(&acts_for.archetype_id);
        }
        Ok(mapping)
    }

    /// Returns `templates` with the policy definitions added, so a policy
    /// party is validated against its definition on every write.
    #[must_use]
    pub fn mapped_templates(&self, mut templates: MappedPartyTemplates) -> MappedPartyTemplates {
        if let Some(agent) = self.agent {
            templates = templates.with_policy_template(agent.template_id.clone());
        }
        for (_, _, role) in self.roles() {
            templates = templates.with_policy_template(role.template_id.clone());
        }
        if let Some(acts_for) = self.acts_for {
            templates = templates.with_acts_for_archetype(acts_for.archetype_id.clone());
        }
        templates
    }
}

/// The compiled client selector of `agent`.
fn client(agent: &AgentSection) -> Result<IdentifierSelector, SelectorError> {
    IdentifierSelector::new(
        agent.client.path.clone(),
        agent.client.identifier_type.clone(),
    )
}
