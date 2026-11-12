// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot rules of the shared party mapping that read its templates.
//!
//! Every template and archetype the mapping names must be in the definition
//! store, rooted at its class (`PERSON`, `ROLE`, `GROUP` or
//! `PARTY_RELATIONSHIP`), with every mapped path present and of an RM type
//! that can carry its source: a selector a `DV_IDENTIFIER`, a name a
//! `DV_TEXT`. Every relationship type code of the representation archetype
//! must be mapped to a `subject_relationship` code. No openEHR spec governs a
//! mapping from SCIM or OpenID Connect to a demographic template: our own
//! design.

use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};

use crate::config::binding::person::{Sections, Source};
use crate::error::ServiceError;
use crate::party_binding::check::{CheckError, CheckInput, Report};
use crate::party_binding::fill::{FillError, Lease};
use crate::service::FerroEhrService;

/// What a mapped path must carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carries {
    /// A `DV_IDENTIFIER`.
    Identifier,
    /// A `DV_TEXT` (or a `DV_CODED_TEXT`).
    Text,
    /// Any data value.
    Any,
}

impl Carries {
    /// Whether a value of `rm_type` fits.
    fn fits(self, rm_type: &str) -> bool {
        match self {
            Self::Identifier => rm_type == "DV_IDENTIFIER",
            Self::Text => matches!(rm_type, "DV_TEXT" | "DV_CODED_TEXT"),
            Self::Any => true,
        }
    }

    /// The RM type the refusal names.
    fn name(self) -> &'static str {
        match self {
            Self::Identifier => "DV_IDENTIFIER",
            Self::Text => "DV_TEXT",
            Self::Any => "data value",
        }
    }
}

impl FerroEhrService {
    /// Judges the shared party mapping against the definitions it names.
    ///
    /// # Errors
    /// [`ServiceError`] on a database failure, wrapped in the outer `Ok`; the
    /// inner list names every refusal, each with its key.
    pub async fn party_mapping_errors(
        &self,
        sections: &Sections<'_>,
    ) -> Result<Vec<String>, ServiceError> {
        let mut errors = Vec::new();
        for template in &sections.person.template {
            let at = format!(
                "[[authz.party_binding.person.template]] {:?}",
                template.template_id
            );
            let Some(wt) = self
                .mapped_definition_of(&at, &template.template_id, "PERSON", &mut errors)
                .await?
            else {
                continue;
            };
            for (name, selector) in template.selectors() {
                path_errors(
                    &wt,
                    &at,
                    name,
                    &selector.path,
                    Carries::Identifier,
                    &mut errors,
                );
            }
            for attribute in &sections.person.attribute {
                let Some(path) = attribute.path_for(&template.template_id) else {
                    continue;
                };
                let carries = match attribute.source() {
                    Source::Identifier => Carries::Identifier,
                    Source::Name => Carries::Text,
                    Source::Contact | Source::Other => Carries::Any,
                };
                path_errors(&wt, &at, &attribute.label(), path, carries, &mut errors);
            }
        }
        if let Some(role) = sections.professional_role {
            self.mapped_definition_of(
                "authz.party_binding.professional_role",
                &role.template_id,
                "ROLE",
                &mut errors,
            )
            .await?;
        }
        if let Some(group) = sections.group {
            let at = "authz.party_binding.group";
            if let Some(wt) = self
                .mapped_definition_of(at, &group.template_id, "GROUP", &mut errors)
                .await?
            {
                path_errors(
                    &wt,
                    at,
                    "external_id",
                    &group.external_id.path,
                    Carries::Identifier,
                    &mut errors,
                );
                if let Some(path) = &group.display_name {
                    path_errors(&wt, at, "display_name", path, Carries::Text, &mut errors);
                }
            }
        }
        if let Some(membership) = sections.membership {
            self.mapped_definition_of(
                "authz.party_binding.membership",
                &membership.archetype_id,
                "PARTY_RELATIONSHIP",
                &mut errors,
            )
            .await?;
        }
        if let Some(representation) = sections.representation {
            let at = "authz.party_binding.representation";
            if let Some(wt) = self
                .mapped_definition_of(
                    at,
                    &representation.archetype_id,
                    "PARTY_RELATIONSHIP",
                    &mut errors,
                )
                .await?
            {
                for code in relationship_codes(&wt) {
                    if !representation.relationship_codes.contains_key(&code) {
                        errors.push(format!(
                            "{at}.relationship_codes maps no subject_relationship code to the \
                             relationship type {code:?} of {:?}",
                            representation.archetype_id
                        ));
                    }
                }
            }
        }
        Ok(errors)
    }

    /// The definition `id` resolves to when it is stored and rooted at
    /// `root`; otherwise the refusal is pushed to `errors`.
    async fn mapped_definition_of(
        &self,
        at: &str,
        id: &str,
        root: &str,
        errors: &mut Vec<String>,
    ) -> Result<Option<std::sync::Arc<WebTemplate>>, ServiceError> {
        if id.trim().is_empty() {
            return Ok(None);
        }
        let wt = match self.web_template(id).await {
            Ok(wt) => wt,
            Err(
                e @ (ServiceError::Storage(_)
                | ServiceError::Database(_)
                | ServiceError::Acquire(_)),
            ) => {
                return Err(e);
            }
            Err(e) => {
                errors.push(format!(
                    "{at} names {id:?}, which the definition store does not hold or cannot \
                     resolve: {}",
                    crate::error::ErrorChain::new(&e)
                ));
                return Ok(None);
            }
        };
        let rooted = wt
            .tree
            .rm_type
            .split('<')
            .next()
            .unwrap_or(&wt.tree.rm_type);
        if rooted != root {
            errors.push(format!(
                "{at} names {id:?}, which is rooted at {rooted}, not {root}"
            ));
            return Ok(None);
        }
        Ok(Some(wt))
    }
}

/// Pushes the refusal of `path` when `wt` has no node there, or the value
/// there cannot carry `carries`.
fn path_errors(
    wt: &WebTemplate,
    at: &str,
    name: &str,
    path: &str,
    carries: Carries,
    errors: &mut Vec<String>,
) {
    let types = value_types(&wt.tree, path);
    if types.is_empty() {
        errors.push(format!(
            "{at} {name}: the template {:?} has no data value at {path:?}",
            wt.template_id
        ));
    } else if !types.iter().any(|rm_type| carries.fits(rm_type)) {
        errors.push(format!(
            "{at} {name}: the value at {path:?} is {}, which cannot carry a {}",
            types.join(" or "),
            carries.name()
        ));
    }
}

/// The RM types of the data values at `path` (an `ELEMENT` or its `value`)
/// in the tree under `node`.
///
/// A web template draws an `ELEMENT` as its data value nodes, each at the
/// element's path with `/value` appended.
pub(super) fn value_types(node: &WebTemplateNode, path: &str) -> Vec<String> {
    let wanted = format!("{}/value", path.trim_end_matches("/value"));
    let mut found = Vec::new();
    collect(node, &wanted, &mut found);
    found
}

/// Collects the RM types of the nodes at `wanted` under `node`.
fn collect(node: &WebTemplateNode, wanted: &str, found: &mut Vec<String>) {
    if plain(&node.aql_path) == wanted {
        found.push(base_type(&node.rm_type));
    }
    for child in &node.children {
        collect(child, wanted, found);
    }
}

/// `path` without the name predicates a web template adds
/// (`[at0001,'Name']`, `[at0001 and name/value='Name']`).
pub(super) fn plain(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('[') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open + 1..).unwrap_or_default();
        let close = after.find(']').unwrap_or(after.len());
        let predicate = after.get(..close).unwrap_or_default();
        let node_id = predicate.split([',', ' ']).next().unwrap_or_default();
        out.push('[');
        out.push_str(node_id);
        out.push(']');
        rest = after.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// `rm_type` without its generic parameters.
fn base_type(rm_type: &str) -> String {
    rm_type.split('<').next().unwrap_or(rm_type).to_owned()
}

/// The codes the root `name` of a relationship archetype is constrained to:
/// its relationship types.
fn relationship_codes(wt: &WebTemplate) -> Vec<String> {
    wt.tree
        .children
        .iter()
        .filter(|child| child.id == "name" || child.aql_path == "/name")
        .flat_map(|name| name.inputs.iter())
        .flat_map(|input| input.list.iter())
        .map(|coded| coded.value.clone())
        .collect()
}

impl FerroEhrService {
    /// Prepares the binding side table for the replica `replica` and takes
    /// its lease; `None` when the installed mapping derives no row.
    ///
    /// # Errors
    /// [`FillError`] when the table was filled under another mapping, a first
    /// fill finds a value two parties carry, or the party store fails.
    pub async fn take_binding_lease(
        &self,
        replica: uuid::Uuid,
    ) -> Result<Option<Lease>, FillError> {
        if self.binding.is_empty() {
            return Ok(None);
        }
        Lease::take(&self.demographic_pool, &self.binding, replica)
            .await
            .map(Some)
    }

    /// Refills the binding side table under the installed mapping once no
    /// replica's lease was renewed within `ttl` ([`crate::party_binding::fill::refill`]),
    /// or, with no mapping installed, the grant rows alone
    /// ([`crate::party_binding::fill::refill_grants`]).
    ///
    /// # Errors
    /// The [`crate::party_binding::fill::refill`] and
    /// [`crate::party_binding::fill::refill_grants`] refusals.
    pub async fn refill_binding(&self, ttl: std::time::Duration) -> Result<(), FillError> {
        if self.binding.is_empty() {
            return crate::party_binding::fill::refill_grants(&self.demographic_pool).await;
        }
        crate::party_binding::fill::refill(&self.demographic_pool, &self.binding, ttl).await
    }

    /// Honour the policy grants under `ceiling`, `[authz.grants] allowed`:
    /// a commit of a capability above it is refused and a stored one is
    /// never honoured.
    #[must_use]
    pub fn with_grant_ceiling(
        self,
        ceiling: crate::party_binding::grants::kind::GrantCeiling,
    ) -> Self {
        self.grants.set_ceiling(ceiling);
        self
    }

    /// The policy grants in force on this replica.
    #[must_use]
    pub fn grants(&self) -> &std::sync::Arc<crate::party_binding::grants::live::LiveGrants> {
        &self.grants
    }

    /// Fills the grant rows when they were never filled, then reads them, as
    /// a boot does before the replica takes traffic.
    ///
    /// # Errors
    /// [`FillError::Storage`] when the party store fails, two `AGENT`s
    /// naming one client included.
    pub async fn read_grants(&self) -> Result<(), FillError> {
        crate::party_binding::fill::fill_grants_once(&self.demographic_pool).await?;
        self.grants
            .refresh()
            .await
            .map_err(|unavailable| FillError::Storage(unavailable.into_inner()))
    }

    /// Runs the pre-flight of a migration to party binding under the installed
    /// mapping, writing nothing ([`crate::party_binding::check::check`]).
    ///
    /// # Errors
    /// [`CheckError::Unmapped`] when no mapping is installed, and the
    /// [`crate::party_binding::check::check`] refusals.
    pub async fn check_binding(&self, input: &CheckInput) -> Result<Report, CheckError> {
        if self.binding.is_empty() {
            return Err(CheckError::Unmapped);
        }
        crate::party_binding::check::check(&self.demographic_pool, &self.binding, input).await
    }
}
