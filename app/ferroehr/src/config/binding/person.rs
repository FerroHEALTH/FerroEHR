// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one `PERSON` mapping SCIM provisioning, just-in-time creation and the
//! login binding share.
//!
//! Its sections are `[authz.party_binding.person]`,
//! `[authz.party_binding.professional_role]`, `[authz.party_binding.group]`,
//! `[authz.party_binding.membership]` and
//! `[authz.party_binding.representation]`.
//!
//! It names the templates a professional's `PERSON`, the professional `ROLE`
//! and a group's `GROUP` are valid against, the identifier selectors of the
//! binding key, `userName` and `externalId`, which SCIM attribute and which
//! OpenID Connect claim goes to which path, and the representation
//! relationship a representative acts through. Names go in `identities` and
//! identifiers in `details` (RM demographic `master02-demographic_package.adoc`
//! §Names and Addresses, §Party Identification). This module holds the
//! rules a configuration can be judged by alone; the rules that read the
//! templates run once the database is reachable
//! ([`crate::service::FerroEhrService::party_mapping_errors`]). No openEHR spec
//! governs a mapping from SCIM or OpenID Connect to a demographic template:
//! our own design.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::party_binding::{
    BindingMapping, IdentifierSelector, NameSelector, PersonSelectors, SelectorError,
};
use crate::service::demographic::template::MappedPartyTemplates;

/// The OID of the Dutch BSN namespace, which no professional's selector may
/// carry: a safeguard of our own design.
pub const BSN_OID: &str = "2.16.840.1.113883.2.4.6.3";

/// The SCIM attribute that feeds the binding key (the FerroEHR extension
/// schema, in the URN form Microsoft Entra requires of a custom attribute).
pub const LOGIN_IDENTIFIER: &str =
    "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier";

/// The SCIM attributes the service provider owns, which a mapping never maps
/// (RFC 7643 §3.1, §4.1.1).
const INTRINSIC: [&str; 6] = ["id", "schemas", "meta", "active", "userName", "externalId"];

/// The claims and attributes a binding key is never fed by: a mutable or
/// reassignable value (OpenID Connect Core 1.0 §5.7; Entra `upn` and
/// `unique_name`).
const NEVER_A_KEY: [&str; 7] = [
    "email",
    "phone_number",
    "preferred_username",
    "name",
    "upn",
    "unique_name",
    "emails",
];

/// `[authz.party_binding.person]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonSection {
    /// `[[authz.party_binding.person.template]]`, one per template a
    /// professional's `PERSON` may use.
    pub template: Vec<PersonTemplate>,
    /// `[[authz.party_binding.person.attribute]]`, one per mapped value.
    pub attribute: Vec<PersonAttribute>,
}

/// One `[[authz.party_binding.person.template]]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonTemplate {
    /// An operational template in the definition store rooted at `PERSON`.
    pub template_id: String,
    /// The binding key.
    pub key: Selector,
    /// The SCIM `userName`.
    pub user_name: Option<Selector>,
    /// The SCIM `externalId`.
    pub external_id: Option<Selector>,
    /// Whether SCIM and just-in-time creation create new persons with this
    /// template; exactly one table sets it.
    pub default: bool,
}

/// An identifier selector: the path of the `ELEMENT` holding the
/// `DV_IDENTIFIER`, and its `type`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selector {
    /// The openEHR path.
    pub path: String,
    /// The `DV_IDENTIFIER.type`.
    #[serde(rename = "type")]
    pub identifier_type: String,
}

/// One `[[authz.party_binding.person.attribute]]` table: a mapped value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PersonAttribute {
    /// A SCIM attribute path, with the RFC 7644 §3.4.2.2 `valuePath` filter
    /// form (`emails[type eq "work"].value`).
    pub scim: Option<String>,
    /// An OpenID Connect Core 1.0 §5.1 claim.
    pub oidc: Option<String>,
    /// Whether just-in-time creation writes the value.
    pub jit: bool,
    /// Whether this is the name the committer reads; at most one table sets
    /// it.
    pub name: bool,
    /// The path the value goes to, per template id.
    pub path: BTreeMap<String, String>,
}

/// What an attribute carries, which decides where it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A name: it goes in `identities`.
    Name,
    /// An identifier: it goes in `details`, as a `DV_IDENTIFIER`.
    Identifier,
    /// A contact: it goes in `contacts`.
    Contact,
    /// Any other value.
    Other,
}

impl PersonAttribute {
    /// What the attribute carries, read from its SCIM attribute or claim.
    #[must_use]
    pub fn source(&self) -> Source {
        let scim = self.scim.as_deref().unwrap_or_default();
        let oidc = self.oidc.as_deref().unwrap_or_default();
        let root = scim.split(['.', '[']).next().unwrap_or_default();
        if matches!(root, "name" | "displayName" | "nickName")
            || matches!(
                oidc,
                "name" | "given_name" | "family_name" | "middle_name" | "nickname"
            )
        {
            Source::Name
        } else if matches!(root, "emails" | "phoneNumbers" | "addresses" | "ims")
            || matches!(oidc, "email" | "phone_number" | "address")
        {
            Source::Contact
        } else if scim == LOGIN_IDENTIFIER
            || scim.ends_with(":employeeNumber")
            || root == "employeeNumber"
        {
            Source::Identifier
        } else {
            Source::Other
        }
    }

    /// The label the boot refusals name the attribute by.
    #[must_use]
    pub fn label(&self) -> String {
        match (&self.scim, &self.oidc) {
            (Some(scim), _) => format!("the attribute {scim:?}"),
            (None, Some(oidc)) => format!("the claim {oidc:?}"),
            (None, None) => "an attribute".to_owned(),
        }
    }
}

/// `[authz.party_binding.professional_role]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleSection {
    /// An operational template rooted at `ROLE`.
    pub template_id: String,
}

/// `[authz.party_binding.group]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupSection {
    /// An operational template rooted at `GROUP`.
    pub template_id: String,
    /// The group's `externalId`.
    pub external_id: Selector,
    /// The path of the group's `displayName`, a `DV_TEXT` in `identities`.
    pub display_name: Option<String>,
}

/// `[authz.party_binding.membership]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MembershipSection {
    /// A `PARTY_RELATIONSHIP` archetype.
    pub archetype_id: String,
}

/// `[authz.party_binding.representation]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RepresentationSection {
    /// A `PARTY_RELATIONSHIP` archetype whose `source` is the representative
    /// and whose `target` is the subject.
    pub archetype_id: String,
    /// Each relationship type code of the archetype, mapped to a code of the
    /// openEHR `subject_relationship` group.
    pub relationship_codes: BTreeMap<String, String>,
}

/// The mapping sections of `[authz.party_binding]`.
#[derive(Debug, Clone, Copy)]
pub struct Sections<'a> {
    /// `person`.
    pub person: &'a PersonSection,
    /// `professional_role`.
    pub professional_role: Option<&'a RoleSection>,
    /// `group`.
    pub group: Option<&'a GroupSection>,
    /// `membership`.
    pub membership: Option<&'a MembershipSection>,
    /// `representation`.
    pub representation: Option<&'a RepresentationSection>,
}

impl Sections<'_> {
    /// Whether any mapping section is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.person.template.is_empty()
            && self.person.attribute.is_empty()
            && self.professional_role.is_none()
            && self.group.is_none()
            && self.membership.is_none()
            && self.representation.is_none()
    }

    /// The boot refusals a configuration can be judged by alone, each naming
    /// its key.
    ///
    /// `key_issuers` are the `identifier_issuer`s of the `[[auth.party]]`
    /// tables, and `protected_schemes` the identifier schemes whose value
    /// leaves the body (`[demographic.identifier_protection] schemes` while
    /// it is enabled).
    #[must_use]
    pub fn errors(&self, key_issuers: &[String], protected_schemes: &[String]) -> Vec<String> {
        let mut errors = Vec::new();
        if self.is_empty() {
            return errors;
        }
        self.template_errors(key_issuers, protected_schemes, &mut errors);
        self.attribute_errors(&mut errors);
        if let Some(role) = self.professional_role
            && role.template_id.trim().is_empty()
        {
            errors.push("authz.party_binding.professional_role.template_id is empty".to_owned());
        }
        if let Some(group) = self.group {
            if group.template_id.trim().is_empty() {
                errors.push("authz.party_binding.group.template_id is empty".to_owned());
            }
            if let Err(e) = identifier(&group.external_id) {
                errors.push(format!("authz.party_binding.group.external_id: {e}"));
            }
            if let Some(path) = &group.display_name
                && let Err(e) = NameSelector::new(path.clone())
            {
                errors.push(format!("authz.party_binding.group.display_name: {e}"));
            }
        }
        if let Some(membership) = self.membership
            && membership.archetype_id.trim().is_empty()
        {
            errors.push("authz.party_binding.membership.archetype_id is empty".to_owned());
        }
        if let Some(representation) = self.representation {
            representation_errors(representation, &mut errors);
        }
        errors
    }

    /// The refusals of the person templates.
    fn template_errors(
        &self,
        key_issuers: &[String],
        protected_schemes: &[String],
        errors: &mut Vec<String>,
    ) {
        let templates = &self.person.template;
        if !templates.is_empty() && templates.iter().filter(|t| t.default).count() != 1 {
            errors.push(
                "exactly one [[authz.party_binding.person.template]] sets default = true: SCIM \
                 and just-in-time creation create new persons with it"
                    .to_owned(),
            );
        }
        let mut seen = BTreeSet::new();
        for template in templates {
            let id = template.template_id.trim();
            let at = format!("[[authz.party_binding.person.template]] {id:?}");
            if id.is_empty() {
                errors.push(
                    "a [[authz.party_binding.person.template]] names no template_id".to_owned(),
                );
                continue;
            }
            if !seen.insert(crate::templates::identity::canonical_key(id)) {
                errors.push(format!("{at} is listed twice"));
            }
            let mut selectors: BTreeMap<(&str, &str), &str> = BTreeMap::new();
            for (name, selector) in template.selectors() {
                if let Err(e) = identifier(selector) {
                    errors.push(format!("{at} {name}: {e}"));
                }
                if let Some(first) = selectors.insert(
                    (selector.path.as_str(), selector.identifier_type.as_str()),
                    name,
                ) {
                    errors.push(format!(
                        "{at}: {first} and {name} are one selector (path {:?}, type {:?}); two \
                         selectors at one path are told apart by type",
                        selector.path, selector.identifier_type
                    ));
                }
                if selector.identifier_type == BSN_OID
                    || (name == "key" && key_issuers.iter().any(|issuer| issuer == BSN_OID))
                {
                    errors.push(format!(
                        "{at} {name} maps an identifier under the Dutch BSN namespace \
                         ({BSN_OID}), which a professional's template never carries"
                    ));
                }
            }
            if protected_schemes.contains(&template.key.identifier_type) {
                errors.push(format!(
                    "{at} key: the scheme {:?} is protected \
                     ([demographic.identifier_protection]), so its value leaves the body and \
                     cannot be a binding key",
                    template.key.identifier_type
                ));
            }
        }
    }

    /// The refusals of the attributes.
    fn attribute_errors(&self, errors: &mut Vec<String>) {
        let attributes = &self.person.attribute;
        if attributes.iter().filter(|a| a.name).count() > 1 {
            errors.push(
                "more than one [[authz.party_binding.person.attribute]] sets name = true: one \
                 attribute is the name the committer reads"
                    .to_owned(),
            );
        }
        let templates: BTreeSet<String> = self
            .person
            .template
            .iter()
            .map(|t| crate::templates::identity::canonical_key(&t.template_id))
            .collect();
        for attribute in attributes {
            let label = attribute.label();
            if attribute.scim.is_none() && attribute.oidc.is_none() {
                errors.push(
                    "a [[authz.party_binding.person.attribute]] names neither scim nor oidc"
                        .to_owned(),
                );
            }
            if let Some(scim) = &attribute.scim
                && INTRINSIC.contains(&scim.as_str())
            {
                errors.push(format!(
                    "{label} is intrinsic to SCIM and is never mapped (userName and externalId \
                     are the user_name and external_id selectors)"
                ));
            }
            if attribute.name && attribute.source() != Source::Name {
                errors.push(format!("{label} sets name = true but carries no name"));
            }
            for (template, path) in &attribute.path {
                if !templates.contains(&crate::templates::identity::canonical_key(template)) {
                    errors.push(format!(
                        "{label} maps a path for {template:?}, which no \
                         [[authz.party_binding.person.template]] names"
                    ));
                }
                placement_errors(attribute, &label, template, path, errors);
            }
            let feeds_key = attribute.path.iter().any(|(template, path)| {
                self.person.template.iter().any(|t| {
                    crate::templates::identity::canonical_key(&t.template_id)
                        == crate::templates::identity::canonical_key(template)
                        && same_element(&t.key.path, path)
                })
            });
            let source = attribute.oidc.as_deref().into_iter().chain(
                attribute
                    .scim
                    .as_deref()
                    .map(|s| s.split(['.', '[']).next().unwrap_or_default()),
            );
            if feeds_key && source.into_iter().any(|s| NEVER_A_KEY.contains(&s)) {
                errors.push(format!(
                    "{label} feeds the binding key, which is never fed by an email address, a \
                     phone number, preferred_username, name, upn or unique_name"
                ));
            }
        }
    }

    /// Returns the mapping the side table is derived under, with the key
    /// selectors held to `key_issuers` and the protected parties writable by
    /// `admin_role`.
    ///
    /// # Errors
    /// [`SelectorError`] when a selector path does not compile, which
    /// [`Self::errors`] reports at boot.
    pub fn binding_mapping(
        &self,
        key_issuers: &[String],
        admin_role: &str,
    ) -> Result<BindingMapping, SelectorError> {
        let mut mapping = BindingMapping::default().with_admin_role(admin_role);
        for template in &self.person.template {
            let key = identifier(&template.key)?.with_issuers(key_issuers.iter().cloned());
            let mut selectors = PersonSelectors::new(key);
            if let Some(user_name) = &template.user_name {
                selectors = selectors.with_user_name(identifier(user_name)?);
            }
            if let Some(external_id) = &template.external_id {
                selectors = selectors.with_external_id(identifier(external_id)?);
            }
            let canonical = crate::templates::identity::canonical_key(&template.template_id);
            let name = self
                .person
                .attribute
                .iter()
                .filter(|a| a.name)
                .find_map(|a| {
                    a.path
                        .iter()
                        .find(|(id, _)| crate::templates::identity::canonical_key(id) == canonical)
                        .map(|(_, path)| path.clone())
                });
            if let Some(path) = name {
                selectors = selectors.with_name(NameSelector::new(path)?);
            }
            mapping = mapping.with_person_selectors(&template.template_id, selectors);
        }
        if let Some(role) = self.professional_role {
            mapping = mapping.with_professional_role(&role.template_id);
        }
        if let Some(group) = self.group {
            mapping = mapping.with_group(&group.template_id, identifier(&group.external_id)?);
            if let Some(path) = &group.display_name {
                mapping = mapping.with_group_display_name(NameSelector::new(path.clone())?);
            }
        }
        if let Some(membership) = self.membership {
            mapping = mapping.with_membership(&membership.archetype_id);
        }
        if let Some(representation) = self.representation {
            mapping = mapping.with_representation(
                &representation.archetype_id,
                representation.relationship_codes.clone(),
            );
        }
        Ok(mapping)
    }

    /// Returns the definitions a demographic write of a mapped node is
    /// validated against.
    #[must_use]
    pub fn mapped_templates(&self) -> MappedPartyTemplates {
        let mut templates = MappedPartyTemplates::default();
        for template in &self.person.template {
            templates = templates.with_person_template(template.template_id.clone());
        }
        if let Some(role) = self.professional_role {
            templates = templates.with_role_template(role.template_id.clone());
        }
        if let Some(group) = self.group {
            templates = templates.with_group_template(group.template_id.clone());
        }
        if let Some(membership) = self.membership {
            templates = templates.with_membership_archetype(membership.archetype_id.clone());
        }
        templates
    }
}

impl PersonTemplate {
    /// The selectors of the template, by name, the key first.
    #[must_use]
    pub fn selectors(&self) -> Vec<(&'static str, &Selector)> {
        let mut selectors = vec![("key", &self.key)];
        if let Some(user_name) = &self.user_name {
            selectors.push(("user_name", user_name));
        }
        if let Some(external_id) = &self.external_id {
            selectors.push(("external_id", external_id));
        }
        selectors
    }
}

/// The compiled selector of `selector`.
fn identifier(selector: &Selector) -> Result<IdentifierSelector, SelectorError> {
    IdentifierSelector::new(selector.path.clone(), selector.identifier_type.clone())
}

/// Whether two paths name one `ELEMENT`, one of them possibly at its `value`.
fn same_element(a: &str, b: &str) -> bool {
    a.trim_end_matches("/value") == b.trim_end_matches("/value")
}

/// The refusals of where an attribute's path puts its value.
fn placement_errors(
    attribute: &PersonAttribute,
    label: &str,
    template: &str,
    path: &str,
    errors: &mut Vec<String>,
) {
    let root = path
        .trim_start_matches('/')
        .split(['/', '['])
        .next()
        .unwrap_or_default();
    let wanted = match attribute.source() {
        Source::Name => Some("identities"),
        Source::Identifier => Some("details"),
        Source::Contact => Some("contacts"),
        Source::Other => None,
    };
    if let Some(wanted) = wanted
        && root != wanted
    {
        errors.push(format!(
            "{label} maps {template:?} to {path:?}: its value goes in PARTY.{wanted} (RM \
             demographic §Names and Addresses, §Party Identification)"
        ));
    }
}

/// The refusals of the representation section.
fn representation_errors(representation: &RepresentationSection, errors: &mut Vec<String>) {
    if representation.archetype_id.trim().is_empty() {
        errors.push("authz.party_binding.representation.archetype_id is empty".to_owned());
    }
    let terminology = openehr_term::bundle::openehr();
    for (code, mapped) in &representation.relationship_codes {
        if !terminology.is_valid_code("subject_relationship", mapped) {
            errors.push(format!(
                "authz.party_binding.representation.relationship_codes maps {code:?} to \
                 {mapped:?}, which is no code of the openEHR subject_relationship group"
            ));
        }
    }
}
