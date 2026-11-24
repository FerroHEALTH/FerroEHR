// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use serde_json::{Value, json};

use super::*;
use crate::party_binding::grants::caller::writer::care_changes;
use crate::party_binding::policy_mark::TERMINOLOGY;
use crate::versioning::Kind;

const TEAM: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";

fn coded(terminology: &str, code: &str) -> Value {
    json!({"_type": "DV_CODED_TEXT", "value": code,
           "defining_code": {"_type": "CODE_PHRASE",
                             "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                             "code_string": code}})
}

fn element(name: &Value, value: &Value) -> Value {
    json!({"_type": "ELEMENT", "archetype_node_id": "at1", "name": name, "value": value})
}

/// A relationship named `code` of `terminology` from and to the
/// container `TEAM`.
fn relationship(terminology: &str, code: &str) -> Value {
    json!({"_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at1",
           "name": coded(terminology, code),
           "source": {"id": {"_type": "HIER_OBJECT_ID", "value": TEAM}, "type": "PERSON"},
           "target": {"id": {"_type": "HIER_OBJECT_ID", "value": TEAM}, "type": "GROUP"}})
}

/// A care relationship of its own to `TEAM` in the role `PART`, in the
/// care of one provider.
fn care_of() -> Value {
    let mut body = relationship(TERMINOLOGY, "under_care_of");
    body["details"] = json!({"_type": "ITEM_TREE", "archetype_node_id": "at2", "items": [
        element(&coded(TERMINOLOGY, "care_role"), &coded("HL7_ParticipationType", "PART")),
        element(&coded(TERMINOLOGY, "provider"),
                &json!({"_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                        "id": "hospital", "type": "ORGANISATION"}))]});
    body
}

#[test]
fn a_care_relationship_is_one_of_its_own_and_never_a_partys() {
    for (kind, rm) in [
        (Kind::Person, "PERSON"),
        (Kind::Organisation, "ORGANISATION"),
    ] {
        let inline = GrantFacts::of(kind, &json!({"_type": rm, "relationships": [care_of()]}));
        assert_eq!(inline.facts, Vec::new(), "{rm}");
        assert_eq!(inline.unread.len(), 1, "{rm}: {:?}", inline.unread);
        assert!(
            inline.unread[0].contains(RESOURCE),
            "{rm}: {:?}",
            inline.unread
        );
    }
    let standalone = GrantFacts::of(Kind::PartyRelationship, &care_of());
    assert_eq!(standalone.unread, Vec::<String>::new());
    assert_eq!(standalone.care_source, Uuid::parse_str(TEAM).ok());
    assert!(matches!(
        standalone.facts.as_slice(),
        [CallerFact {
            kind: FactKind::UnderCareOf,
            value: FactValue::Care(_),
            ..
        }]
    ));
    let mut bare = care_of();
    bare["details"]["items"] = json!([]);
    let refused = GrantFacts::of(Kind::PartyRelationship, &bare);
    assert_eq!(refused.facts, Vec::new());
    assert_eq!(
        refused.unread.len(),
        2,
        "no role, no provider: {:?}",
        refused.unread
    );
    let membership = relationship("HL7_RoleClass", "MBR");
    assert_eq!(
        GrantFacts::of(Kind::PartyRelationship, &membership)
            .unread
            .len(),
        1,
        "a membership is its member's"
    );
    let mut other_role = care_of();
    other_role["details"]["items"][0]["value"] = coded("HL7_ParticipationType", "RESP");
    let other_role = GrantFacts::of(Kind::PartyRelationship, &other_role).facts;
    assert!(
        !care_changes(&standalone.facts, &other_role).is_empty(),
        "a role changed"
    );
}

#[test]
fn a_relationship_never_admits_its_writer_unless_self_declared_and_switched_on() {
    let (writer, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let assigned = |by: Option<Uuid>| {
        CareOf::spanning(
            writer,
            (None, None),
            ForeignCode {
                terminology: "HL7_ParticipationType".to_owned(),
                code: "PART".to_owned(),
            },
            ProviderKey::new("urn:example:providers", "hospital"),
        )
        .written_by(by, false)
    };
    assert!(!assigned(Some(writer)).admits_writer(&[writer], true));
    assert!(assigned(Some(other)).admits_writer(&[writer], false));
    assert!(assigned(None).admits_writer(&[writer], false));
    let declared = assigned(Some(writer)).written_by(Some(writer), true);
    assert!(declared.admits_writer(&[writer], true));
    assert!(!declared.admits_writer(&[writer], false), "the path is off");
}

#[test]
fn a_covers_for_reads_and_one_not_naming_another_carer_is_refused() {
    const COVERED: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c0130a";
    let with_cover = |cover: Value| {
        let mut body = care_of();
        body["details"]["items"]
            .as_array_mut()
            .expect("items")
            .push(element(&coded(TERMINOLOGY, "covers_for"), &cover));
        GrantFacts::of(Kind::PartyRelationship, &body)
    };
    let read = with_cover(json!({"_type": "DV_IDENTIFIER", "issuer": "local",
                                 "id": COVERED, "type": "PERSON"}));
    assert_eq!(read.unread, Vec::<String>::new());
    assert!(matches!(
        read.facts.as_slice(),
        [CallerFact {
            value: FactValue::Care(CareFact {
                covers_for: Some(Cover {
                    key: ProviderKey { issuer, id },
                    kind: CarerKind::Person,
                }),
                ..
            }),
            ..
        }] if issuer == "local" && id == COVERED
    ));
    for refused in [
        json!({"_type": "DV_IDENTIFIER", "id": COVERED, "type": "PERSON"}),
        json!({"_type": "DV_IDENTIFIER", "issuer": "local", "id": COVERED}),
        json!({"_type": "DV_IDENTIFIER", "issuer": "local", "id": COVERED, "type": "AGENT"}),
        json!({"_type": "DV_IDENTIFIER", "issuer": "local", "id": "jansen", "type": "PERSON"}),
        json!({"_type": "DV_IDENTIFIER", "issuer": "local", "id": TEAM, "type": "GROUP"}),
    ] {
        let facts = with_cover(refused.clone());
        assert_eq!(facts.facts, Vec::new(), "{refused}");
        assert_eq!(facts.unread.len(), 1, "{refused}: {:?}", facts.unread);
        assert!(facts.unread[0].contains("covers_for"), "{:?}", facts.unread);
    }
}

#[test]
fn an_episode_reads_and_one_without_an_issuer_and_an_id_is_refused() {
    let with_episode = |episode: Value| {
        let mut body = care_of();
        body["details"]["items"]
            .as_array_mut()
            .expect("items")
            .push(element(&coded(TERMINOLOGY, "episode"), &episode));
        GrantFacts::of(Kind::PartyRelationship, &body)
    };
    let read = with_episode(
        json!({"_type": "DV_IDENTIFIER", "issuer": "H", "id": "C-17",
                                   "type": "VN"}),
    );
    assert_eq!(read.unread, Vec::<String>::new());
    assert!(matches!(
        read.facts.as_slice(),
        [CallerFact {
            value: FactValue::Care(CareFact {
                episode: Some(episode),
                ..
            }),
            ..
        }] if *episode == Episode::new("H", "C-17")
            && episode.key == EpisodeKey::of("H", "C-17")
    ));
    let none = GrantFacts::of(Kind::PartyRelationship, &care_of());
    assert!(matches!(
        none.facts.as_slice(),
        [CallerFact {
            value: FactValue::Care(CareFact { episode: None, .. }),
            ..
        }]
    ));
    for refused in [
        json!({"_type": "DV_IDENTIFIER", "id": "C-17"}),
        json!({"_type": "DV_IDENTIFIER", "issuer": " ", "id": "C-17"}),
        json!({"_type": "DV_IDENTIFIER", "issuer": "H", "id": ""}),
        coded("local", "C-17"),
    ] {
        let facts = with_episode(refused.clone());
        assert_eq!(facts.facts, Vec::new(), "{refused}");
        assert_eq!(facts.unread.len(), 1, "{refused}: {:?}", facts.unread);
        assert!(facts.unread[0].contains("episode"), "{:?}", facts.unread);
    }
}

#[test]
fn a_self_declared_basis_reads() {
    let mut body = care_of();
    let basis = element(
        &coded(TERMINOLOGY, "basis"),
        &coded(TERMINOLOGY, "self_declared"),
    );
    body["details"]["items"]
        .as_array_mut()
        .expect("items")
        .push(basis);
    let facts = GrantFacts::of(Kind::PartyRelationship, &body);
    assert_eq!(facts.unread, Vec::<String>::new());
    assert!(matches!(
        facts.facts.as_slice(),
        [CallerFact {
            value: FactValue::Care(CareFact {
                self_declared: true,
                ..
            }),
            ..
        }]
    ));
}
