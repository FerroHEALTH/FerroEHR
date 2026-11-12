// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a party body holds that the binding side table, the
//! reverse-relationship index and the protection read, derived from the body
//! once, in Rust.
//!
//! A commit derives it from the body it holds in memory, before the body is
//! decomposed, so no commit hook parses `version.body` again (#5220); the
//! refill, an archive load and an import derive it from the stored body with
//! the same function, so the rows each writes are the rows the commit wrote.
//! The selector paths are walked as the lax SQL/JSON path of
//! [`super::NameSelector`] reads them. No openEHR spec governs the side table,
//! the index or their derivation: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the side table is derived from"
)]

use serde_json::Value;
use uuid::Uuid;

use super::{BindingMapping, IdentifierSelector, NameSelector, Step};
use crate::templates::identity::canonical_key;
use crate::versioning::Kind;

/// One `DV_IDENTIFIER` a selector holds: its issuer, its type and its id as
/// written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Identifier {
    /// The `DV_IDENTIFIER.issuer`.
    pub(crate) issuer: String,
    /// The `DV_IDENTIFIER.type`.
    pub(crate) id_type: String,
    /// The `DV_IDENTIFIER.id` as written.
    pub(crate) id: String,
}

impl Identifier {
    /// The identifier `id` of `identifier_type` from `issuer`.
    pub(crate) fn new(issuer: &str, identifier_type: &str, id: &str) -> Self {
        Self {
            issuer: issuer.to_owned(),
            id_type: identifier_type.to_owned(),
            id: id.to_owned(),
        }
    }

    /// The id as the binding key compares it: a GUID or UUID in lower case,
    /// as `ferrohealth-smart` puts a claim value, every other type as written.
    pub(crate) fn canonical(&self) -> String {
        if matches!(self.id_type.to_lowercase().as_str(), "guid" | "uuid") {
            self.id.to_lowercase()
        } else {
            self.id.clone()
        }
    }
}

/// One inline representation relationship of a `PERSON`: the subject it
/// targets, its type code and its `time_validity`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Representation {
    /// The subject's container.
    pub(crate) subject: Uuid,
    /// `name/defining_code/code_string`, when the name is coded.
    pub(crate) code: Option<String>,
    /// `time_validity`, when the relationship carries one.
    pub(crate) validity: Option<Value>,
}

/// The selector identifiers of a `PERSON` of a mapped template, each with its
/// side-table selector (`key`, `user_name` or `external_id`).
pub(crate) type Selected = Vec<(&'static str, Identifier)>;

/// What one party body holds that the side table, the reverse index and the
/// protection read.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Facts {
    /// `archetype_details.template_id` in its canonical form.
    pub(crate) template: Option<String>,
    /// The container of every inline relationship's target, sorted and
    /// distinct: the reverse-relationship index.
    pub(crate) targets: Vec<Uuid>,
    /// The containers the membership relationships of a `PERSON` target,
    /// sorted and distinct: the `GROUP`s it is a member of.
    pub(crate) groups: Vec<Uuid>,
    /// Whether an inline relationship is of the representation archetype.
    pub(crate) represents: bool,
    /// The representation relationships with a container target.
    pub(crate) representations: Vec<Representation>,
    /// The selector identifiers of a `PERSON` of a mapped template, distinct
    /// by selector, issuer and compared value, in document order.
    pub(crate) identifiers: Selected,
    /// The `externalId`s of a `GROUP` of the group template, distinct by
    /// issuer and id.
    pub(crate) group_external_ids: Vec<Identifier>,
    /// The `displayName`s of a `GROUP` of the group template.
    pub(crate) display_names: Vec<String>,
    /// The containers of the `ROLE`s a `PERSON` lists in `roles`.
    pub(crate) roles: Vec<Uuid>,
    /// The `performer` reference id of a `ROLE`, as written.
    pub(crate) performer: Option<String>,
    /// The `time_validity` of a `ROLE`.
    pub(crate) validity: Option<Value>,
}

impl Facts {
    /// Derives what `body`, a party of `kind`, holds under `mapping`.
    pub(crate) fn of(kind: Kind, body: &Value, mapping: &BindingMapping) -> Self {
        let template = body
            .get("archetype_details")
            .and_then(|details| details.get("template_id"))
            .and_then(|id| id.get("value"))
            .and_then(Value::as_str)
            .map(canonical_key);
        let mut facts = Self {
            template,
            ..Self::default()
        };
        facts.relationships(kind, body, mapping);
        match kind {
            Kind::Person => facts.person(body, mapping),
            Kind::Group => facts.group(body, mapping),
            Kind::Role => {
                facts.performer = body
                    .get("performer")
                    .and_then(|performer| performer.get("id"))
                    .and_then(|id| id.get("value"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                facts.validity = body.get("time_validity").cloned();
            }
            // NOTE: a standalone relationship container is listed at its target
            // like an inline one (RM demographic master02 §Party Relationships): our own design.
            Kind::PartyRelationship => {
                facts.targets.extend(
                    body.pointer("/target/id/value")
                        .and_then(Value::as_str)
                        .and_then(container),
                );
            }
            _ => {}
        }
        facts
    }

    /// The inline relationships: every target, and the memberships and
    /// representations among them.
    fn relationships(&mut self, kind: Kind, body: &Value, mapping: &BindingMapping) {
        let Some(relationships) = body.get("relationships").and_then(Value::as_array) else {
            return;
        };
        let person = kind == Kind::Person;
        for relationship in relationships {
            let target = relationship
                .get("target")
                .and_then(|target| target.get("id"))
                .and_then(|id| id.get("value"))
                .and_then(Value::as_str)
                .and_then(container);
            let archetype = relationship
                .get("archetype_node_id")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            let is = |of: Option<&String>| of.is_some() && of == archetype.as_ref();
            if is(mapping.representation.as_ref()) {
                self.represents = true;
                if let Some(subject) = target {
                    self.representations.push(Representation {
                        subject,
                        code: relationship
                            .get("name")
                            .and_then(|name| name.get("defining_code"))
                            .and_then(|code| code.get("code_string"))
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        validity: relationship.get("time_validity").cloned(),
                    });
                }
            }
            if let Some(target) = target {
                self.targets.push(target);
                if person && is(mapping.membership.as_ref()) {
                    self.groups.push(target);
                }
            }
        }
        for set in [&mut self.targets, &mut self.groups] {
            set.sort_unstable();
            set.dedup();
        }
    }

    /// The selector identifiers and roles of a `PERSON`.
    fn person(&mut self, body: &Value, mapping: &BindingMapping) {
        if let Some(roles) = body.get("roles").and_then(Value::as_array) {
            self.roles = roles
                .iter()
                .filter_map(|role| role.get("id")?.get("value")?.as_str())
                .filter_map(container)
                .collect();
        }
        let Some(selectors) = self
            .template
            .as_ref()
            .and_then(|template| mapping.persons.get(template))
        else {
            return;
        };
        for (selector, chosen) in selectors.each() {
            for identifier in chosen.identifiers(body) {
                let compared = |held: &Identifier| {
                    if selector == "key" {
                        held.canonical()
                    } else {
                        held.id.clone()
                    }
                };
                let value = compared(&identifier);
                if !self.identifiers.iter().any(|(s, held)| {
                    *s == selector && held.issuer == identifier.issuer && compared(held) == value
                }) {
                    self.identifiers.push((selector, identifier));
                }
            }
        }
    }

    /// The `externalId`s and `displayName`s of a `GROUP` of the group
    /// template.
    fn group(&mut self, body: &Value, mapping: &BindingMapping) {
        let Some(group) = mapping
            .group
            .as_ref()
            .filter(|g| Some(&g.template) == self.template.as_ref())
        else {
            return;
        };
        for identifier in group.external_id.identifiers(body) {
            if !self
                .group_external_ids
                .iter()
                .any(|held| held.issuer == identifier.issuer && held.id == identifier.id)
            {
                self.group_external_ids.push(identifier);
            }
        }
        if let Some(display_name) = &group.display_name {
            self.display_names = display_name.names(body);
        }
    }

    /// The binding keys `(issuer, canonical value)` among the identifiers.
    pub(crate) fn keys(&self) -> std::collections::BTreeSet<(String, String)> {
        self.identifiers
            .iter()
            .filter(|(selector, _)| *selector == "key")
            .map(|(_, identifier)| (identifier.issuer.clone(), identifier.canonical()))
            .collect()
    }
}

/// The party container a reference id names: the root of a `HIER_OBJECT_ID`
/// value (`root` or `root::extension`) in lower case, when it is a UUID in its
/// hyphenated form; a root of another `UID` form names no container of this
/// store.
pub(crate) fn container(reference: &str) -> Option<Uuid> {
    let root = reference
        .split_once("::")
        .map_or(reference, |(root, _)| root)
        .to_lowercase();
    let hyphenated = root.len() == 36
        && root.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        });
    hyphenated.then(|| Uuid::parse_str(&root).ok()).flatten()
}

impl IdentifierSelector {
    /// The identifiers the selector holds in `body`, in document order: each
    /// `DV_IDENTIFIER` of its type at its path with an issuer and an id, of
    /// one of its issuers when it names any.
    pub(crate) fn identifiers(&self, body: &Value) -> Vec<Identifier> {
        select(body, &self.parsed)
            .into_iter()
            .filter(|value| type_of(value) == Some("DV_IDENTIFIER"))
            .filter_map(|value| {
                let text = |key: &str| value.get(key).and_then(Value::as_str);
                if text("type") != Some(self.identifier_type.as_str()) {
                    return None;
                }
                let issuer = text("issuer")?;
                let id = text("id")?;
                (self.issuers.is_empty() || self.issuers.iter().any(|held| held == issuer))
                    .then(|| Identifier::new(issuer, &self.identifier_type, id))
            })
            .collect()
    }
}

impl NameSelector {
    /// The text values the selector holds in `body`, in document order: the
    /// `value` of each `DV_TEXT` or `DV_CODED_TEXT` at its path.
    pub(crate) fn names(&self, body: &Value) -> Vec<String> {
        select(body, &self.parsed)
            .into_iter()
            .filter(|value| matches!(type_of(value), Some("DV_TEXT" | "DV_CODED_TEXT")))
            .filter_map(|value| value.get("value").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }
}

/// The values `steps` address in `body`, as the lax SQL/JSON path of
/// [`super::NameSelector`] reads them: a single attribute and a multiple one
/// alike, and an array met on the way read element by element (PostgreSQL 18,
/// "The SQL/JSON Path Language", lax mode).
fn select<'a>(body: &'a Value, steps: &[Step]) -> Vec<&'a Value> {
    let mut current = vec![body];
    for step in steps {
        let mut next = Vec::new();
        for value in current {
            members(value, &step.attribute, &mut next);
        }
        if let Some(node_id) = &step.node_id {
            next.retain(|value| {
                value.get("archetype_node_id").and_then(Value::as_str) == Some(node_id.as_str())
            });
        }
        current = next;
    }
    current
}

/// Pushes onto `out` the members `attribute` names in `value`, each element
/// of an array one member.
fn members<'a>(value: &'a Value, attribute: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                members(item, attribute, out);
            }
        }
        Value::Object(map) => match map.get(attribute) {
            Some(Value::Array(items)) => out.extend(items),
            Some(member) => out.push(member),
            None => {}
        },
        _ => {}
    }
}

/// The `_type` of `value`.
fn type_of(value: &Value) -> Option<&str> {
    value.get("_type").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn identifier(issuer: &str, id: &str) -> Value {
        json!({"_type": "DV_IDENTIFIER", "issuer": issuer, "id": id, "type": "guid"})
    }

    fn mapping() -> BindingMapping {
        BindingMapping::default()
            .with_person_selectors(
                "Professional.v1",
                super::super::PersonSelectors::new(
                    IdentifierSelector::new("/details[at0001]/items[at0002]", "guid")
                        .expect("parses"),
                ),
            )
            .with_group(
                "Team.v1",
                IdentifierSelector::new("/details[id2]/items[id3]", "guid").expect("parses"),
            )
            .with_group_display_name(
                NameSelector::new("/identities[id4]/details[id5]/items[id6]").expect("parses"),
            )
            .with_membership("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1")
            .with_representation(
                "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1",
                std::collections::BTreeMap::new(),
            )
    }

    fn relationship(archetype: &str, target: &str) -> Value {
        json!({
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": archetype,
            "name": {"_type": "DV_CODED_TEXT", "value": "x",
                     "defining_code": {"code_string": "MTH"}},
            "target": {"id": {"_type": "HIER_OBJECT_ID", "value": target}}
        })
    }

    #[test]
    fn a_container_is_a_hyphenated_uuid_root_in_any_case() {
        let id = "0190A1BC-3A1E-72E4-B7C5-F78A24C01308";
        let expected = Uuid::parse_str(id).ok();
        assert_eq!(container(id), expected);
        assert_eq!(container(&format!("{id}::ferroehr::1")), expected);
        assert_eq!(container("0190a1bc3a1e72e4b7c5f78a24c01308"), None);
        assert_eq!(container("{0190a1bc-3a1e-72e4-b7c5-f78a24c01308}"), None);
        assert_eq!(container("1.2.840.113619"), None);
    }

    #[test]
    fn a_person_holds_its_keys_once_each_in_document_order() {
        let body = json!({
            "_type": "PERSON",
            "archetype_details": {"template_id": {"value": " PROFESSIONAL.v1 "}},
            "details": [{"archetype_node_id": "at0001", "items": [
                {"archetype_node_id": "at0002", "value": identifier("idp", "ABC")},
                {"archetype_node_id": "at0002", "value": identifier("idp", "abc")},
                {"archetype_node_id": "at0009", "value": identifier("idp", "other")},
                {"archetype_node_id": "at0002", "value": identifier("idp2", "def")}
            ]}],
            "roles": [{"id": {"value": "0190a1bc-3a1e-72e4-b7c5-f78a24c01308"}},
                      {"id": {"value": "not-a-container"}}]
        });
        let facts = Facts::of(Kind::Person, &body, &mapping());
        assert_eq!(facts.template.as_deref(), Some("professional.v1"));
        let held: Vec<(&str, &str)> = facts
            .identifiers
            .iter()
            .map(|(_, id)| (id.issuer.as_str(), id.id.as_str()))
            .collect();
        assert_eq!(held, vec![("idp", "ABC"), ("idp2", "def")]);
        assert_eq!(
            facts.keys(),
            [
                ("idp".to_owned(), "abc".to_owned()),
                ("idp2".to_owned(), "def".to_owned())
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(facts.roles.len(), 1);
    }

    #[test]
    fn a_group_holds_its_names_and_external_ids_and_no_membership() {
        let member = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";
        let other = "0190a1bc-3a1e-72e4-b7c5-f78a24c01309";
        let body = json!({
            "_type": "GROUP",
            "archetype_details": {"template_id": {"value": "team.v1"}},
            "details": {"archetype_node_id": "id2", "items": [
                {"archetype_node_id": "id3", "value": identifier("entra", "G1")}
            ]},
            "identities": [{"archetype_node_id": "id4", "details": {
                "archetype_node_id": "id5", "items": [{"archetype_node_id": "id6",
                    "value": {"_type": "DV_TEXT", "value": "Cardiology"}}]}}],
            "relationships": [
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1", member),
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1", member),
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.other.v1", other)
            ]
        });
        let facts = Facts::of(Kind::Group, &body, &mapping());
        assert_eq!(
            facts.groups,
            Vec::<Uuid>::new(),
            "a GROUP holds no membership"
        );
        assert_eq!(facts.targets.len(), 2);
        assert_eq!(facts.display_names, vec!["Cardiology".to_owned()]);
        assert_eq!(
            facts.group_external_ids,
            vec![Identifier::new("entra", "guid", "G1")]
        );
    }

    #[test]
    fn a_person_holds_the_groups_its_memberships_target() {
        let group = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";
        let other = "0190a1bc-3a1e-72e4-b7c5-f78a24c01309";
        let body = json!({
            "_type": "PERSON",
            "archetype_details": {"template_id": {"value": "professional.v1"}},
            "relationships": [
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1", group),
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1", group),
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.other.v1", other)
            ]
        });
        let facts = Facts::of(Kind::Person, &body, &mapping());
        assert_eq!(
            facts.groups,
            Uuid::parse_str(group).into_iter().collect::<Vec<_>>()
        );
        assert_eq!(facts.targets.len(), 2);
    }

    #[test]
    fn a_representation_is_read_with_its_code_and_marks_the_person() {
        let body = json!({
            "_type": "PERSON",
            "relationships": [
                relationship(
                    "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1",
                    "0190a1bc-3a1e-72e4-b7c5-f78a24c01308"
                ),
                relationship("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1", "x")
            ]
        });
        let facts = Facts::of(Kind::Person, &body, &mapping());
        assert!(facts.represents);
        assert_eq!(facts.representations.len(), 1);
        assert_eq!(facts.representations[0].code.as_deref(), Some("MTH"));
    }
}
