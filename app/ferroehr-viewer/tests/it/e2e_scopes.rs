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
//! End-to-end journeys over Your access (`/access`): who the session is, how
//! it signed in, what decides its privileges, and the scope previewer.
//!
//! The previewer is a plain GET form: the scope travels in the URL and the
//! server parses it with the SMART v2 grammar (`openehr_smart`), so the page
//! works with no WebAssembly at all.

use crate::common;

use common::{Harness, count_matching, fill, login_basic, main_text};

/// Two scopes on one claim: a patient-compartment composition read with
/// search, and a user-level template grant naming one template.
const TWO_SCOPES: &str = "patient/composition-*.rs user/template-MyHospital::Template.v0.crud";

/// A scope whose permission tail the grammar rejects (`x` is not a
/// permission letter).
const BAD_PERMISSION: &str = "patient/composition-*.rx";

/// The previewed scopes, one item each.
const PREVIEWED: &str = "ul.fui-scopes li.fui-scope";

/// Your access states the identity, the policy source, and that a Basic
/// session carries no scopes; Server's SMART tab leads to it.
#[tokio::test]
async fn access_drawer_shows_effective_identity_and_policy_source() {
    let Some(h) = Harness::start("scopes-identity").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/access").await;
    h.shot(1, "access").await;

    let text = main_text(&h).await;
    for fact in ["ferroehr", "Basic sign-in", "no SMART scopes", "privileges"] {
        assert!(text.contains(fact), "Your access states `{fact}`: {text}");
    }

    h.goto("/server/smart").await;
    h.wait_css("main a[href='/access']").await.click().await;
    h.wait_url_contains("/access").await;
    h.wait_css("#scope").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The previewer reads a two-scope claim into two grants with the right
/// compartments and permissions, and explains a scope the grammar rejects.
#[tokio::test]
async fn scope_previewer_renders_grants_and_explains_an_invalid_scope() {
    let Some(h) = Harness::start("scopes-previewer").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/access").await;
    assert_eq!(
        count_matching(&h, PREVIEWED).await,
        0,
        "an empty previewer renders no grants"
    );

    fill(&h, "#scope", TWO_SCOPES).await;
    common::submit(&h, "#scope-read").await;
    h.wait_url_contains("scope=").await;
    h.wait_css(PREVIEWED).await;
    h.shot(1, "previewer-two-scopes").await;
    assert_eq!(
        count_matching(&h, PREVIEWED).await,
        2,
        "a two-scope claim renders two grants"
    );
    let rendered = h.wait_css("ul.fui-scopes").await.text().await;
    for expected in [
        "Patient",
        "Compositions",
        "Read",
        "Search",
        "User",
        "Operational templates",
        "MyHospital::Template.v0",
        "Create",
        "Delete",
    ] {
        assert!(
            rendered.contains(expected),
            "the parsed grants must show `{expected}`: {rendered}"
        );
    }
    // The permitted operations are the marked ones.
    assert_eq!(
        count_matching(&h, "ul.fui-scopes li:first-child .fui-scope-op.is-on").await,
        2,
        "`.rs` permits read and search only"
    );
    // The field keeps what was read.
    assert_eq!(h.wait_css("#scope").await.prop("value").await, TWO_SCOPES);

    fill(&h, "#scope", BAD_PERMISSION).await;
    common::submit(&h, "#scope-read").await;
    common::wait_main_contains(&h, "is not a permission tail").await;
    h.shot(2, "previewer-invalid-scope").await;
    assert_eq!(
        count_matching(&h, PREVIEWED).await,
        1,
        "one scope in, one item out"
    );

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
