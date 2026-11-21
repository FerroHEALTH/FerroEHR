// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The JSON read's check that a stored body carries its own root `uid`
//! (#5755) reads only the root prefix the commit writes (#6198).

use super::carries_version_uid;

const OWN: &str = "8849182c-82ad-4088-a07f-48ead4180515::local.ferroehr::2";
const OTHER: &str = "8849182c-82ad-4088-a07f-48ead4180515::local.ferroehr::1";

/// A compact body with `root_uid` as its root member after
/// `archetype_node_id`, and `nested_uid` deep in its content.
fn body(root_uid: Option<&str>, nested_uid: &str) -> String {
    let root = root_uid.map_or_else(String::new, |uid| {
        format!(r#""uid":{{"_type":"OBJECT_VERSION_ID","value":"{uid}"}},"#)
    });
    format!(
        concat!(
            r#"{{"_type":"COMPOSITION","name":{{"_type":"DV_TEXT","value":"a \"}}, \\ name"}},"#,
            r#""archetype_node_id":"openEHR-EHR-COMPOSITION.report.v1",{root}"#,
            r#""language":{{"terminology_id":{{"value":"ISO_639-1"}},"code_string":"en"}},"#,
            r#""content":[{{"_type":"OBSERVATION","uid":{{"_type":"OBJECT_VERSION_ID","value":"{nested}"}}}}]}}"#,
        ),
        root = root,
        nested = nested_uid,
    )
}

#[test]
fn the_root_uid_of_this_version_is_carried() {
    let text = body(Some(OWN), OTHER);
    serde_json::from_str::<serde_json::Value>(&text).expect("the fixture is JSON");
    assert!(carries_version_uid(&text, OWN));
}

#[test]
fn a_body_without_its_own_uid_takes_the_parse_path() {
    assert!(
        !carries_version_uid(&body(None, OWN), OWN),
        "a nested uid equal to the version's is not the root uid"
    );
    assert!(!carries_version_uid(&body(None, OTHER), OWN));
}

#[test]
fn a_body_with_another_versions_uid_takes_the_parse_path() {
    assert!(!carries_version_uid(&body(Some(OTHER), OWN), OWN));
    let prefix = OWN.strip_suffix(":2").expect("the own uid ends in its version");
    assert!(
        !carries_version_uid(&body(Some(OWN), OTHER), prefix),
        "a uid that only starts with the version's is another one"
    );
}

#[test]
fn a_root_uid_past_the_leading_members_takes_the_parse_path() {
    let text = format!(
        r#"{{"_type":"COMPOSITION","language":{{"code_string":"en"}},"uid":{{"_type":"OBJECT_VERSION_ID","value":"{OWN}"}}}}"#
    );
    assert!(!carries_version_uid(&text, OWN));
}

#[test]
fn a_malformed_body_takes_the_parse_path() {
    for text in [
        "",
        "[]",
        "{",
        r#"{"_type""#,
        r#"{"name":{"value":"x""#,
        "not json",
    ] {
        assert!(!carries_version_uid(text, OWN), "{text:?}");
    }
}
