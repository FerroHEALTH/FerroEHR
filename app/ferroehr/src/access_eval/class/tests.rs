// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures are raw canonical JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

use serde_json::json;

use super::*;
use crate::party_binding::policy_mark::TERMINOLOGY;

const DOCTOR: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";
const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f82";
const OBJECT: &str = "8849182c-82ad-4088-a07f-48ead4180515";

fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

fn party(id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// Settings naming `who` everywhere a party or an object can be named, in
/// force from `from`.
fn settings(who: &str, from: &str) -> AccessDirectiveSettings {
    let value = json!({
        "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": term("restricted"),
        "gate_keepers": [
            { "_type": "PARTY_SELF" },
            { "_type": "PARTY_IDENTIFIED", "external_ref": party(who) }
        ],
        "directives": [{
            "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": format!("permit for {who}") },
            "archetype_node_id": "at0001",
            "kind": term("consent"),
            "effect": term("permit"),
            "emergency_override": false,
            "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "party": party(who) }],
            "data": [{ "_type": "ACCESS_DATA_SCOPE",
                "objects": [{ "_type": "OBJECT_REF", "namespace": "local",
                    "type": "VERSIONED_COMPOSITION",
                    "id": { "_type": "HIER_OBJECT_ID", "value": OBJECT } }],
                "origins": [format!("system-of-{who}")],
                "recorded": { "_type": "DV_INTERVAL",
                    "lower": { "_type": "DV_DATE_TIME", "value": from },
                    "lower_unbounded": false, "upper_unbounded": true,
                    "lower_included": true, "upper_included": false } }],
            "time_validity": { "_type": "DV_INTERVAL",
                "lower": { "_type": "DV_DATE_TIME", "value": from },
                "lower_unbounded": false, "upper_unbounded": true,
                "lower_included": true, "upper_included": false }
        }]
    });
    serde_json::from_value(value).expect("the test settings decode")
}

/// Settings that name different parties and instants make one class, and the
/// facts are the values that differ, in the order of the walk.
#[test]
fn settings_of_one_shape_share_a_class() {
    let first = classify(&settings(DOCTOR, "2026-11-01T00:00:00Z")).expect("classifies");
    let second = classify(&settings(NURSE, "2026-12-01T00:00:00Z")).expect("classifies");
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.class, second.class);
    assert_eq!(
        first.facts,
        vec![
            json!("2026-11-01T00:00:00Z"),
            json!(DOCTOR),
            json!([OBJECT]),
            json!([format!("system-of-{DOCTOR}")]),
            json!("2026-11-01T00:00:00Z"),
            json!(DOCTOR),
        ]
    );
}

/// The class of a class is itself: the walk places the same facts in it, so
/// a replica reads the layout off the stored class.
#[test]
fn a_class_classifies_as_itself() {
    let first = classify(&settings(DOCTOR, "2026-11-01T00:00:00Z")).expect("classifies");
    let class = Class::read(&first.class.to_string()).expect("the class reads");
    let Some(EhrAccessSettings::Directives(read)) = class.settings.as_ref() else {
        panic!("the class holds settings");
    };
    let again = classify(read).expect("classifies");
    assert_eq!(again.digest, first.digest);
    assert_eq!(class.layout.facts, first.facts.len());
    assert_eq!(
        class.layout.keepers,
        vec![KeeperLayout::Subject, KeeperLayout::Party(5)]
    );
    let directive = class.layout.directives.first().expect("one directive");
    assert_eq!((directive.from, directive.until), (Some(0), None));
    assert_eq!(directive.parties, vec![Some(1)]);
    assert_eq!(
        directive.data,
        vec![DataLayout {
            objects: Some(2),
            origins: Some(3),
            from: Some(4),
            until: None
        }]
    );
}

/// A gate-keeper that names the caller is a fact of the class; `PARTY_SELF`
/// names the subject whatever the facts.
#[test]
fn a_gate_keeper_reads_its_fact() {
    let first = classify(&settings(DOCTOR, "2026-11-01T00:00:00Z")).expect("classifies");
    let class = Class::read(&first.class.to_string()).expect("the class reads");
    let keys = |keeper: Option<&str>| CallerKeys {
        now: jiff::Timestamp::UNIX_EPOCH,
        parties: Vec::new().into(),
        keeper: keeper.map(str::to_owned),
    };
    let doctor = SettingsCaller::new(false, false, Some(DOCTOR.to_owned()));
    let named = class.keeper(&doctor);
    assert_eq!(
        resolve(&named, &first.facts, &keys(Some(DOCTOR))),
        Pred::True
    );
    assert_eq!(
        resolve(&named, &first.facts, &keys(Some(NURSE))),
        Pred::False
    );
    let subject = SettingsCaller::new(false, true, None);
    assert_eq!(class.keeper(&subject), Pred::True);
}
