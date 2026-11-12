// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The local party source of the login binding: the binding side table the
//! lookup reads on every request, and the mapping its rows are derived under.
//!
//! `ferrohealth-smart` owns the binding rules (the match, the duplicate
//! refusal and the typed outcomes); FerroEHR owns the read. A login's key is
//! looked up among the binding-key identifiers of the `PERSON`s of the mapped
//! templates, each at the key selector's path and `DV_IDENTIFIER.type`
//! (RM demographic `master02-demographic_package.adoc` §Party Identification:
//! identifiers are "recorded as part of the data in the `PARTY._details_`
//! structure"), never `PARTY.identities`, which holds names.
//!
//! [`store`] derives the side-table rows from the party versions in each
//! commit's transaction and refills them from the versions alone; [`source`]
//! implements [`ferrohealth_smart::party::source::PartySource`] over them,
//! with no cache. No openEHR spec governs resolving a login to a party: our
//! own design.

pub mod check;
pub(crate) mod derive;
pub mod fill;
pub mod lookup;
pub mod policy;
pub mod protect;
pub mod provisioned;
mod rows;
pub mod source;
pub mod store;

use std::collections::BTreeMap;

use crate::templates::identity::canonical_key;

/// Why a selector path cannot be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    /// The path is not an absolute openEHR path of attribute steps, each with
    /// at most one `archetype_node_id` predicate.
    #[error(
        "the selector path {path:?} is not an absolute path of attribute steps with an optional \
         node id predicate each, such as /details[at0001]/items[at0002]"
    )]
    Path {
        /// The path as written.
        path: String,
    },
    /// The `DV_IDENTIFIER.type` of the selector is empty.
    #[error("the selector at {path:?} names no DV_IDENTIFIER type")]
    Type {
        /// The path as written.
        path: String,
    },
}

/// One identifier selector: the openEHR path of the `ELEMENT` whose value is
/// the `DV_IDENTIFIER`, and that identifier's `type`.
///
/// Every identifier of a party can sit at one path (the repeated element of an
/// identifier cluster), so a selector is never a path alone. The path is
/// parsed once into its steps, which `IdentifierSelector::identifiers`
/// walks over a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierSelector {
    /// The path as written.
    path: String,
    /// The `DV_IDENTIFIER.type` the selector holds.
    identifier_type: String,
    /// The parsed path.
    parsed: Vec<Step>,
    /// The `DV_IDENTIFIER.issuer`s the selector holds, sorted; empty holds
    /// every issuer.
    issuers: Vec<String>,
}

impl IdentifierSelector {
    /// Returns the selector of the identifiers of `identifier_type` at
    /// `path`, an absolute openEHR path such as
    /// `/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1]/items[at0001]`.
    ///
    /// The path names the `ELEMENT` holding the identifier, or that
    /// element's `value`.
    ///
    /// # Errors
    /// [`SelectorError`] naming the path when a step is not an attribute name
    /// with an optional node id predicate, or the type is empty.
    pub fn new(
        path: impl Into<String>,
        identifier_type: impl Into<String>,
    ) -> Result<Self, SelectorError> {
        let path = path.into();
        let identifier_type = identifier_type.into();
        if identifier_type.trim().is_empty() {
            return Err(SelectorError::Type { path });
        }
        let Some(parsed) = parse(&path) else {
            return Err(SelectorError::Path { path });
        };
        Ok(Self {
            path,
            identifier_type,
            parsed,
            issuers: Vec::new(),
        })
    }

    /// Returns the path as written.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the `DV_IDENTIFIER.type` the selector holds.
    #[must_use]
    pub fn identifier_type(&self) -> &str {
        &self.identifier_type
    }

    /// Returns this selector holding only the identifiers whose
    /// `DV_IDENTIFIER.issuer` is one of `issuers`, so two selectors at one
    /// path and of one type are told apart by issuer.
    #[must_use]
    pub fn with_issuers<I, S>(mut self, issuers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut issuers: Vec<String> = issuers.into_iter().map(Into::into).collect();
        issuers.sort();
        issuers.dedup();
        self.issuers = issuers;
        self
    }

    /// Returns the issuers the selector holds; empty holds every issuer.
    #[must_use]
    pub fn issuers(&self) -> &[String] {
        &self.issuers
    }

    /// The steps of the path: each attribute with its node id predicate.
    #[must_use]
    pub fn steps(&self) -> Vec<(String, Option<String>)> {
        steps(&self.path)
    }
}

/// A selector of the `DV_TEXT` at a path: the `displayName` of a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSelector {
    /// The path as written.
    path: String,
    /// The parsed path.
    parsed: Vec<Step>,
    /// The compiled SQL/JSON path.
    json_path: String,
    /// The compiled path's variables, as a JSON object text.
    variables: String,
}

impl NameSelector {
    /// Returns the selector of the `DV_TEXT` (or `DV_CODED_TEXT`) values at
    /// `path`, the `ELEMENT` holding the name or its `value`.
    ///
    /// # Errors
    /// [`SelectorError::Path`] naming the path when a step is not an
    /// attribute name with an optional node id predicate.
    pub fn new(path: impl Into<String>) -> Result<Self, SelectorError> {
        let path = path.into();
        let (Some(parsed), Some((json_path, variables))) = (parse(&path), compile(&path, TEXT))
        else {
            return Err(SelectorError::Path { path });
        };
        Ok(Self {
            path,
            parsed,
            json_path,
            variables,
        })
    }

    /// Returns the path as written.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}
/// The filter of a compiled path that keeps a `DV_TEXT` or `DV_CODED_TEXT`.
const TEXT: &str = " ? (@._type == \"DV_TEXT\" || @._type == \"DV_CODED_TEXT\")";

/// The selectors of one person template: the binding key, and the SCIM
/// `userName` and `externalId` when the mapping names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonSelectors {
    /// The binding key.
    key: IdentifierSelector,
    /// The SCIM `userName`.
    user_name: Option<IdentifierSelector>,
    /// The SCIM `externalId`.
    external_id: Option<IdentifierSelector>,
    /// The name the committer reads.
    name: Option<NameSelector>,
}

impl PersonSelectors {
    /// Returns the selectors of a template binding by `key` alone.
    #[must_use]
    pub fn new(key: IdentifierSelector) -> Self {
        Self {
            key,
            user_name: None,
            external_id: None,
            name: None,
        }
    }

    /// Returns these selectors with the SCIM `userName` at `selector`.
    #[must_use]
    pub fn with_user_name(mut self, selector: IdentifierSelector) -> Self {
        self.user_name = Some(selector);
        self
    }

    /// Returns these selectors with the committer's name at `selector`.
    #[must_use]
    pub fn with_name(mut self, selector: NameSelector) -> Self {
        self.name = Some(selector);
        self
    }

    /// Returns these selectors with the SCIM `externalId` at `selector`.
    #[must_use]
    pub fn with_external_id(mut self, selector: IdentifierSelector) -> Self {
        self.external_id = Some(selector);
        self
    }

    /// The selectors as `(side-table selector, selector)` pairs, the key
    /// first.
    fn each(&self) -> impl Iterator<Item = (&'static str, &IdentifierSelector)> {
        std::iter::once(("key", &self.key))
            .chain(self.user_name.iter().map(|s| ("user_name", s)))
            .chain(self.external_id.iter().map(|s| ("external_id", s)))
    }
}

/// The selectors of the provisioned group template: its `externalId`, and its
/// `displayName` when the mapping names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GroupSelectors {
    /// The canonical template id.
    template: String,
    /// The `externalId`.
    external_id: IdentifierSelector,
    /// The `displayName`.
    display_name: Option<NameSelector>,
}

/// What a SCIM Group is written with: the group template, its `externalId`
/// and `displayName` selectors, and the membership archetype.
#[derive(Debug, Clone, Copy)]
pub struct GroupMapping<'a> {
    /// The canonical id of the group template.
    pub template: &'a str,
    /// The `externalId` selector.
    pub external_id: &'a IdentifierSelector,
    /// The `displayName` selector.
    pub display_name: &'a NameSelector,
    /// The canonical id of the membership archetype.
    pub membership: &'a str,
}

/// One step of a selector path: an attribute and its optional
/// `archetype_node_id` predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    /// The RM attribute name.
    attribute: String,
    /// The node id the step keeps, when it names one.
    node_id: Option<String>,
}

/// Parses `path`, an absolute path of attribute steps each with at most one
/// node id predicate, ending with a step that reads the `value` of the
/// `ELEMENT` it names unless the path already ends at `value`.
fn parse(path: &str) -> Option<Vec<Step>> {
    let rest = path.strip_prefix('/')?;
    let mut parsed = Vec::new();
    for step in rest.split('/') {
        let (attribute, node_id) = match step.split_once('[') {
            Some((attribute, predicate)) => (attribute, Some(predicate.strip_suffix(']')?)),
            None => (step, None),
        };
        if !attribute_name(attribute) || node_id.is_some_and(|id| !node_id_text(id)) {
            return None;
        }
        parsed.push(Step {
            attribute: attribute.to_owned(),
            node_id: node_id.map(str::to_owned),
        });
    }
    if parsed
        .last()
        .is_none_or(|last| last.attribute != "value" || last.node_id.is_some())
    {
        parsed.push(Step {
            attribute: "value".to_owned(),
            node_id: None,
        });
    }
    Some(parsed)
}

/// Compiles `path` to a lax SQL/JSON path that yields every value it
/// addresses that passes `filter`, with the node ids as `$pN` variables (PostgreSQL 18, "The
/// SQL/JSON Path Language": in lax mode `[*]` treats a non-array as a
/// one-element array, so a single and a multiple attribute read alike).
fn compile(path: &str, filter: &str) -> Option<(String, String)> {
    let mut json_path = String::from("$");
    let mut variables = BTreeMap::new();
    for step in parse(path)? {
        json_path.push('.');
        json_path.push_str(&step.attribute);
        json_path.push_str("[*]");
        if let Some(node_id) = step.node_id {
            let name = format!("p{}", variables.len());
            json_path.push_str(" ? (@.archetype_node_id == $");
            json_path.push_str(&name);
            json_path.push(')');
            variables.insert(name, node_id);
        }
    }
    json_path.push_str(filter);
    let variables = serde_json::to_string(&variables).ok()?;
    Some((json_path, variables))
}

/// The steps of `path`, an absolute path [`compile`] accepts: each attribute
/// with its node id predicate.
fn steps(path: &str) -> Vec<(String, Option<String>)> {
    path.trim_start_matches('/')
        .split('/')
        .map(|step| match step.split_once('[') {
            Some((attribute, predicate)) => (
                attribute.to_owned(),
                Some(predicate.trim_end_matches(']').to_owned()),
            ),
            None => (step.to_owned(), None),
        })
        .collect()
}

/// Whether `name` is an RM attribute name: lower-case snake case.
fn attribute_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether `node_id` is a node id or an archetype id: no predicate operator,
/// quote or separator in it.
fn node_id_text(node_id: &str) -> bool {
    !node_id.is_empty()
        && node_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The part of the shared party mapping the binding side table is derived
/// under.
///
/// It names the templates a professional's `PERSON` may use with the key
/// selector of each, the professional `ROLE` template, the group template
/// with its `externalId` selector, and the membership and representation
/// archetypes.
///
/// Empty by default, which derives no row, so every login is unbound until a
/// deployment installs its mapping. Template and archetype ids compare in
/// their BASE §Composite Identifiers and Case canonical form, as the
/// definition store keys them.
#[derive(Debug, Clone, Default)]
pub struct BindingMapping {
    /// The person templates, by canonical template id, with their selectors.
    persons: BTreeMap<String, PersonSelectors>,
    /// The canonical template id of the professional `ROLE`.
    professional_role: Option<String>,
    /// The provisioned `GROUP` template with its selectors.
    group: Option<GroupSelectors>,
    /// The canonical archetype id of a membership `PARTY_RELATIONSHIP`.
    membership: Option<String>,
    /// The canonical archetype id of a representation `PARTY_RELATIONSHIP`.
    representation: Option<String>,
    /// Each representation type code, mapped to its openEHR
    /// `subject_relationship` code.
    relationship_codes: BTreeMap<String, String>,
    /// The role whose holder may write a protected party
    /// ([`protect`]), compared case-insensitively.
    admin_role: Option<String>,
    /// The system `AGENT` template with its client selector.
    agent: Option<policy::AgentSelectors>,
    /// The policy `ROLE` templates, by canonical template id, with their kind.
    policy_roles: BTreeMap<String, policy::PolicyRole>,
    /// The canonical archetype id of an acts-for `PARTY_RELATIONSHIP`.
    acts_for: Option<String>,
}

impl BindingMapping {
    /// Returns this mapping binding the `PERSON`s of `template_id` by the
    /// identifiers `key` selects.
    #[must_use]
    pub fn with_person(self, template_id: &str, key: IdentifierSelector) -> Self {
        self.with_person_selectors(template_id, PersonSelectors::new(key))
    }

    /// Returns this mapping reading the `PERSON`s of `template_id` through
    /// `selectors`.
    #[must_use]
    pub fn with_person_selectors(mut self, template_id: &str, selectors: PersonSelectors) -> Self {
        self.persons.insert(canonical_key(template_id), selectors);
        self
    }

    /// Returns this mapping reading the professional `ROLE` of `template_id`.
    #[must_use]
    pub fn with_professional_role(mut self, template_id: &str) -> Self {
        self.professional_role = Some(canonical_key(template_id));
        self
    }

    /// Returns this mapping reading the `GROUP`s of `template_id`, each named
    /// by the identifier `external_id` selects.
    #[must_use]
    pub fn with_group(mut self, template_id: &str, external_id: IdentifierSelector) -> Self {
        self.group = Some(GroupSelectors {
            template: canonical_key(template_id),
            external_id,
            display_name: None,
        });
        self
    }

    /// Returns this mapping reading a group's `displayName` at `selector`;
    /// without a group template it changes nothing.
    #[must_use]
    pub fn with_group_display_name(mut self, selector: NameSelector) -> Self {
        if let Some(group) = &mut self.group {
            group.display_name = Some(selector);
        }
        self
    }

    /// Returns this mapping reading a group's members from its inline
    /// `PARTY_RELATIONSHIP`s of `archetype_id`.
    #[must_use]
    pub fn with_membership(mut self, archetype_id: &str) -> Self {
        self.membership = Some(canonical_key(archetype_id));
        self
    }

    /// Returns this mapping reading a representative's subjects from the
    /// inline `PARTY_RELATIONSHIP`s of `archetype_id`, each type code mapped
    /// by `codes` to an openEHR `subject_relationship` code.
    #[must_use]
    pub fn with_representation(
        mut self,
        archetype_id: &str,
        codes: BTreeMap<String, String>,
    ) -> Self {
        self.representation = Some(canonical_key(archetype_id));
        self.relationship_codes = codes;
        self
    }

    /// Returns this mapping letting the holders of `role` write a protected
    /// party through the demographic API.
    #[must_use]
    pub fn with_admin_role(mut self, role: impl Into<String>) -> Self {
        self.admin_role = Some(role.into());
        self
    }

    /// Returns this mapping naming `template_id` as the system `AGENT`
    /// template, whose client identifier `client` selects.
    #[must_use]
    pub fn with_agent(mut self, template_id: &str, client: IdentifierSelector) -> Self {
        self.agent = Some(policy::AgentSelectors {
            template: canonical_key(template_id),
            client,
        });
        self
    }

    /// Returns this mapping naming `template_id` as the policy `ROLE`
    /// template of `kind`.
    #[must_use]
    pub fn with_policy_role(mut self, kind: policy::PolicyRole, template_id: &str) -> Self {
        self.policy_roles.insert(canonical_key(template_id), kind);
        self
    }

    /// Returns this mapping naming `archetype_id` as the acts-for
    /// `PARTY_RELATIONSHIP` archetype.
    #[must_use]
    pub fn with_acts_for(mut self, archetype_id: &str) -> Self {
        self.acts_for = Some(canonical_key(archetype_id));
        self
    }

    /// Whether the mapping names a policy party: the system `AGENT`, a policy
    /// `ROLE` or the acts-for relationship.
    #[must_use]
    pub fn names_policy(&self) -> bool {
        self.agent.is_some() || !self.policy_roles.is_empty() || self.acts_for.is_some()
    }

    /// The kind of the policy `ROLE` template `template_id` (in any case),
    /// when the mapping names it.
    #[must_use]
    pub fn policy_role(&self, template_id: &str) -> Option<policy::PolicyRole> {
        self.policy_roles.get(&canonical_key(template_id)).copied()
    }

    /// The binding-key selector of the person template `template_id`, in any
    /// case.
    #[must_use]
    pub fn key_selector(&self, template_id: &str) -> Option<&IdentifierSelector> {
        self.persons
            .get(&canonical_key(template_id))
            .map(|selectors| &selectors.key)
    }

    /// Whether the mapping derives any row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.persons.is_empty() && self.group.is_none()
    }

    /// Whether the mapping names `template_id` (in any case) as a person,
    /// role, group, agent or policy role template.
    #[must_use]
    pub fn names_template(&self, template_id: &str) -> bool {
        let key = canonical_key(template_id);
        self.persons.contains_key(&key)
            || self.professional_role.as_ref() == Some(&key)
            || self.group.as_ref().is_some_and(|g| g.template == key)
            || self.agent.as_ref().is_some_and(|a| a.template == key)
            || self.policy_roles.contains_key(&key)
    }

    /// The canonical id of the professional role template, when the mapping
    /// names one.
    #[must_use]
    pub fn professional_role(&self) -> Option<&str> {
        self.professional_role.as_deref()
    }

    /// Whether `template_id` (in any case) is the professional role
    /// template.
    #[must_use]
    pub fn is_professional_role(&self, template_id: &str) -> bool {
        self.professional_role.as_deref() == Some(canonical_key(template_id).as_str())
    }

    /// What a SCIM Group is written with, when the mapping names all of it.
    #[must_use]
    pub fn group_mapping(&self) -> Option<GroupMapping<'_>> {
        let group = self.group.as_ref()?;
        Some(GroupMapping {
            template: &group.template,
            external_id: &group.external_id,
            display_name: group.display_name.as_ref()?,
            membership: self.membership.as_deref()?,
        })
    }

    /// Returns the digest of everything the side-table rows are derived
    /// under: two mappings with one digest derive the same rows.
    ///
    /// It is the SHA-256 of a canonical text of the templates, selectors,
    /// archetypes and relationship codes, in lower-case hex.
    #[must_use]
    pub fn digest(&self) -> String {
        use sha2::Digest as _;
        let mut text = String::new();
        let mut line = |parts: &[&str]| {
            text.push_str(&parts.join("\u{1f}"));
            text.push('\n');
        };
        for (template, selectors) in &self.persons {
            for (selector, chosen) in selectors.each() {
                line(&[
                    "person",
                    template,
                    selector,
                    chosen.path(),
                    chosen.identifier_type(),
                    &chosen.issuers().join(","),
                ]);
            }
        }
        if let Some(role) = &self.professional_role {
            line(&["role", role]);
        }
        if let Some(group) = &self.group {
            let external = &group.external_id;
            line(&[
                "group",
                &group.template,
                external.path(),
                external.identifier_type(),
                &external.issuers().join(","),
                group.display_name.as_ref().map_or("", NameSelector::path),
            ]);
        }
        if let Some(membership) = &self.membership {
            line(&["membership", membership]);
        }
        if let Some(representation) = &self.representation {
            line(&["representation", representation]);
        }
        for (code, mapped) in &self.relationship_codes {
            line(&["relationship", code, mapped]);
        }
        if let Some(agent) = &self.agent {
            let client = &agent.client;
            line(&[
                "agent",
                &agent.template,
                client.path(),
                client.identifier_type(),
            ]);
        }
        for (template, kind) in &self.policy_roles {
            line(&["policy_role", kind.as_str(), template]);
        }
        if let Some(acts_for) = &self.acts_for {
            line(&["acts_for", acts_for]);
        }
        sha2::Sha256::digest(text.as_bytes()).iter().fold(
            String::with_capacity(64),
            |mut hex, byte| {
                use std::fmt::Write as _;
                // NOTE: `impl fmt::Write for String` never fails
                // (<https://doc.rust-lang.org/std/fmt/trait.Write.html#impl-Write-for-String>).
                let _written = write!(hex, "{byte:02x}");
                hex
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_parses_into_its_steps_and_reads_the_element_value() {
        let selector = IdentifierSelector::new(
            "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1]/items[at0001]",
            "guid",
        )
        .expect("parses");
        let step = |attribute: &str, node_id: Option<&str>| Step {
            attribute: attribute.to_owned(),
            node_id: node_id.map(str::to_owned),
        };
        assert_eq!(
            selector.parsed,
            vec![
                step("details", Some("at0001")),
                step(
                    "items",
                    Some("openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1")
                ),
                step("items", Some("at0001")),
                step("value", None),
            ]
        );
    }

    #[test]
    fn a_path_ending_at_the_value_reads_it_once() {
        let selector = IdentifierSelector::new("/details[at0001]/items[at0002]/value", "guid")
            .expect("parses");
        assert_eq!(
            selector
                .parsed
                .iter()
                .filter(|step| step.attribute == "value")
                .count(),
            1
        );
    }

    #[test]
    fn a_name_path_compiles_with_its_node_ids_as_variables() {
        let selector = NameSelector::new("/identities[at0001]/details[at0002]/items[at0003]")
            .expect("compiles");
        assert_eq!(
            selector.json_path,
            "$.identities[*] ? (@.archetype_node_id == $p0).details[*] ? (@.archetype_node_id \
             == $p1).items[*] ? (@.archetype_node_id == $p2).value[*] ? (@._type == \"DV_TEXT\" \
             || @._type == \"DV_CODED_TEXT\")"
        );
        assert_eq!(
            selector.variables,
            r#"{"p0":"at0001","p1":"at0002","p2":"at0003"}"#
        );
    }

    #[test]
    fn a_path_that_is_not_attribute_steps_is_refused() {
        for path in [
            "details[at0001]",
            "/details[at0001",
            "/Details[at0001]",
            "/details[at0001 and name/value='x']",
            "/details[at0001\"]",
            "//items",
            "/",
        ] {
            assert_eq!(
                IdentifierSelector::new(path, "guid"),
                Err(SelectorError::Path {
                    path: path.to_owned()
                }),
                "{path}"
            );
        }
        assert!(matches!(
            IdentifierSelector::new("/details[at0001]", " "),
            Err(SelectorError::Type { .. })
        ));
    }
}
