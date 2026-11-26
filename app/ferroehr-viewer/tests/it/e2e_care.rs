// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a browser journey asserts by panicking, and the shared harness panics when a configured stack cannot be driven"
)]
#![allow(
    clippy::print_stdout,
    reason = "the skip-with-reason and progress lines ARE this suite's report"
)]
#![allow(
    unreachable_pub,
    dead_code,
    reason = "the shared `common` harness is compiled into every journey binary; each one drives a different subset of it"
)]
#![allow(
    clippy::disallowed_types,
    reason = "the journey seeds its parties as raw JSON by the testing rule (.claude/rules/testing.md)"
)]
//! End-to-end journey over care relationships: the administrator adds one
//! on a patient's Care page, opens it, ends it behind its confirmation, and
//! finds its two versions and the patient in the carer's list.

use crate::common;

use common::{Harness, env, fill, login_basic_as, main_text, submit, wait_main_contains};

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`).
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// A party of `rm_type` and `archetype` whose `details` hold `details`,
/// created as the administrator; its container id.
async fn party(
    cdr: &str,
    segment: &str,
    archetype: &str,
    name: &str,
    details: Vec<serde_json::Value>,
) -> String {
    let (user, pass) = admin_credentials();
    let rm_type = segment.to_uppercase();
    let mut body = serde_json::json!({
        "_type": rm_type, "name": { "_type": "DV_TEXT", "value": rm_type },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "identities": [{ "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "archetype_node_id": "at0001",
            "details": { "_type": "ITEM_TREE", "name": { "_type": "DV_TEXT", "value": "Details" },
                "archetype_node_id": "at0002", "items": [{ "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "Name" }, "archetype_node_id": "at0003",
                    "value": { "_type": "DV_TEXT", "value": name } }] } }]
    });
    if let Some(party) = body.as_object_mut().filter(|_| !details.is_empty()) {
        party.insert(
            "details".to_owned(),
            serde_json::json!({ "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "details" }, "archetype_node_id": "at0010",
                "items": details }),
        );
    }
    let answer: serde_json::Value = reqwest::Client::new()
        .post(format!(
            "{cdr}/ferroehr/rest/openehr/v1/demographic/{segment}"
        ))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .header("Prefer", "return=representation")
        .json(&body)
        .send()
        .await
        .expect("the CDR answers the seed")
        .json()
        .await
        .expect("the seed answers a party");
    answer
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .and_then(|uid| uid.split("::").next())
        .expect("the party has a uid")
        .to_owned()
}

#[tokio::test]
async fn admin_adds_ends_and_reads_a_care_relationship() {
    let Some(h) = Harness::start("care").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP care: UI_E2E_CDR_URL is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let provider = format!("e2e-{}", jiff::Timestamp::now().as_millisecond());
    let patient = party(
        &cdr,
        "person",
        "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "Care journey patient",
        Vec::new(),
    )
    .await;
    let carer = party(
        &cdr,
        "person",
        "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "Care journey carer",
        Vec::new(),
    )
    .await;
    party(
        &cdr,
        "organisation",
        "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
        "Care journey hospital",
        vec![
            serde_json::json!({ "_type": "ELEMENT", "archetype_node_id": "at0011",
            "name": { "_type": "DV_TEXT", "value": "Identifier" },
            "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example:e2e-providers",
                "id": provider, "type": "ORGANISATION" } }),
        ],
    )
    .await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto(&format!("/parties/person/{patient}/care")).await;
    wait_main_contains(&h, "Add a care relationship").await;
    fill(&h, "#care-carer", &carer).await;
    fill(&h, "#care-issuer", "urn:example:e2e-providers").await;
    fill(&h, "#care-provider", &provider).await;
    fill(&h, "#care-new-from", "2026-01-01").await;
    h.shot(1, "add").await;
    submit(&h, "button[name='intent'][value='add']").await;
    h.wait_url_contains("/care/").await;
    wait_main_contains(&h, "This care relationship").await;
    h.shot(2, "relationship").await;

    h.wait_css("button[commandfor='care-end']")
        .await
        .click()
        .await;
    common::wait_visible(&h, "#care-end").await;
    submit(&h, "#care-end button[value='end']").await;
    wait_main_contains(&h, "Versions").await;
    let text = main_text(&h).await;
    assert!(
        text.contains("attender"),
        "the relationship keeps its role: {text}"
    );
    assert!(
        common::is_present(&h, "main a[href*='%3A%3A1']").await,
        "the ended relationship lists its first version"
    );

    h.goto(&format!("/parties/person/{carer}/care")).await;
    wait_main_contains(&h, "In their care").await;
    assert!(
        main_text(&h).await.contains("PERSON"),
        "the carer's list names the patient"
    );
    h.shot(3, "in-their-care").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
