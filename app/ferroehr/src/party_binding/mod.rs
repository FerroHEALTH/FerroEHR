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
/// compiled once to a PostgreSQL SQL/JSON path whose node ids travel as
/// variables, never as text of the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierSelector {
    /// The path as written.
    path: String,
    /// The `DV_IDENTIFIER.type` the selector holds.
    identifier_type: String,
    /// The compiled SQL/JSON path.
    json_path: String,
    /// The compiled path's variables, as a JSON object text.
    variables: String,
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
        let Some((json_path, variables)) = compile(&path) else {
            return Err(SelectorError::Path { path });
        };
        Ok(Self {
            path,
            identifier_type,
            json_path,
            variables,
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
}

/// Compiles `path` to a lax SQL/JSON path that yields every `DV_IDENTIFIER`
/// it addresses, with the node ids as `$pN` variables (PostgreSQL 18, "The
/// SQL/JSON Path Language": in lax mode `[*]` treats a non-array as a
/// one-element array, so a single and a multiple attribute read alike).
fn compile(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix('/')?;
    let mut json_path = String::from("$");
    let mut variables = BTreeMap::new();
    let mut last_is_value = false;
    for step in rest.split('/') {
        let (attribute, node_id) = match step.split_once('[') {
            Some((attribute, predicate)) => (attribute, Some(predicate.strip_suffix(']')?)),
            None => (step, None),
        };
        if !attribute_name(attribute) {
            return None;
        }
        json_path.push('.');
        json_path.push_str(attribute);
        json_path.push_str("[*]");
        if let Some(node_id) = node_id {
            if !node_id_text(node_id) {
                return None;
            }
            let name = format!("p{}", variables.len());
            json_path.push_str(" ? (@.archetype_node_id == $");
            json_path.push_str(&name);
            json_path.push(')');
            variables.insert(name, node_id.to_owned());
        }
        last_is_value = attribute == "value" && node_id.is_none();
    }
    if !last_is_value {
        json_path.push_str(".value[*]");
    }
    json_path.push_str(" ? (@._type == \"DV_IDENTIFIER\")");
    let variables = serde_json::to_string(&variables).ok()?;
    Some((json_path, variables))
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
    /// The person templates, by canonical template id, with their key
    /// selector.
    persons: BTreeMap<String, IdentifierSelector>,
    /// The canonical template id of the professional `ROLE`.
    professional_role: Option<String>,
    /// The canonical template id of a provisioned `GROUP`, with its
    /// `externalId` selector.
    group: Option<(String, IdentifierSelector)>,
    /// The canonical archetype id of a membership `PARTY_RELATIONSHIP`.
    membership: Option<String>,
    /// The canonical archetype id of a representation `PARTY_RELATIONSHIP`.
    representation: Option<String>,
    /// Each representation type code, mapped to its openEHR
    /// `subject_relationship` code.
    relationship_codes: BTreeMap<String, String>,
}

impl BindingMapping {
    /// Returns this mapping binding the `PERSON`s of `template_id` by the
    /// identifiers `key` selects.
    #[must_use]
    pub fn with_person(mut self, template_id: &str, key: IdentifierSelector) -> Self {
        self.persons.insert(canonical_key(template_id), key);
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
        self.group = Some((canonical_key(template_id), external_id));
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

    /// Whether the mapping derives any row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.persons.is_empty() && self.group.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_compiles_with_its_node_ids_as_variables() {
        let selector = IdentifierSelector::new(
            "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1]/items[at0001]",
            "guid",
        )
        .expect("compiles");
        assert_eq!(
            selector.json_path,
            "$.details[*] ? (@.archetype_node_id == $p0).items[*] ? (@.archetype_node_id == $p1)\
             .items[*] ? (@.archetype_node_id == $p2).value[*] ? (@._type == \"DV_IDENTIFIER\")"
        );
        assert_eq!(
            selector.variables,
            r#"{"p0":"at0001","p1":"openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1","p2":"at0001"}"#
        );
    }

    #[test]
    fn a_path_ending_at_the_value_reads_it_once() {
        let selector = IdentifierSelector::new("/details[at0001]/items[at0002]/value", "guid")
            .expect("compiles");
        assert!(
            selector.json_path.ends_with(
                ".items[*] ? (@.archetype_node_id == $p1).value[*] ? (@._type == \"DV_IDENTIFIER\")"
            ),
            "{}",
            selector.json_path
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
