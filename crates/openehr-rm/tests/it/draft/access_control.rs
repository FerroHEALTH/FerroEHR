// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test fixtures: a fixture that does not read or write, or does not have the \
              shape it is built with, should fail loudly"
)]
//! The draft access control's generated types (#5506): the worked examples of
//! the ABAC design read and write back through the strict canonical-JSON
//! codec, every class round-trips, the settings travel in the RM's
//! `EHR_ACCESS.settings` carrier, and each class invariant of the BMM refuses
//! an object that breaks it.

use openehr_base::serde_support::OpenSubtype;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_classification::AccessClassification;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::access_control::access_label::AccessLabel;
use openehr_rm::draft::access_control::access_control::access_log_restriction::AccessLogRestriction;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::access_control::versioned_access_policy::VersionedAccessPolicy;
use openehr_rm::draft::access_control::common::archetyped::locatable::Locatable;
use openehr_rm::draft::access_control::common::change_control::versioned_object::VersionedObject;
use openehr_rm::draft::access_control::invariants::{check_policy, check_settings};
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL;
use openehr_rm::draft::invariants::{Violation, Violations};
use openehr_rm::v1_2::ehr::ehr_access::EhrAccess;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A term of the draft access control groups, its code the rubric.
fn term(rubric: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": rubric,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY_ID_ACCESS_CONTROL },
                "code_string": rubric } })
}

/// A term of an external terminology.
fn coded(terminology: &str, value: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A reference to a party of the demographic service.
fn party(kind: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// The LOCATABLE header of a node named `name`.
fn locatable(node_id: &str, name: &str) -> serde_json::Map<String, Value> {
    let header =
        json!({ "name": { "_type": "DV_TEXT", "value": name }, "archetype_node_id": node_id });
    header.as_object().expect("an object").clone()
}

/// A directive of `kind` and `effect` named `name`, with `extra` members.
fn directive(name: &str, kind: &str, effect: &str, extra: Value) -> Value {
    let mut d = locatable("at0001", name);
    d.insert("_type".into(), json!("ACCESS_DIRECTIVE"));
    d.insert("kind".into(), term(kind));
    d.insert("effect".into(), term(effect));
    d.insert("emergency_override".into(), json!(false));
    let Value::Object(extra) = extra else {
        panic!("the extra members are an object")
    };
    d.extend(extra);
    Value::Object(d)
}

const MS_A: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cca427";
const DR_B: &str = "6f1c2d3e-4b5a-4c6d-8e7f-90a1b2c3d4e5";
const DR_C: &str = "8a2d3e4f-5b6c-4d7e-9f80-a1b2c3d4e5f6";
const HOSPITAL: &str = "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7";
const REGION: &str = "3d4e5f6a-7b8c-4d9e-8f0a-b1c2d3e4f5a6";

/// The ex-partner: a consent deny on Dr B's `PERSON`, no data scope, lifted by
/// a declared emergency.
fn ex_partner() -> Value {
    directive(
        "Not Dr B",
        "consent",
        "deny",
        json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", DR_B) }],
            "emergency_override": true,
            "instructed_by": { "_type": "PARTY_SELF" },
            "instructed": { "_type": "DV_DATE_TIME", "value": "2026-11-02T10:00:00Z" }
        }),
    )
}

/// Mental health, except the psychiatrist: a restriction on a category with
/// one permit exception for Dr C.
fn mental_health() -> Value {
    directive(
        "Mental health restriction",
        "restriction",
        "deny",
        json!({
            "data": [{ "_type": "ACCESS_DATA_SCOPE",
                "categories": [coded("local", "mental health", "MH")] }],
            "exceptions": [directive("Except my psychiatrist", "consent", "permit", json!({
                "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", DR_C) }]
            }))],
            "legal_grounds": [coded("local", "EHDS Art. 8", "ehds-8")],
            "time_validity": { "_type": "DV_INTERVAL",
                "lower": { "_type": "DV_DATE_TIME", "value": "2026-11-01T00:00:00Z" },
                "lower_unbounded": false, "upper_unbounded": true,
                "lower_included": true, "upper_included": false }
        }),
    )
}

/// Regional sharing: the catalogue entry identified by the region's BPPC
/// policy, applying to whoever acts for an organisation of the regional group.
fn regional_sharing() -> Value {
    directive(
        "Regional sharing consent",
        "consent",
        "permit",
        json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("GROUP", REGION) }],
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "regional health network",
                "id": "2.16.840.1.113883.3.7.1", "type": "BPPC policy" }]
        }),
    )
}

/// Ms A's settings: consent required, herself as gate-keeper, the three
/// directives of the worked examples (her regional consent copied from the
/// catalogue with the signed form as evidence) and one sensitivity label.
fn settings() -> Value {
    let mut signed = regional_sharing();
    signed["evidence"] = json!([{ "_type": "DV_EHR_URI",
        "value": format!("ehr://{MS_A}/8849182c-82ad-4088-a07f-48ead4180515::cdr.example.org::1") }]);
    json!({
        "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": term("consent_required"),
        "gate_keepers": [{ "_type": "PARTY_SELF" }],
        "directives": [ex_partner(), mental_health(), signed],
        "labels": [{ "_type": "ACCESS_LABEL",
            "target": { "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" } },
            "sensitivities": [coded("http://terminology.hl7.org/CodeSystem/v3-ActCode", "HIV", "HIV")],
            "reason": { "_type": "DV_TEXT", "value": "requested by the subject" } }]
    })
}

/// A term of the access log's openEHR `accessor kind` group, its code the
/// rubric.
fn accessor_kind(rubric: &str) -> Value {
    coded("openehr", rubric, rubric)
}

/// A restriction of the subject view under a national provision: the name of
/// a person of another organisation withheld.
fn log_restriction() -> Value {
    let mut r = locatable("at0003", "External accessor withheld");
    r.extend(
        json!({
            "_type": "ACCESS_LOG_RESTRICTION",
            "accessor_kinds": [accessor_kind("external person")],
            "withheld": [term("accessor_party")],
            "legal_grounds": [coded("urn:example:national-law", "Art. 15e(3)", "15e-3")]
        })
        .as_object()
        .expect("an object")
        .clone(),
    );
    Value::Object(r)
}

/// The hospital's policy: the ward's care relationship with a 14-day grace,
/// the HIV classification of `lab_hiv.v1`, the regional catalogue entry, and
/// one restriction of the subject's view of the access log.
fn policy() -> Value {
    let mut p = locatable(
        "openEHR-EHR-ACCESS_POLICY.hospital.v1",
        "Hospital access policy",
    );
    p.extend(
        json!({
            "_type": "ACCESS_POLICY",
            "archetype_details": { "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-ACCESS_POLICY.hospital.v1" },
                "rm_version": "1.2.0" },
            "controller": party("ORGANISATION", HOSPITAL),
            "classifications": [{ "_type": "ACCESS_CLASSIFICATION",
                "name": { "_type": "DV_TEXT", "value": "HIV results" }, "archetype_node_id": "at0002",
                "data": { "_type": "ACCESS_DATA_SCOPE", "templates": ["lab_hiv.v1"] },
                "sensitivities": [coded("http://terminology.hl7.org/CodeSystem/v3-ActCode", "HIV", "HIV")] }],
            "care_relationship_required": true,
            "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
            "purpose_required": true,
            "catalogue": [regional_sharing()],
            "log_restrictions": [log_restriction()]
        })
        .as_object()
        .expect("an object")
        .clone(),
    );
    Value::Object(p)
}

/// Reads `value` as a `T` through the strict canonical-JSON codec.
fn read<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("the object reads")
}

/// Reads `value` as a `T`, writes it, and asserts the written form is the
/// input and reads back as the same object.
fn round_trip<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(value: &Value) -> T {
    let object: T = read(value.clone());
    let written = serde_json::to_value(&object).expect("the object writes");
    assert_eq!(&written, value, "the written form is the canonical input");
    assert_eq!(read::<T>(written), object, "the written form reads back");
    object
}

/// The invariants broken, as `CLASS.Invariant`.
fn named(result: Result<(), Violations>) -> Vec<String> {
    result.err().map_or_else(Vec::new, |violations| {
        violations.0.iter().map(Violation::to_string).collect()
    })
}

#[test]
fn the_worked_examples_round_trip_and_hold_every_invariant() {
    let settings: AccessDirectiveSettings = round_trip(&settings());
    assert_eq!(check_settings(&settings), Ok(()));
    let policy: AccessPolicy = round_trip(&policy());
    assert_eq!(check_policy(&policy), Ok(()));
}

#[test]
fn every_class_round_trips() {
    let s = settings();
    let p = policy();
    round_trip::<AccessDirectiveSettings>(&s);
    round_trip::<AccessDirective>(&s["directives"][1]);
    round_trip::<AccessActorScope>(&s["directives"][0]["applies_to"][0]);
    round_trip::<AccessDataScope>(&s["directives"][1]["data"][0]);
    round_trip::<AccessLabel>(&s["labels"][0]);
    round_trip::<AccessPolicy>(&p);
    round_trip::<AccessClassification>(&p["classifications"][0]);
    round_trip::<AccessLogRestriction>(&p["log_restrictions"][0]);
    let versioned = json!({
        "_type": "VERSIONED_ACCESS_POLICY",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "5e6f7a8b-9c0d-4e1f-8a2b-c3d4e5f6a7b8" },
        "owner_id": { "_type": "OBJECT_REF", "namespace": "local", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": HOSPITAL } },
        "time_created": { "_type": "DV_DATE_TIME", "value": "2026-11-13T09:00:00Z" }
    });
    round_trip::<VersionedAccessPolicy>(&versioned);
    // The draft classes join the re-emitted RM polymorphic slots.
    assert!(matches!(
        round_trip::<VersionedObject>(&versioned),
        VersionedObject::VersionedAccessPolicy(_)
    ));
    assert!(matches!(
        round_trip::<Locatable>(&p),
        Locatable::AccessPolicy(_)
    ));
    assert!(matches!(
        round_trip::<Locatable>(&s["directives"][0]),
        Locatable::AccessDirective(_)
    ));
}

#[test]
fn the_settings_travel_in_the_ehr_access_carrier() {
    let ehr_access = json!({
        "_type": "EHR_ACCESS",
        "name": { "_type": "DV_TEXT", "value": "EHR access" },
        "archetype_node_id": "openEHR-EHR-EHR_ACCESS.access_directives.v1",
        "settings": settings()
    });
    let ehr_access: EhrAccess = read(ehr_access);
    let carrier = ehr_access
        .settings
        .as_ref()
        .expect("the settings are carried");
    assert_eq!(carrier.type_name(), "ACCESS_DIRECTIVE_SETTINGS");
    let typed = AccessDirectiveSettings::try_from(carrier).expect("the carrier reads as settings");
    assert_eq!(typed, read(settings()));
    let back = OpenSubtype::try_from(&typed).expect("the settings write into the carrier");
    assert_eq!(&back, carrier);
    let other = OpenSubtype::new(
        "OTHER_ACCESS_CONTROL_SETTINGS",
        settings()
            .as_object()
            .expect("an object")
            .clone()
            .into_iter()
            .filter(|(k, _)| k != "_type")
            .collect(),
    )
    .expect("a carrier of another scheme");
    assert!(
        AccessDirectiveSettings::try_from(&other).is_err(),
        "a carrier of another scheme is refused"
    );
}

/// An invariant, and the edit of the example that breaks it alone.
type Breaking = (&'static str, fn(&mut Value));

#[test]
fn each_settings_invariant_refuses_settings_that_break_it() {
    let cases: [Breaking; 11] = [
        ("ACCESS_DIRECTIVE_SETTINGS.Default_access_valid", |s| {
            s["default_access"] = term("closed");
        }),
        ("ACCESS_DIRECTIVE_SETTINGS.Opt_out_unique", |s| {
            let opt_out = directive("Opt-out", "opt_out", "deny", json!({}));
            s["directives"] = json!([opt_out.clone(), opt_out]);
        }),
        ("ACCESS_DIRECTIVE.Kind_valid", |s| {
            s["directives"][0]["kind"] = coded("local", "consent", "consent");
        }),
        ("ACCESS_DIRECTIVE.Effect_valid", |s| {
            s["directives"][0]["effect"] = term("maybe");
        }),
        ("ACCESS_DIRECTIVE.Restriction_denies", |s| {
            s["directives"][1]["effect"] = term("permit");
            s["directives"][1]["exceptions"][0]["effect"] = term("deny");
        }),
        ("ACCESS_DIRECTIVE.Opt_out_denies_whole_record", |s| {
            s["directives"][1]["kind"] = term("opt_out");
        }),
        ("ACCESS_DIRECTIVE.Exceptions_opposite", |s| {
            s["directives"][1]["exceptions"][0]["effect"] = term("deny");
        }),
        ("ACCESS_DIRECTIVE.Exceptions_opposite", |s| {
            let inner = directive("Nested", "consent", "deny", json!({}));
            s["directives"][1]["exceptions"][0]["exceptions"] = json!([inner]);
        }),
        ("ACCESS_ACTOR_SCOPE.Not_empty", |s| {
            s["directives"][0]["applies_to"][0] = json!({ "_type": "ACCESS_ACTOR_SCOPE" });
        }),
        ("ACCESS_ACTOR_SCOPE.Relationship_valid", |s| {
            s["directives"][0]["applies_to"][0]["relationship"] = term("neighbour");
        }),
        ("ACCESS_ACTOR_SCOPE.Caller_kind_valid", |s| {
            s["directives"][0]["applies_to"][0]["caller_kind"] = term("robot");
        }),
    ];
    for (invariant, breaking) in cases {
        let mut s = settings();
        breaking(&mut s);
        assert_eq!(
            named(check_settings(&read(s))),
            vec![invariant.to_owned()],
            "{invariant}"
        );
    }
    let mut empty_scope = settings();
    empty_scope["directives"][1]["data"][0] = json!({ "_type": "ACCESS_DATA_SCOPE" });
    assert_eq!(
        named(check_settings(&read(empty_scope))),
        vec!["ACCESS_DATA_SCOPE.Not_empty".to_owned()]
    );
}

#[test]
fn each_policy_invariant_refuses_a_policy_that_breaks_it() {
    let cases: [Breaking; 7] = [
        ("ACCESS_POLICY.Is_archetype_root", |p| {
            p.as_object_mut()
                .expect("an object")
                .remove("archetype_details");
        }),
        ("ACCESS_POLICY.Grace_valid", |p| {
            p["relationship_grace"]["value"] = json!("-P14D");
        }),
        ("ACCESS_POLICY.Catalogue_identified", |p| {
            p["catalogue"][0]
                .as_object_mut()
                .expect("an object")
                .remove("identifiers");
        }),
        ("ACCESS_CLASSIFICATION.No_objects", |p| {
            p["classifications"][0]["data"]["objects"] = json!([{ "_type": "OBJECT_REF",
                "namespace": "local", "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" } }]);
        }),
        ("ACCESS_DATA_SCOPE.Not_empty", |p| {
            p["classifications"][0]["data"] = json!({ "_type": "ACCESS_DATA_SCOPE" });
        }),
        ("ACCESS_LOG_RESTRICTION.Accessor_kinds_valid", |p| {
            p["log_restrictions"][0]["accessor_kinds"] = json!([accessor_kind("visitor")]);
        }),
        ("ACCESS_LOG_RESTRICTION.Withheld_valid", |p| {
            p["log_restrictions"][0]["withheld"] = json!([term("accessor_credential")]);
        }),
    ];
    for (invariant, breaking) in cases {
        let mut p = policy();
        breaking(&mut p);
        assert_eq!(
            named(check_policy(&read(p))),
            vec![invariant.to_owned()],
            "{invariant}"
        );
    }
}

#[test]
fn each_optional_list_is_absent_or_non_empty() {
    let settings_lists: [&[&str]; 13] = [
        &["gate_keepers"],
        &["directives"],
        &["labels"],
        &["directives", "0", "applies_to"],
        &["directives", "0", "operations"],
        &["directives", "0", "purposes"],
        &["directives", "0", "data"],
        &["directives", "0", "exceptions"],
        &["directives", "0", "legal_grounds"],
        &["directives", "0", "evidence"],
        &["directives", "0", "identifiers"],
        &["directives", "1", "data", "0", "objects"],
        &["directives", "1", "data", "0", "templates"],
    ];
    let scope_lists: [&str; 4] = ["archetypes", "categories", "sensitivities", "origins"];
    let mut refused = Vec::new();
    for path in settings_lists {
        let mut s = settings();
        *path
            .iter()
            .fold(&mut s, |v, key| match key.parse::<usize>() {
                Ok(i) => &mut v[i],
                Err(_) => &mut v[*key],
            }) = json!([]);
        refused.push(serde_json::from_value::<AccessDirectiveSettings>(s).is_err());
    }
    for list in scope_lists {
        let mut s = settings();
        s["directives"][1]["data"][0][list] = json!([]);
        refused.push(serde_json::from_value::<AccessDirectiveSettings>(s).is_err());
    }
    for list in ["classifications", "catalogue", "log_restrictions"] {
        let mut p = policy();
        p[list] = json!([]);
        refused.push(serde_json::from_value::<AccessPolicy>(p).is_err());
    }
    assert_eq!(
        refused,
        vec![true; 20],
        "every empty optional list is refused"
    );
}
