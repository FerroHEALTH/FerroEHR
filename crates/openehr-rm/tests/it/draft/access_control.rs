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
use openehr_rm::draft::access_control::access_control::access_log_retention::AccessLogRetention;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::access_control::access_retention_bound::AccessRetentionBound;
use openehr_rm::draft::access_control::access_control::versioned_access_policy::VersionedAccessPolicy;
use openehr_rm::draft::access_control::common::archetyped::locatable::Locatable;
use openehr_rm::draft::access_control::common::change_control::versioned_object::VersionedObject;
use openehr_rm::draft::access_control::invariants::{check_policy, check_settings};
use openehr_rm::draft::access_control::terminology::{
    TERMINOLOGY_ID_ACCESS, TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
};
use openehr_rm::draft::invariants::{Violation, Violations};
use openehr_rm::v1_2::ehr::ehr_access::EhrAccess;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A term of the draft `openehr_access` terminology, its code the rubric.
/// An `ACCESS_RELATIONSHIP_VALIDITY` with the periods given.
fn validity(default: Option<&str>, max: Option<&str>) -> Value {
    let mut entry = json!({ "_type": "ACCESS_RELATIONSHIP_VALIDITY" });
    for (name, period) in [("default_period", default), ("max_period", max)] {
        if let Some(period) = period {
            entry[name] = json!({ "_type": "DV_DURATION", "value": period });
        }
    }
    entry
}

fn term(rubric: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": rubric,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY_ID_ACCESS },
                "code_string": rubric } })
}

/// An `access effect`: a code of HL7 FHIR `HL7_ConsentProvisionType`.
fn effect(code: &str) -> Value {
    coded(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, code, code)
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
    d.insert("effect".into(), self::effect(effect));
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

/// The nurse rule: the controller denies the nurse role the commit of the
/// diagnosis template, under a national provision.
fn nurse_rule() -> Value {
    directive(
        "Nurses do not record diagnoses",
        "controller_rule",
        "deny",
        json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "role": coded("urn:example:roles", "nurse", "NURSE") }],
            "operations": [term("write")],
            "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["diagnosis.v1"] }],
            "legal_grounds": [coded("urn:example:national-law", "Art. 4", "4")]
        }),
    )
}

/// The psychiatry rule: the controller clears the psychiatrist role for
/// psychiatric data, read for treatment.
fn psychiatry_rule() -> Value {
    directive(
        "Psychiatry reads psychiatric notes",
        "controller_rule",
        "permit",
        json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "role": coded("urn:example:roles", "psychiatrist", "PSYCHIATRIST") }],
            "operations": [term("read")],
            "purposes": [coded("HL7_ActReason", "treatment", "TREAT")],
            "data": [{ "_type": "ACCESS_DATA_SCOPE",
                "sensitivities": [coded("HL7_ActCode", "PSY", "PSY")] }]
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
            "sensitivities": [coded("HL7_ActCode", "HIV", "HIV")],
            "reason": { "_type": "DV_TEXT", "value": "requested by the subject" } }]
    })
}

/// A term of the draft `accessor kind` group, its code the rubric.
fn accessor_kind(rubric: &str) -> Value {
    term(rubric)
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

/// A floor or ceiling of `period` under the synthetic provision `code`.
fn bound(period: &str, code: &str) -> Value {
    json!({ "_type": "ACCESS_RETENTION_BOUND",
        "period": { "_type": "DV_DURATION", "value": period },
        "ground": coded("http://data.europa.eu/eli/example/2026/1/oj", code, code) })
}

/// The access log's retention of a controller in the Union: five years,
/// within a national floor of four years and a ceiling of ten.
fn log_retention() -> Value {
    let mut r = locatable("at0004", "Access log retention");
    r.extend(
        json!({
            "_type": "ACCESS_LOG_RETENTION",
            "in_union": true,
            "period": { "_type": "DV_DURATION", "value": "P5Y" },
            "floors": [bound("P4Y", "art_4")],
            "ceilings": [bound("P10Y", "art_10")]
        })
        .as_object()
        .expect("an object")
        .clone(),
    );
    Value::Object(r)
}

/// The hospital's policy: the ward's care relationship with a 14-day grace,
/// the HIV classification of `lab_hiv.v1`, the nurse and psychiatry rules,
/// the regional catalogue entry, one restriction of the subject's view of
/// the access log, and the log's retention.
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
                "sensitivities": [coded("HL7_ActCode", "HIV", "HIV")] }],
            "care_relationship_required": true,
            "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
            "purpose_required": true,
            "rules": [nurse_rule(), psychiatry_rule()],
            "catalogue": [regional_sharing()],
            "self_declared_care": { "_type": "DV_DURATION", "value": "P1D" },
            "attested_care": [{ "_type": "DV_IDENTIFIER", "issuer": "Regional exchange",
                "id": "urn:example:agreement:attested-care" }],
            "caseload": true,
            "conflict_rules": [{ "_type": "ACCESS_CONFLICT_RULE",
                "conflict": term("related"),
                "kinships": [coded("openehr", "mother", "10")] }],
            "concealed_confidentialities": [coded("HL7_Confidentiality", "very restricted", "V")],
            "value_sets": [{ "_type": "ACCESS_VALUE_SET", "binding": term("kinship"),
                "value_set": { "_type": "DV_URI", "value": "http://terminology.example/ValueSet/kinship" } }],
            "log_restrictions": [log_restriction()],
            "log_retention": log_retention()
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
    round_trip::<AccessDirective>(&p["rules"][0]);
    round_trip::<AccessLogRestriction>(&p["log_restrictions"][0]);
    round_trip::<AccessLogRetention>(&p["log_retention"]);
    round_trip::<AccessRetentionBound>(&p["log_retention"]["floors"][0]);
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
    let cases: [Breaking; 13] = [
        (
            "ACCESS_DIRECTIVE_SETTINGS.Directives_not_controller_rules",
            |s| {
                s["directives"][0]["kind"] = term("controller_rule");
            },
        ),
        (
            "ACCESS_DIRECTIVE_SETTINGS.Directives_not_controller_rules",
            |s| {
                s["directives"][1]["exceptions"][0]["kind"] = term("controller_rule");
            },
        ),
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
            s["directives"][0]["effect"] = effect("maybe");
        }),
        ("ACCESS_DIRECTIVE.Restriction_denies", |s| {
            s["directives"][1]["effect"] = effect("permit");
            s["directives"][1]["exceptions"][0]["effect"] = effect("deny");
        }),
        ("ACCESS_DIRECTIVE.Opt_out_denies_whole_record", |s| {
            s["directives"][1]["kind"] = term("opt_out");
        }),
        ("ACCESS_DIRECTIVE.Exceptions_opposite", |s| {
            s["directives"][1]["exceptions"][0]["effect"] = effect("deny");
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

/// The controller's exclusion of a named party: a deny of kind `exclusion`
/// with no exception, naming Dr B.
fn exclusion() -> Value {
    directive(
        "Not Dr B on this record",
        "exclusion",
        "deny",
        json!({
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party("PERSON", DR_B) }]
        }),
    )
}

/// An exclusion sits only in `exclusions`, always denies with no exception,
/// and nothing else sits there or in `directives` as one.
#[test]
fn each_exclusion_invariant_refuses_settings_that_break_it() {
    let mut held = settings();
    held["exclusions"] = json!([exclusion()]);
    assert_eq!(named(check_settings(&read(held))), Vec::<String>::new());
    let cases: [Breaking; 3] = [
        ("CONFIDENTIALITY_SETTINGS.Exclusions_kind", |s| {
            s["exclusions"] = json!([ex_partner()]);
        }),
        ("ACCESS_DIRECTIVE.Exclusion_denies", |s| {
            let mut permit = exclusion();
            permit["effect"] = effect("permit");
            s["exclusions"] = json!([permit]);
        }),
        (
            "ACCESS_DIRECTIVE_SETTINGS.Directives_not_controller_rules",
            |s| {
                s["directives"][0]["kind"] = term("exclusion");
            },
        ),
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
    let mut nested = settings();
    nested["directives"][1]["exceptions"][0]["kind"] = term("exclusion");
    assert!(
        named(check_settings(&read(nested)))
            .contains(&"ACCESS_DIRECTIVE_SETTINGS.Directives_not_controller_rules".to_owned())
    );
    let mut excepted = settings();
    let mut with_exception = exclusion();
    with_exception["exceptions"] = json!([directive("Unless", "consent", "permit", json!({}))]);
    excepted["exclusions"] = json!([with_exception]);
    assert!(
        named(check_settings(&read(excepted)))
            .contains(&"ACCESS_DIRECTIVE.Exclusion_denies".to_owned())
    );
}

#[test]
fn each_policy_invariant_refuses_a_policy_that_breaks_it() {
    let cases: [Breaking; 22] = [
        ("ACCESS_POLICY.Self_declared_valid", |p| {
            p["self_declared_care"]["value"] = json!("PT0S");
        }),
        ("ACCESS_POLICY.Concealed_valid", |p| {
            p["concealed_confidentialities"] =
                json!([coded("HL7_ActCode", "very restricted", "V")]);
        }),
        ("ACCESS_CONFLICT_RULE.Conflict_valid", |p| {
            p["conflict_rules"][0]["conflict"] = term("kinship");
            // NOTE: draft BMM ACCESS_CONFLICT_RULE.Kinships_valid, kinships need `related`.
            p["conflict_rules"][0]
                .as_object_mut()
                .expect("an object")
                .remove("kinships");
        }),
        ("ACCESS_CONFLICT_RULE.Kinships_valid", |p| {
            p["conflict_rules"][0]["conflict"] = term("self");
        }),
        ("ACCESS_VALUE_SET.Binding_valid", |p| {
            p["value_sets"][0]["binding"] = term("related");
        }),
        ("ACCESS_POLICY.Rules_kind", |p| {
            p["rules"][0]["kind"] = term("consent");
        }),
        ("ACCESS_POLICY.Rules_kind", |p| {
            let exception = directive("Except the ward", "consent", "permit", json!({}));
            p["rules"][0]["exceptions"] = json!([exception]);
        }),
        ("ACCESS_POLICY.Catalogue_not_controller_rules", |p| {
            p["catalogue"][0]["kind"] = term("controller_rule");
        }),
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
        ("ACCESS_RETENTION_BOUND.Period_valid", |p| {
            p["log_retention"]["floors"] = json!([bound("P0D", "art_0")]);
        }),
        ("ACCESS_LOG_RETENTION.Floor_outside_union", |p| {
            p["log_retention"]["in_union"] = json!(false);
            p["log_retention"]
                .as_object_mut()
                .expect("an object")
                .remove("floors");
        }),
        ("ACCESS_LOG_RETENTION.Period_union", |p| {
            p["log_retention"]["period"]["value"] = json!("P1095D");
            p["log_retention"]["floors"] = json!([bound("P1Y", "art_1")]);
        }),
        ("ACCESS_LOG_RETENTION.Period_floors", |p| {
            p["log_retention"]["floors"] = json!([bound("P6Y", "art_6")]);
        }),
        ("ACCESS_LOG_RETENTION.Ceilings_need_period", |p| {
            p["log_retention"]
                .as_object_mut()
                .expect("an object")
                .remove("period");
        }),
        ("ACCESS_LOG_RETENTION.Period_ceilings", |p| {
            p["log_retention"]["period"]["value"] = json!("P11Y");
            p["log_retention"]["ceilings"] = json!([bound("P10Y", "art_10")]);
        }),
        ("ACCESS_LOG_RETENTION.Period_ceilings", |p| {
            p["log_retention"]["period"]["value"] = json!("P3653D");
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

/// Each invariant of `ACCESS_RELATIONSHIP_VALIDITY`, `ACCESS_RELATIONSHIP_WRITER`
/// and the policy's `team_targets` refuses a policy that breaks it (#6073).
#[test]
fn each_relationship_invariant_refuses_a_policy_that_breaks_it() {
    let cases: [Breaking; 6] = [
        ("ACCESS_RELATIONSHIP_VALIDITY.Default_valid", |p| {
            p["relationship_validity"] = json!([validity(Some("P0D"), Some("P1D"))]);
        }),
        ("ACCESS_RELATIONSHIP_VALIDITY.Max_valid", |p| {
            p["relationship_validity"] = json!([validity(None, Some("P0D"))]);
        }),
        ("ACCESS_RELATIONSHIP_VALIDITY.Default_within_max", |p| {
            p["relationship_validity"] = json!([validity(Some("P31D"), Some("P30D"))]);
        }),
        ("ACCESS_RELATIONSHIP_VALIDITY.Bounded_together", |p| {
            p["relationship_validity"] = json!([validity(Some("P30D"), None)]);
        }),
        ("ACCESS_RELATIONSHIP_WRITER.Confidentialities_valid", |p| {
            p["relationship_writers"] = json!([{ "_type": "ACCESS_RELATIONSHIP_WRITER",
                "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                    "caller_kind": term("system") }],
                "confidentialities": [coded("local", "R", "restricted")] }]);
        }),
        ("ACCESS_POLICY.Team_targets_valid", |p| {
            p["team_targets"] = json!([{ "_type": "PARTY_REF", "namespace": "demographic",
                "type": "GROUP",
                "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180516" } }]);
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

/// A ceiling below a floor leaves no period between them: the period breaks
/// one of the two as well, and a period kept without end breaks every
/// ceiling (`Ceilings_need_period`).
#[test]
fn a_ceiling_below_a_floor_and_a_ceiling_without_a_period_are_refused() {
    let mut p = policy();
    p["log_retention"]["in_union"] = json!(false);
    p["log_retention"]["period"]["value"] = json!("P2Y");
    p["log_retention"]["floors"] = json!([bound("P2Y", "art_2")]);
    p["log_retention"]["ceilings"] = json!([bound("P18M", "art_18")]);
    assert_eq!(
        named(check_policy(&read(p.clone()))),
        vec![
            "ACCESS_LOG_RETENTION.Period_ceilings".to_owned(),
            "ACCESS_LOG_RETENTION.Ceilings_floors".to_owned()
        ]
    );
    p["log_retention"]
        .as_object_mut()
        .expect("an object")
        .remove("period");
    assert_eq!(
        named(check_policy(&read(p))),
        vec![
            "ACCESS_LOG_RETENTION.Ceilings_need_period".to_owned(),
            "ACCESS_LOG_RETENTION.Ceilings_floors".to_owned()
        ]
    );
}

/// A controller outside the Union keeps the log as long as its own law asks,
/// under three years too, and a period without end meets every floor.
#[test]
fn outside_the_union_a_period_under_three_years_holds() {
    let mut p = policy();
    p["log_retention"] = {
        let mut r = locatable("at0004", "Access log retention");
        r.extend(
            json!({ "_type": "ACCESS_LOG_RETENTION", "in_union": false,
                "period": { "_type": "DV_DURATION", "value": "P1Y" },
                "floors": [bound("P1Y", "art_4_5")] })
            .as_object()
            .expect("an object")
            .clone(),
        );
        Value::Object(r)
    };
    assert_eq!(named(check_policy(&read(p.clone()))), Vec::<String>::new());
    p["log_retention"]
        .as_object_mut()
        .expect("an object")
        .remove("period");
    assert_eq!(named(check_policy(&read(p))), Vec::<String>::new());
}

/// Each `DV_DATE_TIME` and `DV_DURATION` the settings and the policy hold
/// keeps its RM `Value_valid`: a value that is no ISO 8601 date-time or
/// duration is refused (RM Data Types, `DV_DATE_TIME` and `DV_DURATION`
/// §Invariants).
#[test]
fn each_date_time_and_duration_is_iso_8601() {
    let cases: [Breaking; 4] = [
        ("DV_DATE_TIME.Value_valid", |s| {
            s["directives"][1]["time_validity"]["lower"]["value"] = json!("T00:00:00Z");
        }),
        ("DV_DATE_TIME.Value_valid", |s| {
            s["directives"][0]["instructed"]["value"] = json!("yesterday");
        }),
        ("DV_DATE_TIME.Value_valid", |s| {
            s["directives"][1]["exceptions"][0]["time_validity"] = json!({ "_type": "DV_INTERVAL",
                "upper": { "_type": "DV_DATE_TIME", "value": "2026-13-01T00:00:00Z" },
                "lower_unbounded": true, "upper_unbounded": false,
                "lower_included": false, "upper_included": true });
        }),
        ("DV_DATE_TIME.Value_valid", |s| {
            s["directives"][1]["data"][0]["recorded"] = json!({ "_type": "DV_INTERVAL",
                "lower": { "_type": "DV_DATE_TIME", "value": "01-11-2026T00:00:00Z" },
                "lower_unbounded": false, "upper_unbounded": true,
                "lower_included": true, "upper_included": false });
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
    let mut p = policy();
    p["relationship_grace"]["value"] = json!("14 days");
    assert!(
        named(check_policy(&read(p))).contains(&"DV_DURATION.Value_valid".to_owned()),
        "a grace that is no ISO 8601 duration"
    );
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
    for list in [
        "classifications",
        "rules",
        "catalogue",
        "log_restrictions",
        "attested_care",
        "conflict_rules",
        "concealed_confidentialities",
        "value_sets",
    ] {
        let mut p = policy();
        p[list] = json!([]);
        refused.push(serde_json::from_value::<AccessPolicy>(p).is_err());
    }
    assert_eq!(
        refused,
        vec![true; 25],
        "every empty optional list is refused"
    );
}
