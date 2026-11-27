// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `PERSON`'s binding keys and an administrator's re-key over a stubbed
//! party and re-key route: a re-key and a refusal (#6661).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]
#![expect(
    clippy::expect_used,
    reason = "reading the recorded fixture is harness construction"
)]

use serde_json::Value;

use crate::ssr_app::{Viewer, id};

/// The issuer of the stubbed binding key.
const ISSUER: &str = "urn:example:staff-login";

/// The recorded exchange that serves the seeded `PERSON`, its document given
/// a binding key in its details.
fn keyed_person(party: &str) -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cdr/recording.json");
    let recording: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("the recording"))
            .expect("the recording is JSON");
    let person_path = format!("/ferroehr/rest/openehr/v1/demographic/person/{party}");
    let mut exchange = recording
        .into_iter()
        .find(|exchange| exchange.get("path").and_then(Value::as_str) == Some(person_path.as_str()))
        .expect("the recording serves the seeded person");
    let mut document: Value = serde_json::from_str(
        exchange
            .get("response")
            .and_then(Value::as_str)
            .expect("the person is a JSON text"),
    )
    .expect("the person is JSON");
    let details = serde_json::json!({
        "_type": "ITEM_TREE",
        "name": {"_type": "DV_TEXT", "value": "details"},
        "archetype_node_id": "id2",
        "items": [{
            "_type": "ELEMENT",
            "name": {"_type": "DV_TEXT", "value": "login"},
            "archetype_node_id": "id3",
            "value": {"_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": "5b0f-old", "type": "login"}
        }]
    });
    document
        .as_object_mut()
        .expect("the person is an object")
        .insert("details".to_owned(), details);
    exchange
        .as_object_mut()
        .expect("the exchange is an object")
        .insert("response".to_owned(), Value::String(document.to_string()));
    exchange
}

/// A stubbed re-key answered `status` with `response`.
fn rekey(status: u16, response: &str) -> Value {
    serde_json::json!({
        "method": "POST",
        "path": "/ferroehr/rest/extensions/v1/admin/party/binding-key",
        "body": "",
        "status": status,
        "headers": {"content-type": "application/json"},
        "response": response,
    })
}

/// The person's Overview lists its binding key with Re-key, confirms it
/// asking for the new id, and returns to the person once the CDR re-keyed.
#[tokio::test]
async fn a_binding_key_is_rekeyed() {
    let party = id("party");
    let page_path = format!("/parties/person/{party}");
    let viewer = Viewer::start_with(vec![keyed_person(&party), rekey(204, "")]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &page_path).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in ["Binding keys", ISSUER, "5b0f-old", "?rekey="] {
        assert!(
            page.main().contains(words),
            "the person shows {words}:\n{}",
            page.main()
        );
    }
    let confirm = viewer
        .page(
            &cookie,
            &format!("{page_path}?rekey={}", urlencoding::encode(ISSUER)),
        )
        .await;
    assert!(
        confirm.main().contains(r#"name="intent" value="rekey""#),
        "{}",
        confirm.main()
    );
    let done = viewer
        .send(
            http::Method::POST,
            &page_path,
            Some(&cookie),
            Some(&[("intent", "rekey"), ("issuer", ISSUER), ("id", "5b0f-new")]),
        )
        .await;
    assert_eq!(done.location(), page_path);
}

/// A refused re-key is drawn in the CDR's words.
#[tokio::test]
async fn a_refused_rekey_is_drawn_in_the_cdrs_words() {
    let party = id("party");
    let page_path = format!("/parties/person/{party}");
    let viewer = Viewer::start_with(vec![
        keyed_person(&party),
        rekey(
            409,
            r#"{"title":"Conflict","detail":"the key is already bound to another party"}"#,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .send(
            http::Method::POST,
            &page_path,
            Some(&cookie),
            Some(&[("intent", "rekey"), ("issuer", ISSUER), ("id", "taken")]),
        )
        .await;
    assert!(
        page.main()
            .contains("the key is already bound to another party"),
        "{}",
        page.main()
    );
}
