// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A kinship: a `PARTY_RELATIONSHIP` of its own named `related_to`.
//!
//! Its `source` and its `target` are two `PERSON`s of this store, and the
//! `kinship` element in its `details` codes how they are kin (draft access
//! control package, Care Relationships). Kinship never grants access; it
//! refuses a professional in conflict with a subject it is kin to (draft
//! decision procedure step 5). The deployment writes it and the CDR decides
//! on it (BASE Architecture Overview `master07-security.adoc` §The Problem
//! of Roles). This reads one into a caller fact about each end, so either
//! person's facts name the other. Why one cannot be read is the refusal of
//! its commit (`422`). Our own design (#6077).

use openehr_rm::draft::access_control::terminology::kinship::{Kinship, KinshipElement};
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind, Term};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use super::facts::Kin;
use super::{CallerFact, FactKind, FactValue};
use crate::party_binding::derive::container;
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::policy_mark::{carries, coded, first};
use crate::versioning::tree::Tree;

/// What one end of a kinship states: the other person and the kinship.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KinFact {
    /// The other person's `PERSON` container.
    pub(crate) person: Uuid,
    /// The kinship the relationship codes.
    pub(crate) kinship: ForeignCode,
}

impl From<&KinFact> for Kin {
    fn from(fact: &KinFact) -> Self {
        Self {
            person: fact.person,
            kinship: fact.kinship.clone(),
        }
    }
}

/// Whether `relationship` carries the `related_to` code in its `name` or its
/// `details`: a kinship.
pub(crate) fn is_kinship<'a, T: Tree<'a>>(relationship: T) -> bool {
    let term = Kinship::RelatedTo;
    carries(relationship, term.terminology(), term.code())
}

/// Why a kinship held in `relationships[ordinal]` of a party is refused: the
/// single relationship is the stored unit.
pub(crate) fn held_inline(ordinal: usize) -> String {
    format!(
        "relationships[{ordinal}]: a kinship (related_to) is a PARTY_RELATIONSHIP of its own, \
         never one of its source's relationships: write it at {}",
        super::care_of::RESOURCE
    )
}

/// Pushes onto `facts` the kinship `body`, a `PARTY_RELATIONSHIP` of its own
/// named `related_to`, states about each of its ends; or why it cannot be
/// read.
pub(crate) fn read<'a, T: Tree<'a>>(body: T, facts: &mut GrantFacts) {
    let end = |at: &str| {
        (body.pointer_text(&format!("/{at}/type")).as_deref() == Some("PERSON"))
            .then(|| body.pointer_text(&format!("/{at}/id/value")))
            .flatten()
            .and_then(|id| container(&id))
    };
    let mut wrong = Vec::new();
    let (source, target) = (end("source"), end("target"));
    if source.is_none() || target.is_none() || source == target {
        wrong.push(
            "its source and its target are two PERSONs of this store, by their containers"
                .to_owned(),
        );
    }
    let kinship = kinship(body).map_err(|why| wrong.push(why)).ok();
    if let (true, Some(source), Some(target), Some(kinship)) =
        (wrong.is_empty(), source, target, kinship)
    {
        let validity = body.get("time_validity").map(Tree::to_value);
        for (about, other) in [(source, target), (target, source)] {
            facts.facts.push(CallerFact {
                ordinal: 0,
                kind: FactKind::RelatedTo,
                value: FactValue::Kin(KinFact {
                    person: other,
                    kinship: kinship.clone(),
                }),
                validity: validity.clone(),
                not_before: Bound::Open,
                not_after: Bound::Open,
                about: Some(about),
            });
        }
        return;
    }
    facts.unread.extend(
        wrong
            .into_iter()
            .map(|why| format!("a related_to relationship: {why}")),
    );
}

/// The `kinship` of the kinship `body`: its `DV_CODED_TEXT`, of a
/// terminology other than the access terminology.
///
/// # Errors
/// Why it cannot be read: no `kinship` element, or one that is no such code.
fn kinship<'a, T: Tree<'a>>(body: T) -> Result<ForeignCode, String> {
    let value = first(body.get("details").ok_or_else(missing)?, &mut |item: T| {
        let named = item.get_text("_type").as_deref() == Some("ELEMENT")
            && item.get("name").and_then(coded).as_deref() == Some(KinshipElement::Kinship.code());
        named.then(|| item.get("value")).flatten()
    })
    .ok_or_else(missing)?;
    if value.get_text("_type").as_deref() != Some("DV_CODED_TEXT") {
        return Err("its kinship is no DV_CODED_TEXT".to_owned());
    }
    let coded: DvCodedText = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its kinship does not read: {error}"))?;
    ForeignCode::read(ForeignKind::Kinship, &coded).map_err(|error| error.to_string())
}

/// Why a kinship carries no kinship.
fn missing() -> String {
    "it carries no kinship element, which is 1..1".to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;
    use crate::versioning::Kind;

    const MOTHER: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01310";
    const NURSE: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01311";

    fn coded(terminology: &str, code: &str) -> Value {
        json!({"_type": "DV_CODED_TEXT", "value": code,
               "defining_code": {"_type": "CODE_PHRASE",
                                 "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                                 "code_string": code}})
    }

    fn person(id: &str) -> Value {
        json!({"_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
               "id": {"_type": "HIER_OBJECT_ID", "value": id}})
    }

    fn kinship(kinship: Option<Value>, target: &Value) -> Value {
        let items: Vec<Value> = kinship
            .into_iter()
            .map(|value| {
                json!({"_type": "ELEMENT", "archetype_node_id": "at2",
                                "name": coded(TERMINOLOGY, "kinship"), "value": value})
            })
            .collect();
        json!({"_type": "PARTY_RELATIONSHIP",
               "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.kinship.v0",
               "name": coded(TERMINOLOGY, "related_to"),
               "details": {"_type": "ITEM_TREE", "archetype_node_id": "at1", "items": items},
               "source": person(MOTHER), "target": target})
    }

    #[test]
    fn a_kinship_is_a_fact_about_each_of_its_ends() {
        let body = kinship(Some(coded("openehr", "254")), &person(NURSE));
        let facts = GrantFacts::of(Kind::PartyRelationship, &body);
        assert_eq!(facts.unread, Vec::<String>::new());
        let (mother, nurse) = (
            Uuid::parse_str(MOTHER).expect("a uuid"),
            Uuid::parse_str(NURSE).expect("a uuid"),
        );
        let read: Vec<(Option<Uuid>, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.about, fact.value.clone()))
            .collect();
        let of = |person| {
            FactValue::Kin(KinFact {
                person,
                kinship: ForeignCode {
                    terminology: "openehr".to_owned(),
                    code: "254".to_owned(),
                },
            })
        };
        assert_eq!(
            read,
            vec![(Some(mother), of(nurse)), (Some(nurse), of(mother))]
        );
        assert!(facts.care_source.is_none(), "a kinship puts no one in care");
    }

    #[test]
    fn a_kinship_that_does_not_read_is_refused() {
        let no_code = GrantFacts::of(Kind::PartyRelationship, &kinship(None, &person(NURSE)));
        assert_eq!(no_code.facts, Vec::new());
        assert!(
            no_code.unread[0].contains("kinship element"),
            "{:?}",
            no_code.unread
        );
        let mut group = person(NURSE);
        group["type"] = json!("GROUP");
        let defective = [
            kinship(Some(coded(TERMINOLOGY, "parent")), &person(NURSE)),
            kinship(Some(coded("openehr", "254")), &person(MOTHER)),
            kinship(Some(coded("openehr", "254")), &group),
        ];
        for body in defective {
            let facts = GrantFacts::of(Kind::PartyRelationship, &body);
            assert_eq!(facts.facts, Vec::new());
            assert!(
                facts
                    .unread
                    .first()
                    .is_some_and(|why| why.starts_with("a related_to relationship")),
                "{:?}",
                facts.unread
            );
        }
    }

    #[test]
    fn a_kinship_held_in_a_persons_relationships_is_refused() {
        let body = json!({"_type": "PERSON",
                          "relationships": [kinship(Some(coded("openehr", "254")), &person(NURSE))]});
        let facts = GrantFacts::of(Kind::Person, &body);
        assert_eq!(facts.facts, Vec::new());
        assert!(
            facts.unread[0].starts_with("relationships[0]"),
            "{:?}",
            facts.unread
        );
    }
}
