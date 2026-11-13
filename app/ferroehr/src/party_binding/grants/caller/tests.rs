// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use serde_json::json;

use super::*;

fn coded(terminology: &str, code: &str) -> Value {
    json!({"_type": "DV_CODED_TEXT", "value": code,
           "defining_code": {"_type": "CODE_PHRASE",
                             "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                             "code_string": code}})
}

fn element(name: &Value, value: &Value) -> Value {
    json!({"_type": "ELEMENT", "archetype_node_id": "at1", "name": name, "value": value})
}

fn code(terminology: &str, code: &str) -> ForeignCode {
    ForeignCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    }
}

const TEAM: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";

fn relationship(code: &str, target: &str) -> Value {
    json!({"_type": "PARTY_RELATIONSHIP",
           "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward.v4",
           "name": {"_type": "DV_TEXT", "value": "ward"},
           "details": {"_type": "ITEM_TREE", "archetype_node_id": "at2",
                       "items": [element(&json!({"_type": "DV_TEXT", "value": "kind"}),
                                         &coded(TERMINOLOGY, code))]},
           "time_validity": {"_type": "DV_INTERVAL",
                             "lower": {"_type": "DV_DATE", "value": "2026-11-03"},
                             "lower_unbounded": false, "upper_unbounded": true},
           "target": {"id": {"_type": "HIER_OBJECT_ID", "value": target}}})
}

#[test]
fn a_capability_clears_and_permits_by_coded_element_anywhere_in_its_credentials() {
    let body = json!({
        "_type": "ROLE", "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-ROLE.psychiatrist.v2",
        "capabilities": [
            {"_type": "CAPABILITY", "credentials": {"items": [
                {"items": [element(&coded(TERMINOLOGY, "sensitivity_clearance"),
                                   &coded("2.16.840.1.113883.5.25", "PSY"))]},
                element(&coded(TERMINOLOGY, "permitted_purpose"),
                        &coded("2.16.840.1.113883.5.8", "TREAT")),
                element(&json!({"_type": "DV_TEXT", "value": "sensitivity_clearance"}),
                        &coded("2.16.840.1.113883.5.25", "HIV"))
            ]}},
            {"_type": "CAPABILITY", "credentials": {"items": [
                element(&coded(TERMINOLOGY, "sensitivity_clearance"),
                        &coded(TERMINOLOGY, "open")),
                element(&coded(TERMINOLOGY, "permitted_purpose"),
                        &json!({"_type": "DV_TEXT", "value": "TREAT"}))
            ]}}
        ]
    });
    let facts = GrantFacts::of(Kind::Role, &body);
    let read: Vec<(i32, FactKind, FactValue)> = facts
        .facts
        .iter()
        .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
        .collect();
    assert_eq!(
        read,
        vec![
            (
                0,
                FactKind::Clearance,
                FactValue::Code(code("2.16.840.1.113883.5.25", "PSY"))
            ),
            (
                0,
                FactKind::Purpose,
                FactValue::Code(code("2.16.840.1.113883.5.8", "TREAT"))
            ),
        ],
        "an element whose name is no code of the terminology clears nothing"
    );
    assert_eq!(facts.unread.len(), 2, "{:?}", facts.unread);
    assert!(facts.unread[0].contains("sensitivity_clearance"));
    assert!(facts.unread[1].contains("permitted_purpose"));
    assert!(
        facts.capabilities.is_empty(),
        "a clearance grants no operation"
    );
}

#[test]
fn a_persons_care_relationships_are_read_by_their_code_in_any_archetype() {
    let body = json!({"_type": "PERSON", "relationships": [
        relationship("member_of", TEAM),
        relationship("represents", TEAM),
        relationship("under_care_of", TEAM),
        relationship("under_care_of", "1.2.840.113619")
    ]});
    let facts = GrantFacts::of(Kind::Person, &body);
    let team = Uuid::parse_str(TEAM).expect("a uuid");
    let read: Vec<(i32, FactKind, FactValue)> = facts
        .facts
        .iter()
        .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
        .collect();
    assert_eq!(
        read,
        vec![
            (0, FactKind::MemberOf, FactValue::Team(team)),
            (2, FactKind::UnderCareOf, FactValue::Team(team)),
        ]
    );
    assert!(facts.facts[0].validity.is_some());
    assert_eq!(facts.unread.len(), 1, "{:?}", facts.unread);
    assert!(facts.unread[0].starts_with("relationships[3]"));
}

#[test]
fn a_care_relationship_is_a_persons_and_never_one_of_its_own() {
    let body = json!({"_type": "GROUP", "relationships": [relationship("member_of", TEAM)]});
    let facts = GrantFacts::of(Kind::Group, &body);
    assert_eq!(facts.facts, Vec::new());
    assert_eq!(facts.unread.len(), 1);
    let standalone = GrantFacts::of(
        Kind::PartyRelationship,
        &relationship("under_care_of", TEAM),
    );
    assert_eq!(standalone.unread.len(), 1);
    assert_eq!(
        GrantFacts::of(Kind::PartyRelationship, &relationship("represents", TEAM)).unread,
        Vec::<String>::new()
    );
}

#[test]
fn every_kind_reads_back_from_its_code() {
    for kind in FactKind::ALL {
        assert_eq!(FactKind::of(kind.code()), Some(kind));
    }
    assert_eq!(FactKind::of("represents"), None);
}

#[test]
fn caller_facts_answer_by_terminology_and_code() {
    let facts = CallerFacts::new(
        vec![code("ActCode", "PSY"), code("ActCode", "PSY")],
        vec![code("PurposeOfUse", "TREAT")],
        Vec::new(),
    );
    assert_eq!(facts.clearances().len(), 1);
    assert!(facts.clears(&code("ActCode", "PSY")));
    assert!(!facts.clears(&code("local", "PSY")));
    assert!(facts.permits(&code("PurposeOfUse", "TREAT")));
    assert!(!facts.permits(&code("PurposeOfUse", "HOPERAT")));
}

#[test]
fn a_care_team_holds_within_its_dates() {
    let day = |text: &str| text.parse::<jiff::civil::Date>().expect("a date");
    let team = CareTeam::new(
        Uuid::nil(),
        Dates {
            lower: Some(day("2026-11-03")),
            upper: Some(day("2026-11-11")),
        },
    );
    assert!(team.holds_on(day("2026-11-03")) && team.holds_on(day("2026-11-10")));
    assert!(!team.holds_on(day("2026-11-11")) && !team.holds_on(day("2026-11-02")));
    assert_eq!(team.until(), Some(day("2026-11-11")));
    assert_eq!(team.from(), Some(day("2026-11-03")));
}
