// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A representation: a `PARTY_RELATIONSHIP` of its own named `represents`.
//!
//! Its `source` is the representative's `PERSON` and its `target` the
//! subject's, with the elements of the `representation element` group in
//! its `details`: `basis` and `provider` 1..1, `evidence` 0..1 (draft access
//! control package, Care Relationships, "A representation is", and The
//! Representation as a Stored Fact). The writer decides who represents
//! whom and stores the fact; the access decision reads it and never derives
//! one from an age or a capacity. The single relationship is the stored
//! unit (The Relationship Store), so one held in a `PERSON`'s
//! `relationships` is refused, naming the resource that holds it. Why one
//! cannot be read is the refusal of its commit (`422`).

use openehr_rm::draft::access_control::terminology::representation::{self, RepresentationElement};
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind, Term};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use crate::party_binding::derive::{Representation, container};
use crate::party_binding::grants::caller::care_of::{self, ProviderKey};
use crate::party_binding::policy_mark::coded;
use crate::versioning::tree::Tree;

/// The resource a representation is written at.
pub(crate) const RESOURCE: &str = care_of::RESOURCE;

/// The representation the `PARTY_RELATIONSHIP` `body` states, with the
/// container of its source, the representative; or why it cannot be read.
///
/// # Errors
/// Each reason the relationship is no readable representation.
pub(crate) fn read<'a, T: Tree<'a>>(body: T) -> Result<(Uuid, Representation), Vec<String>> {
    let mut wrong = Vec::new();
    if body.get("name").and_then(coded).as_deref()
        != Some(representation::Representation::Represents.code())
    {
        wrong.push(format!(
            "its name is the DV_CODED_TEXT {}::{}",
            representation::Representation::Represents.terminology(),
            representation::Representation::Represents.code()
        ));
    }
    let party = |end: &str| {
        let person = body.pointer_text(&format!("/{end}/type")).as_deref() == Some("PERSON");
        body.pointer_text(&format!("/{end}/id/value"))
            .and_then(|id| container(&id))
            .filter(|_| person)
    };
    let representative = party("source");
    if representative.is_none() {
        wrong.push(
            "its source is the representative's PERSON of this store, by its container".to_owned(),
        );
    }
    let subject = party("target");
    if subject.is_none() {
        wrong.push("its target is the subject's PERSON of this store, by its container".to_owned());
    }
    let basis = basis(body).map_err(|why| wrong.push(why)).ok();
    let provider = provider(body).map_err(|why| wrong.push(why)).ok();
    if let Err(why) = evidence(body) {
        wrong.push(why);
    }
    match (representative, subject, basis, provider) {
        (Some(representative), Some(subject), Some(basis), Some(provider)) if wrong.is_empty() => {
            Ok((
                representative,
                Representation {
                    subject,
                    code: crate::party_binding::policy_mark::subject_relationship(body),
                    basis,
                    provider,
                    validity: body.get("time_validity").map(Tree::to_value),
                },
            ))
        }
        _ => Err(wrong
            .into_iter()
            .map(|why| format!("a represents relationship: {why}"))
            .collect()),
    }
}

/// Why a representation held in `relationships[ordinal]` of a party is
/// refused: the single relationship is the stored unit.
pub(crate) fn held_inline(ordinal: usize) -> String {
    format!(
        "relationships[{ordinal}]: a representation (represents) is a PARTY_RELATIONSHIP of its \
         own, never one of its source's relationships: write it at {RESOURCE}"
    )
}

/// The `basis` `DV_CODED_TEXT` of the representation `body`.
///
/// # Errors
/// Why it cannot be read: no `basis` element, or one that is no such code.
pub(crate) fn basis_text<'a, T: Tree<'a>>(body: T) -> Result<DvCodedText, String> {
    let value = care_of::named_element(body, RepresentationElement::Basis.code())
        .ok_or_else(|| "it carries no basis element, which is 1..1".to_owned())?;
    if value.get_text("_type").as_deref() != Some("DV_CODED_TEXT") {
        return Err("its basis is no DV_CODED_TEXT".to_owned());
    }
    openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its basis does not read: {error}"))
}

/// The `basis` of the representation `body`: a code of a terminology other
/// than the access terminology, which codes none.
fn basis<'a, T: Tree<'a>>(body: T) -> Result<ForeignCode, String> {
    let coded = basis_text(body)?;
    ForeignCode::read(ForeignKind::RepresentationBasis, &coded).map_err(|error| error.to_string())
}

/// The `provider` of the representation `body`.
fn provider<'a, T: Tree<'a>>(body: T) -> Result<ProviderKey, String> {
    let value = care_of::named_element(body, RepresentationElement::Provider.code())
        .ok_or_else(|| "it carries no provider element, which is 1..1".to_owned())?;
    care_of::organisation(value)
}

/// Refuses an `evidence` element of `body` that is no `DV_IDENTIFIER`.
fn evidence<'a, T: Tree<'a>>(body: T) -> Result<(), String> {
    match care_of::named_element(body, RepresentationElement::Evidence.code()) {
        Some(value) if value.get_text("_type").as_deref() != Some("DV_IDENTIFIER") => {
            Err("its evidence is no DV_IDENTIFIER".to_owned())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the canonical party bodies a \
              representation is read from, written as test fixtures"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    const PARENT: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01301";
    const CHILD: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01302";

    fn coded_value(terminology: &str, code: &str) -> Value {
        json!({"_type": "DV_CODED_TEXT", "value": code,
               "defining_code": {"_type": "CODE_PHRASE",
                                 "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                                 "code_string": code}})
    }

    fn element(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1",
               "name": coded_value(TERMINOLOGY, code), "value": value})
    }

    fn person(id: &str) -> Value {
        json!({"id": {"_type": "HIER_OBJECT_ID", "value": id}, "namespace": "demographic",
               "type": "PERSON"})
    }

    /// A representation of `CHILD` by `PARENT` on the basis `GUARD`, for one
    /// provider.
    pub(crate) fn represents() -> Value {
        json!({"_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at1",
               "name": coded_value(TERMINOLOGY, "represents"),
               "source": person(PARENT), "target": person(CHILD),
               "details": {"_type": "ITEM_TREE", "archetype_node_id": "at2", "items": [
                   element("basis", &coded_value("HL7_RoleCode", "GUARD")),
                   element("provider", &json!({"_type": "DV_IDENTIFIER",
                       "issuer": "urn:example:providers", "id": "hospital",
                       "type": "ORGANISATION"}))]}})
    }

    #[test]
    fn a_representation_reads_its_basis_provider_and_ends() {
        let (representative, representation) = read(&represents()).expect("it reads");
        assert_eq!(representative.to_string(), PARENT);
        assert_eq!(representation.subject.to_string(), CHILD);
        assert_eq!(representation.basis.terminology, "HL7_RoleCode");
        assert_eq!(representation.basis.code, "GUARD");
        assert_eq!(
            representation.provider,
            ProviderKey::new("urn:example:providers", "hospital")
        );
    }

    #[test]
    fn a_representation_without_basis_or_provider_is_refused_naming_each() {
        let mut bare = represents();
        bare["details"]["items"] = json!([]);
        let wrong = read(&bare).expect_err("it is refused");
        assert_eq!(wrong.len(), 2, "{wrong:?}");
        assert!(
            wrong[0].contains("basis") && wrong[1].contains("provider"),
            "{wrong:?}"
        );
    }

    #[test]
    fn a_basis_of_the_access_terminology_or_an_unnamed_relationship_is_refused() {
        let mut access = represents();
        access["details"]["items"][0]["value"] = coded_value(TERMINOLOGY, "guardian");
        assert!(read(&access).is_err());
        let mut text = represents();
        text["name"] = json!({"_type": "DV_TEXT", "value": "represents"});
        let wrong = read(&text).expect_err("its name is no code");
        assert!(wrong[0].contains("its name"), "{wrong:?}");
        let mut evidence = represents();
        evidence["details"]["items"]
            .as_array_mut()
            .expect("items")
            .push(element(
                "evidence",
                &json!({"_type": "DV_TEXT", "value": "order"}),
            ));
        assert!(read(&evidence).is_err());
    }
}
