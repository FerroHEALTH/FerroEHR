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
    reason = "the journey seeds its records as raw JSON by the testing rule (.claude/rules/testing.md)"
)]
//! End-to-end journey over a professional's binding key: the administrator
//! opens a professional's PERSON, re-keys the Basic-user login it binds, and
//! finds the new key in its place.
//!
//! The PERSON is the journey's own, made on the template the development
//! posture binds professionals with, so no login of the stack moves.

use crate::common;

use common::{Harness, fill, login_basic_as, main_text, submit, wait_main_contains};

/// The identifier issuer the Basic users bind under
/// (`docker/ferroehr.dev.toml`, `[auth.issuer.basic_users.party]`).
const ISSUER: &str = "urn:example:basic-user";

/// A healthcare professional's PERSON whose binding key is the login `key`.
fn professional(key: &str) -> serde_json::Value {
    let archetype = "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1";
    serde_json::json!({
        "_type": "PERSON",
        "name": { "_type": "DV_TEXT", "value": "Re-key journey professional" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": format!("{archetype}.0.0") } },
        "identities": [{ "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "Person name" }, "archetype_node_id": "id4",
            "details": { "_type": "ITEM_TREE", "name": { "_type": "DV_TEXT", "value": "id5" },
                "archetype_node_id": "id5" } }],
        "details": { "_type": "ITEM_TREE", "name": { "_type": "DV_TEXT", "value": "id2" },
            "archetype_node_id": "id2", "items": [{ "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": "Identifier main data" },
                "archetype_node_id": "id3",
                "value": { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": key, "type": "login" } }] }
    })
}

#[tokio::test]
async fn admin_rekeys_a_professionals_binding_key() {
    let Some(h) = Harness::start("rekey").await else {
        return;
    };
    let Some(cdr) = common::cdr_url("rekey") else {
        h.finish().await;
        return;
    };
    let old = common::fresh("e2e-login");
    let new = common::fresh("e2e-login-rekeyed");
    let party = common::admin_create(&cdr, "/demographic/person", &professional(&old)).await;
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto(&format!("/parties/person/{party}")).await;
    wait_main_contains(&h, "Binding keys").await;
    assert!(
        main_text(&h).await.contains(&old),
        "the party lists the key it binds"
    );
    h.shot(1, "binding-keys").await;

    h.wait_css("main a[href*='?rekey=']").await.click().await;
    h.wait_url_contains("rekey=").await;
    fill(&h, "#k-id", &new).await;
    h.shot(2, "rekey-form").await;
    submit(&h, "form:has(#k-id) button[type=submit]").await;
    h.wait_url_not_contains("rekey=").await;
    wait_main_contains(&h, &new).await;
    assert!(
        !main_text(&h).await.contains(&old),
        "the old key is gone once re-keyed"
    );
    h.shot(3, "rekeyed").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
