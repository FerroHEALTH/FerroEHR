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
//! End-to-end journeys over the viewer's **System panel** (`/system`): the
//! openEHR System API conformance manifest and the per-family `OpenAPI` document
//! selector.
//!
//! Read-only throughout — nothing here writes to the CDR, so the journeys are
//! order-independent and leave the stack exactly as they found it.

use crate::common;

use common::{Harness, login_basic, wait_text_contains};

/// The conformance-manifest card renders what the CDR advertises about itself
/// through `OPTIONS {base_path}`: its product identity, its claimed conformance
/// profile, and the API groups it actually mounts (the composed stack runs the
/// standard groups plus the admin group).
#[tokio::test]
async fn system_panel_shows_the_conformance_manifest() {
    let Some(h) = Harness::start("system-conformance-manifest").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/system").await;

    h.wait_css("#conformance-manifest").await;
    // The live mounted-group set: `/ehr` and `/query` are always there, and the
    // composed CDR runs with the admin group enabled.
    h.wait_css("[data-manifest-endpoint='/ehr']").await;
    h.wait_css("[data-manifest-endpoint='/query']").await;
    h.wait_css("[data-manifest-endpoint='/admin']").await;
    // The profile the CDR claims is stated, not implied.
    wait_text_contains(&h, "#conformance-manifest", "conformance profile").await;
    h.shot(1, "conformance-manifest").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The `OpenAPI` card's per-family selector: the complete surface lists paths from
/// several families, picking **openEHR — Query** narrows the document to the
/// query paths, and the choice lives in the URL so it is shareable and survives
/// a reload.
#[tokio::test]
async fn system_panel_selects_a_per_family_openapi_document() {
    let Some(h) = Harness::start("system-openapi-families").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/system").await;

    // The default document is the complete surface: it carries the AQL endpoint
    // AND the EHR endpoints.
    h.wait_css("#openapi-family").await;
    wait_text_contains(&h, "#openapi-family-card", "/query/aql").await;
    wait_text_contains(&h, "#openapi-family-card", "/ehr/{ehr_id}").await;
    h.shot(1, "openapi-complete").await;

    // Pick the Query family and submit the GET form: the URL carries the choice
    // and the rendered document drops the EHR paths.
    let option = h.wait_css("#openapi-family option[value='query']").await;
    option.click().await;
    h.wait_css("#openapi-family-show").await.click().await;
    h.wait_url_contains("openapi=query").await;
    wait_text_contains(&h, "#openapi-family-card", "/query/aql").await;
    let text = h.wait_css("#openapi-family-card").await.text().await;
    assert!(
        !text.contains("/ehr/{ehr_id}/composition"),
        "the Query family document must not carry the EHR paths (got `{text}`)"
    );
    h.shot(2, "openapi-query-family").await;

    // The selection is URL state: a fresh load of the same URL shows the same
    // document with the selector still on Query.
    h.goto("/system?openapi=query").await;
    let selected = h.wait_css("#openapi-family").await.prop("value").await;
    assert_eq!(
        selected, "query",
        "a shared /system?openapi=query URL must reopen on that family"
    );
    h.shot(3, "openapi-query-family-reloaded").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
