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

use std::borrow::Cow;

use serde_json::Value;
use uuid::Uuid;

use super::{BindingMapping, IdentifierSelector, NameSelector, Step};
use crate::templates::identity::canonical_key;
use crate::versioning::Kind;
use crate::versioning::tree::{Shape, Tree};

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

/// One representation ([`crate::party_binding::representation`]): the
/// subject it targets, its `basis`, its `provider`, the openEHR
/// `subject_relationship` code it names and its `time_validity`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Representation {
    /// The subject's container.
    pub(crate) subject: Uuid,
    /// The openEHR `subject_relationship` code, when the relationship names
    /// one ([`crate::party_binding::policy_mark::subject_relationship`]),
    /// which a representative's `PARTY_RELATED` committer carries.
    pub(crate) code: Option<String>,
    /// Why the representative may act for the subject.
    pub(crate) basis: openehr_rm::draft::access_control::terminology::ForeignCode,
    /// The provider for which the representation is recorded.
    pub(crate) provider: crate::party_binding::grants::caller::care_of::ProviderKey,
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
    /// The representation a standalone `PARTY_RELATIONSHIP` is.
    pub(crate) representations: Vec<Representation>,
    /// The container of a standalone representation's source, the
    /// representative.
    pub(crate) representative: Option<Uuid>,
    /// The selector identifiers of a `PERSON` of a mapped template, distinct
    /// by selector, issuer and compared value, in document order.
    pub(crate) identifiers: Selected,
    /// The identifiers of a `PERSON` of the patient template, distinct by
    /// issuer, type and id, in document order.
    pub(crate) patient_identifiers: Vec<Identifier>,
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
    /// The client identifiers and granted operations the grant rows read.
    pub(crate) grants: super::grants::derive::GrantFacts,
    /// What makes the version a policy party, whatever the mapping
    /// ([`crate::party_binding::policy_mark::policy_content`]).
    pub(crate) policy: Option<crate::party_binding::policy_mark::PolicyContent>,
}

impl Facts {
    /// Derives what `body`, a party of `kind`, holds under `mapping`.
    pub(crate) fn of<'a, T: Tree<'a>>(kind: Kind, body: T, mapping: &BindingMapping) -> Self {
        let template = body
            .pointer_text("/archetype_details/template_id/value")
            .map(|id| canonical_key(&id));
        let mut facts = Self {
            template,
            grants: super::grants::derive::GrantFacts::of(kind, body),
            policy: crate::party_binding::policy_mark::policy_content(body),
            ..Self::default()
        };
        facts.relationships(kind, body, mapping);
        match kind {
            Kind::Person => facts.person(body, mapping),
            Kind::Group => facts.group(body, mapping),
            Kind::Role => {
                facts.performer = body
                    .pointer_text("/performer/id/value")
                    .map(Cow::into_owned);
                facts.validity = validity(body);
            }
            // NOTE: a standalone relationship container is listed at its target
            // like an inline one (RM demographic master02 §Party Relationships): our own design.
            Kind::PartyRelationship => {
                let target = body
                    .pointer_text("/target/id/value")
                    .and_then(|id| container(&id));
                facts.targets.extend(target);
                if crate::party_binding::policy_mark::carries(
                    body,
                    crate::party_binding::policy_mark::TERMINOLOGY,
                    crate::party_binding::policy_mark::REPRESENTS,
                ) {
                    match crate::party_binding::representation::read(body) {
                        Ok((representative, representation)) => {
                            facts.representative = Some(representative);
                            facts.representations.push(representation);
                        }
                        Err(wrong) => facts.grants.unread.extend(wrong),
                    }
                }
            }
            _ => {}
        }
        facts
    }

    /// The inline relationships: every target, and the memberships and
    /// representations among them.
    fn relationships<'a, T: Tree<'a>>(&mut self, kind: Kind, body: T, mapping: &BindingMapping) {
        let Some(relationships) = body.get("relationships") else {
            return;
        };
        let person = kind == Kind::Person;
        for (ordinal, relationship) in relationships.items().enumerate() {
            let target = relationship
                .pointer_text("/target/id/value")
                .and_then(|id| container(&id));
            let archetype = relationship
                .get_text("archetype_node_id")
                .map(|id| id.to_lowercase());
            let is = |of: Option<&String>| of.is_some() && of == archetype.as_ref();
            if crate::party_binding::policy_mark::carries(
                relationship,
                crate::party_binding::policy_mark::TERMINOLOGY,
                crate::party_binding::policy_mark::REPRESENTS,
            ) {
                self.grants
                    .unread
                    .push(crate::party_binding::representation::held_inline(ordinal));
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
    fn person<'a, T: Tree<'a>>(&mut self, body: T, mapping: &BindingMapping) {
        if let Some(roles) = body.get("roles") {
            self.roles = roles
                .items()
                .filter_map(|role| role.pointer_text("/id/value"))
                .filter_map(|id| container(&id))
                .collect();
        }
        if let Some(patient) = mapping
            .patient
            .as_ref()
            .filter(|patient| self.template.as_ref() == Some(&patient.template))
        {
            for identifier in patient.identifiers(body) {
                if !self.patient_identifiers.contains(&identifier) {
                    self.patient_identifiers.push(identifier);
                }
            }
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
    fn group<'a, T: Tree<'a>>(&mut self, body: T, mapping: &BindingMapping) {
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

/// The `DV_IDENTIFIER`s `steps` address in `body`, in document order.
fn dv_identifiers<'a, T: Tree<'a>>(body: T, steps: &[Step]) -> impl Iterator<Item = T> {
    select(body, steps)
        .into_iter()
        .filter(|value| type_of(*value).as_deref() == Some("DV_IDENTIFIER"))
}

impl IdentifierSelector {
    /// The identifiers of `body` this selector chooses.
    pub(crate) fn identifiers<'a, T: Tree<'a>>(&self, body: T) -> Vec<Identifier> {
        dv_identifiers(body, &self.parsed)
            .filter_map(|value| {
                let text = |key: &str| value.get_text(key);
                if text("type").as_deref() != Some(self.identifier_type.as_str()) {
                    return None;
                }
                let issuer = text("issuer")?;
                let id = text("id")?;
                (self.issuers.is_empty() || self.issuers.iter().any(|held| *held == issuer))
                    .then(|| Identifier::new(&issuer, &self.identifier_type, &id))
            })
            .collect()
    }
}

impl super::PatientSelector {
    /// The patient identifiers of `body` this selector chooses.
    pub(crate) fn identifiers<'a, T: Tree<'a>>(&self, body: T) -> Vec<Identifier> {
        dv_identifiers(body, &self.parsed)
            .filter_map(|value| {
                let text = |key: &str| value.get_text(key).filter(|text| !text.trim().is_empty());
                Some(Identifier::new(
                    &text("issuer")?,
                    &text("type").unwrap_or_default(),
                    &text("id")?,
                ))
            })
            .collect()
    }
}

impl NameSelector {
    /// The names of `body` this selector chooses.
    pub(crate) fn names<'a, T: Tree<'a>>(&self, body: T) -> Vec<String> {
        select(body, &self.parsed)
            .into_iter()
            .filter(|value| {
                matches!(
                    type_of(*value).as_deref(),
                    Some("DV_TEXT" | "DV_CODED_TEXT")
                )
            })
            .filter_map(|value| value.get_text("value"))
            .map(Cow::into_owned)
            .collect()
    }
}

/// The values the steps `steps` select in `body`.
fn select<'a, T: Tree<'a>>(body: T, steps: &[Step]) -> Vec<T> {
    let mut current = vec![body];
    for step in steps {
        let mut next = Vec::new();
        for value in current {
            members(value, &step.attribute, &mut next);
        }
        if let Some(node_id) = &step.node_id {
            next.retain(|value| {
                value.get_text("archetype_node_id").as_deref() == Some(node_id.as_str())
            });
        }
        current = next;
    }
    current
}

/// Pushes the members `attribute` of `value`, or of each item of an array,
/// onto `out`, an array member's items one by one.
fn members<'a, T: Tree<'a>>(value: T, attribute: &str, out: &mut Vec<T>) {
    match value.shape() {
        Shape::Array => {
            for item in value.items() {
                members(item, attribute, out);
            }
        }
        Shape::Object => match value.get(attribute) {
            Some(member) if member.shape() == Shape::Array => out.extend(member.items()),
            Some(member) => out.push(member),
            None => {}
        },
        Shape::String | Shape::Other => {}
    }
}

/// The `_type` of `value`.
fn type_of<'a, T: Tree<'a>>(value: T) -> Option<Cow<'a, str>> {
    value.get_text("_type")
}

/// The `time_validity` of `value`, kept as its canonical value.
fn validity<'a, T: Tree<'a>>(value: T) -> Option<Value> {
    value.get("time_validity").map(Tree::to_value)
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
    fn a_patient_holds_each_identifier_with_an_issuer_once() {
        let mapping = mapping()
            .with_patient("Patient.v1", "/details[id2]/items[id3]")
            .expect("parses");
        let untyped = json!({"_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.1", "id": "P-2"});
        let unissued = json!({"_type": "DV_IDENTIFIER", "type": "NI", "id": "P-3"});
        let body = json!({
            "_type": "PERSON",
            "archetype_details": {"template_id": {"value": "patient.v1"}},
            "details": {"archetype_node_id": "id2", "items": [
                {"archetype_node_id": "id3", "value": identifier("urn:oid:2.999.1", "P-1")},
                {"archetype_node_id": "id3", "value": identifier("urn:oid:2.999.1", "P-1")},
                {"archetype_node_id": "id3", "value": untyped},
                {"archetype_node_id": "id3", "value": unissued},
                {"archetype_node_id": "id4", "value": identifier("urn:oid:2.999.1", "P-4")}
            ]}
        });
        let facts = Facts::of(Kind::Person, &body, &mapping);
        assert_eq!(
            facts.patient_identifiers,
            vec![
                Identifier::new("urn:oid:2.999.1", "guid", "P-1"),
                Identifier::new("urn:oid:2.999.1", "", "P-2"),
            ]
        );
        let professional = json!({
            "_type": "PERSON",
            "archetype_details": {"template_id": {"value": "professional.v1"}},
            "details": {"archetype_node_id": "id2", "items": [
                {"archetype_node_id": "id3", "value": identifier("urn:oid:2.999.1", "P-1")}
            ]}
        });
        assert_eq!(
            Facts::of(Kind::Person, &professional, &mapping).patient_identifiers,
            Vec::new(),
            "a PERSON of another template holds no patient identifier"
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

    /// A representation of `subject` by `source` of its own, its name coded
    /// `represents`, on the basis `GUARD` for one provider.
    fn representation(source: &str, subject: &str) -> Value {
        let coded = |terminology: &str, code: &str| {
            json!({"_type": "DV_CODED_TEXT", "value": code,
                "defining_code": {"_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                    "code_string": code}})
        };
        let party = |id: &str| {
            json!({"id": {"_type": "HIER_OBJECT_ID", "value": id},
            "namespace": "demographic", "type": "PERSON"})
        };
        json!({"_type": "PARTY_RELATIONSHIP", "archetype_node_id": "org.example::proxy.v1",
            "name": coded("openehr_access", "represents"),
            "source": party(source), "target": party(subject),
            "details": {"_type": "ITEM_TREE", "archetype_node_id": "id3", "items": [
                {"_type": "ELEMENT", "archetype_node_id": "id4",
                 "name": coded("openehr_access", "basis"), "value": coded("HL7_RoleCode", "GUARD")},
                {"_type": "ELEMENT", "archetype_node_id": "id5",
                 "name": coded("openehr_access", "provider"),
                 "value": {"_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                           "id": "hospital", "type": "ORGANISATION"}}]}})
    }

    #[test]
    fn a_representation_held_in_a_persons_relationships_is_refused() {
        let subject = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";
        let body = json!({"_type": "PERSON",
            "relationships": [representation(subject, subject)]});
        let facts = Facts::of(Kind::Person, &body, &mapping());
        assert_eq!(facts.representations, Vec::<Representation>::new());
        assert_eq!(facts.grants.unread.len(), 1, "{:?}", facts.grants.unread);
        assert!(
            facts.grants.unread[0].contains("/demographic/party_relationship"),
            "{:?}",
            facts.grants.unread
        );
    }

    #[test]
    fn a_standalone_representation_names_its_source_subject_basis_and_provider() {
        let (source, subject) = (
            "0190a1bc-3a1e-72e4-b7c5-f78a24c01310",
            "0190a1bc-3a1e-72e4-b7c5-f78a24c01308",
        );
        let mut body = representation(source, subject);
        let facts = Facts::of(Kind::PartyRelationship, &body, &mapping());
        assert_eq!(facts.grants.unread, Vec::<String>::new());
        assert_eq!(
            facts.representative.map(|id| id.to_string()).as_deref(),
            Some(source)
        );
        assert_eq!(facts.representations.len(), 1);
        assert_eq!(facts.representations[0].subject.to_string(), subject);
        assert_eq!(facts.representations[0].basis.code, "GUARD");
        assert_eq!(facts.representations[0].provider.id, "hospital");
        body["details"] = Value::Null;
        body["name"] = json!({"_type": "DV_TEXT", "value": "proxy"});
        let plain = Facts::of(Kind::PartyRelationship, &body, &mapping());
        assert_eq!(plain.representative, None);
        assert_eq!(plain.representations, Vec::<Representation>::new());
        assert_eq!(
            plain.grants.unread,
            Vec::<String>::new(),
            "no representation"
        );
    }
}
