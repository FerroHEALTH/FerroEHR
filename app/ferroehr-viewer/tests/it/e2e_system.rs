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
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end journeys over what the viewer says about the CDR itself: the
//! openEHR System API conformance manifest on About, and the per-family
//! `OpenAPI` documents on Server's API tab.
//!
//! Read-only throughout: nothing here writes to the CDR, so the journeys are
//! order-independent and leave the stack exactly as they found it.

use crate::common;

use common::{Harness, login_basic, main_text, wait_main_contains};

/// About shows what the CDR advertises through `OPTIONS {base_path}`: its
/// product identity, its REST API version, and the API groups it
/// mounts (the composed stack runs the standard groups plus the admin group).
#[tokio::test]
async fn system_panel_shows_the_conformance_manifest() {
    let Some(h) = Harness::start("system-conformance-manifest").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/about").await;

    wait_main_contains(&h, "restapi_specs_version").await;
    let text = main_text(&h).await;
    for endpoint in ["\"/ehr\"", "\"/query\"", "\"/admin\""] {
        assert!(
            text.contains(endpoint),
            "the manifest lists the mounted group {endpoint}: {text}"
        );
    }
    // The Server overview points at it.
    h.goto("/server").await;
    h.wait_css("main a[href='/about']").await;
    h.shot(1, "conformance-manifest").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The API tab's per-family picker: the EHR family lists the EHR paths,
/// picking **openEHR: Query** narrows the document to the query paths, and the
/// choice lives in the URL so it is shareable and survives a reload.
#[tokio::test]
async fn system_panel_selects_a_per_family_openapi_document() {
    let Some(h) = Harness::start("system-openapi-families").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/server/api").await;

    wait_main_contains(&h, "/ehr/{ehr_id}/composition").await;
    h.shot(1, "openapi-ehr").await;

    // Pick the Query family and submit the GET form: the URL carries the choice
    // and the rendered document drops the EHR paths.
    h.wait_css("#api-f").await.select_by_value("query").await;
    common::submit(&h, "form[action='/server/api'] button[type=submit]").await;
    h.wait_url_contains("family=query").await;
    wait_main_contains(&h, "/query/aql").await;
    let text = main_text(&h).await;
    assert!(
        !text.contains("/ehr/{ehr_id}/composition"),
        "the Query family document must not carry the EHR paths (got `{text}`)"
    );
    h.shot(2, "openapi-query-family").await;

    // The selection is URL state: a fresh load of the same URL shows the same
    // document with the picker still on Query.
    h.goto("/server/api?family=query").await;
    let selected = h.wait_css("#api-f").await.prop("value").await;
    assert_eq!(
        selected, "query",
        "a shared /server/api?family=query URL must reopen on that family"
    );
    h.shot(3, "openapi-query-family-reloaded").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
