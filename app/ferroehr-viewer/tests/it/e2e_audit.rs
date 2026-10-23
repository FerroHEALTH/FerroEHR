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
//! End-to-end audit-log journeys: the `/audit` browser over the CDR's
//! RESTful-ATNA ITI-81 retrieval. Driven by `scripts/ui-e2e.sh`; each test
//! owns its own [`Harness`] and skips with a printed reason when the harness
//! environment is absent.
//!
//! The composed CDR audits every request by default (the local Audit Record
//! Repository is on out of the box), so by the time these journeys run the
//! stack's own activity has populated the trail. The audit surface is
//! admin-only under RBAC: the journeys log in as the quickstart
//! `ferroehr-admin` Basic user (`UI_E2E_ADMIN_USER`/`UI_E2E_ADMIN_PASS`
//! override), and one journey proves the operator gate by visiting as the
//! plain `ferroehr` user.

use crate::common;

use common::{Harness, env, login_basic, login_basic_as, wait_text_contains};

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`).
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// As the admin: the audit table renders real records (the stack's own
/// audited activity), a row opens the record as stored, and a nothing-matches
/// filter renders the empty state.
#[tokio::test]
async fn admin_browses_audit_records_and_the_empty_state() {
    let Some(h) = Harness::start("audit-browse").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    // The viewer's own CDR calls are audited, so loading a screen guarantees
    // at least one record before browsing.
    h.goto("/templates").await;
    h.wait_css(common::SHELL_FOOTER).await;

    h.goto("/audit").await;
    h.wait_css("main table tbody tr").await;
    h.shot(1, "audit-records").await;

    // A row opens its record, which shows the stored FHIR AuditEvent.
    h.wait_css("main table tbody a.fui-row-link").await.click().await;
    h.wait_url_contains("/audit/").await;
    wait_text_contains(&h, "#audit-record-text", "\"resourceType\": \"AuditEvent\"").await;
    // The hydrated copy button appears; it is hidden without WebAssembly.
    h.wait_hydrated().await;
    common::wait_visible(&h, "#audit-record").await;
    h.shot(2, "audit-record").await;

    // A filter that cannot match renders the empty state, with the filter
    // named and a way to clear it.
    h.goto("/audit?patient=e2e-audit-no-such-patient").await;
    common::wait_main_contains(&h, "No audit record matches these filters").await;
    h.wait_css("main a[href='/audit']").await;
    h.shot(3, "audit-empty").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// As the plain USER: the audit surface is refused (the ATNA trail is an
/// operator surface) and the refusal renders as a readable inline error —
/// never a blank screen.
#[tokio::test]
async fn plain_user_is_refused_the_audit_surface() {
    let Some(h) = Harness::start("audit-forbidden").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/audit").await;
    h.wait_css(common::SHELL_FOOTER).await;
    // The CDR's 403 surfaces as the page's refusal, with the CDR's words.
    common::wait_main_contains(&h, "403").await;
    common::wait_main_contains(&h, "ADMIN").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}
