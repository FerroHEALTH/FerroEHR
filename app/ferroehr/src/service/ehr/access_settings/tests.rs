// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use serde_json::{Value, json};

use super::*;
use crate::party_binding::policy_mark::TERMINOLOGY;

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// A directive of `kind` and `effect`, with `extra` members.
fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
    let mut d = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Directive" },
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term(effect),
        "emergency_override": false
    });
    if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
        d.extend(extra.clone());
    }
    d
}

/// Settings of the default `code` holding `directives`.
fn settings(code: &str, directives: &[Value]) -> Value {
    let mut s = json!({ "_type": SETTINGS_TYPE, "default_access": term(code) });
    if !directives.is_empty() {
        s["directives"] = json!(directives);
    }
    s
}

/// The refusal message `settings` draw, which must be a `400`.
fn refused(settings: &Value) -> String {
    match check_written_settings(settings) {
        Err(ServiceError::BadRequest(refusal)) => refusal.message,
        other => panic!("a 400 for {settings}, got {other:?}"),
    }
}

#[test]
fn settings_of_the_scheme_are_accepted() {
    let consent = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "caller_kind": term("professional") }] }),
    );
    check_written_settings(&settings("restricted", &[consent])).expect("valid settings");
    check_written_settings(&settings("open", &[])).expect("open settings");
}

/// A body of the removed scheme, of any other scheme or of none is refused,
/// naming the `_type`.
#[test]
fn another_scheme_is_refused() {
    let old = json!({ "_type": "FERROEHR_ACCESS_CONTROL_V1", "default_access": "restricted" });
    assert!(refused(&old).contains("\"FERROEHR_ACCESS_CONTROL_V1\""));
    assert!(refused(&json!({ "default_access": term("open") })).contains("absent"));
    assert!(refused(&json!({ "_type": "SOME_SCHEME" })).contains("SOME_SCHEME"));
}

/// A member the class does not declare, or an empty list, is refused by the
/// strict reader.
#[test]
fn a_member_that_does_not_decode_is_refused() {
    let mut extra = settings("open", &[]);
    extra["access_list"] = json!([]);
    assert!(refused(&extra).contains("do not read"));
    let mut empty = settings("open", &[]);
    empty["gate_keepers"] = json!([]);
    assert!(refused(&empty).contains("do not read"));
}

/// Each invariant of the settings and the classes they hold is refused,
/// named as the BMM names it.
#[test]
fn each_invariant_is_refused_by_name() {
    let opt_out = || directive("opt_out", "deny", &json!({}));
    let cases = [
        (
            settings("closed", &[]),
            "ACCESS_DIRECTIVE_SETTINGS.Default_access_valid",
        ),
        (
            settings("open", &[opt_out(), opt_out()]),
            "ACCESS_DIRECTIVE_SETTINGS.Opt_out_unique",
        ),
        (
            settings("open", &[directive("wish", "deny", &json!({}))]),
            "ACCESS_DIRECTIVE.Kind_valid",
        ),
        (
            settings("open", &[directive("consent", "allow", &json!({}))]),
            "ACCESS_DIRECTIVE.Effect_valid",
        ),
        (
            settings("open", &[directive("restriction", "permit", &json!({}))]),
            "ACCESS_DIRECTIVE.Restriction_denies",
        ),
        (
            settings(
                "open",
                &[directive(
                    "opt_out",
                    "deny",
                    &json!({ "data": [{ "templates": ["t.v1"] }] }),
                )],
            ),
            "ACCESS_DIRECTIVE.Opt_out_denies_whole_record",
        ),
        (
            settings(
                "open",
                &[directive(
                    "restriction",
                    "deny",
                    &json!({ "exceptions": [directive("consent", "deny", &json!({}))] }),
                )],
            ),
            "ACCESS_DIRECTIVE.Exceptions_opposite",
        ),
        (
            settings(
                "open",
                &[directive(
                    "consent",
                    "permit",
                    &json!({ "applies_to": [{}] }),
                )],
            ),
            "ACCESS_ACTOR_SCOPE.Not_empty",
        ),
        (
            settings(
                "open",
                &[directive(
                    "consent",
                    "permit",
                    &json!({ "applies_to": [{ "relationship": term("friend") }] }),
                )],
            ),
            "ACCESS_ACTOR_SCOPE.Relationship_valid",
        ),
        (
            settings(
                "open",
                &[directive(
                    "consent",
                    "permit",
                    &json!({ "applies_to": [{ "caller_kind": term("robot") }] }),
                )],
            ),
            "ACCESS_ACTOR_SCOPE.Caller_kind_valid",
        ),
        (
            settings(
                "open",
                &[directive("restriction", "deny", &json!({ "data": [{}] }))],
            ),
            "ACCESS_DATA_SCOPE.Not_empty",
        ),
    ];
    for (body, invariant) in cases {
        let message = refused(&body);
        assert!(message.contains(invariant), "{invariant}: {message}");
    }
}

/// A code of the right group in another terminology is outside the group.
#[test]
fn a_code_of_another_terminology_is_refused() {
    let mut s = settings("open", &[]);
    s["default_access"]["defining_code"]["terminology_id"]["value"] = json!("local");
    assert!(refused(&s).contains("Default_access_valid"));
}

#[test]
fn stored_settings_read_their_default_and_gate_keepers() {
    let mut s = settings("consent_required", &[]);
    s["gate_keepers"] = json!([{ "_type": "PARTY_SELF" }]);
    let access = json!({ "_type": "EHR_ACCESS", "settings": s });
    let read = EhrAccessSettings::from_ehr_access(&access)
        .expect("the settings read")
        .expect("settings");
    assert_eq!(read.default_access(), DefaultAccess::ConsentRequired);
    assert_eq!(read.gate_keepers().len(), 1);
    assert!(matches!(
        EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS" })),
        Ok(None)
    ));
}

/// Stored settings that do not read hold the EHR closed: restricted, with no
/// gate-keeper.
#[test]
fn unreadable_stored_settings_hold_the_ehr_closed() {
    let access = json!({ "_type": "EHR_ACCESS",
                         "settings": { "_type": "FERROEHR_ACCESS_CONTROL_V1" } });
    assert!(EhrAccessSettings::from_ehr_access(&access).is_err());
    let held = EhrAccessSettings::held_or_read(&access).expect("held");
    assert_eq!(held, EhrAccessSettings::HeldClosed);
    assert_eq!(held.default_access(), DefaultAccess::Restricted);
    assert_eq!(held.gate_keepers(), &[]);
}
