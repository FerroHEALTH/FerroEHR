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
//! End-to-end journeys over the releases of one template (#4506): the list
//! groups them under the newest, the template's header names its release and
//! switches to another with FerroUI's scripts loaded and refused, and Compare
//! names the paths one release adds to the other.
//!
//! The pair is the corpus's two releases of `test_versioned` (ADL 1.4,
//! `corpus/fixtures/opt/valid/versioned.v1.opt` and `versioned.v2.opt`),
//! the second adding one element. Every scene is SEED-AND-CLEAN over the
//! CDR's REST API: it uploads the pair when absent and removes it at scene
//! end as the ADMIN dev user, so a shared stack is left as it was found.

use crate::common;

use common::{Harness, env, login_basic, wait_main_contains};
use reqwest::StatusCode;

/// The earlier release.
const V1: &str = "test_versioned.en.v1";

/// The later release, which adds an element.
const V2: &str = "test_versioned.en.v2";

/// The pair, each with its corpus file.
const PAIR: [(&str, &str); 2] = [(V1, "versioned.v1.opt"), (V2, "versioned.v2.opt")];

/// The CDR the scene seeds, or `None` (with the reason printed) when the
/// harness names none.
fn cdr() -> Option<String> {
    let cdr = env("UI_E2E_CDR_URL");
    if cdr.is_none() {
        println!("SKIP template releases: UI_E2E_CDR_URL unset, so the pair cannot be seeded");
    }
    cdr
}

/// Upload the pair over the Definition API (`201` stored, `409` already
/// there).
///
/// # Panics
/// On any other answer.
async fn seed(cdr: &str) {
    let http = reqwest::Client::new();
    for (_, file) in PAIR {
        let source = std::fs::read_to_string(format!(
            "{}/../../corpus/fixtures/opt/valid/{file}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("the corpus fixture exists");
        let status = http
            .post(format!(
                "{cdr}/ferroehr/rest/openehr/v1/definition/template/adl1.4"
            ))
            .basic_auth("ferroehr", Some("ferroehr"))
            .header("Content-Type", "application/xml")
            .body(source)
            .send()
            .await
            .expect("upload a release of the pair")
            .status();
        assert!(
            status == StatusCode::CREATED || status == StatusCode::CONFLICT,
            "seeding {file} -> {status}"
        );
    }
}

/// Remove the pair as the ADMIN dev user (`204` deleted, `404` gone).
///
/// # Panics
/// On any other answer.
async fn clean(cdr: &str) {
    let http = reqwest::Client::new();
    let user = env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned());
    let pass = env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    for (id, _) in PAIR {
        let status = http
            .delete(format!(
                "{cdr}/ferroehr/rest/extensions/v1/admin/template/{id}"
            ))
            .basic_auth(&user, Some(&pass))
            .send()
            .await
            .expect("delete a release of the pair")
            .status();
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND,
            "removing {id} -> {status}"
        );
    }
}

/// The row link to the release `id`, wherever the row is.
fn row_link(id: &str) -> String {
    format!("a.fui-row-link[href='/templates/{id}']")
}

/// The header's link to the release `id` among the releases.
fn release_link(id: &str) -> String {
    format!("#template-releases a[role='option'][href='/templates/{id}']")
}

/// The list draws the pair as one group under the newest release, and the
/// newest's header switches to the earlier release with the scripts loaded.
#[tokio::test]
async fn the_releases_group_in_the_list_and_switch_in_the_header() {
    let Some(cdr) = cdr() else {
        return;
    };
    let Some(h) = Harness::start("template-releases").await else {
        return;
    };
    seed(&cdr).await;
    login_basic(&h).await;
    h.goto("/templates").await;
    h.wait_css(&format!(
        "main tbody:has(> tr:first-child {}) tr:not(:first-child) {}",
        row_link(V2),
        row_link(V1)
    ))
    .await;
    wait_main_contains(&h, "2 releases").await;
    h.shot(1, "releases-grouped").await;

    h.goto(&format!("/templates/{V2}")).await;
    h.wait_scripts().await;
    h.wait_css("header summary[aria-controls='template-releases']")
        .await
        .click()
        .await;
    wait_main_contains(&h, "Newest").await;
    h.shot(2, "releases-open").await;
    h.wait_css(&release_link(V1)).await.click().await;
    h.wait_url_contains(V1).await;
    wait_main_contains(&h, "On this page").await;
    h.shot(3, "release-switched").await;

    clean(&cdr).await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// With FerroUI's scripts refused, the header's releases open natively and
/// every release is a link.
#[tokio::test]
async fn the_releases_switch_in_the_header_with_the_scripts_refused() {
    let Some(cdr) = cdr() else {
        return;
    };
    let Some(h) = Harness::start_without_scripts("template-releases-no-script").await else {
        return;
    };
    seed(&cdr).await;
    login_basic(&h).await;
    h.goto(&format!("/templates/{V2}")).await;
    h.wait_css("header summary[aria-controls='template-releases']")
        .await
        .click()
        .await;
    h.wait_css(&release_link(V1)).await.click().await;
    h.wait_url_contains(V1).await;
    wait_main_contains(&h, "On this page").await;
    assert_eq!(
        0,
        common::count_matching(&h, "html[data-fui-script]").await,
        "no script ran"
    );
    h.shot(1, "release-switched-no-script").await;

    clean(&cdr).await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// Compare reads the newest release against the one before it, names the
/// element the later release adds, and takes another pair as a plain `GET`
/// form.
#[tokio::test]
async fn compare_names_the_paths_the_later_release_adds() {
    let Some(cdr) = cdr() else {
        return;
    };
    let Some(h) = Harness::start("template-compare").await else {
        return;
    };
    seed(&cdr).await;
    login_basic(&h).await;
    h.goto(&format!("/templates/{V2}")).await;
    h.wait_css("a.fui-tab[data-tab='compare']")
        .await
        .click()
        .await;
    h.wait_url_contains("tab=compare").await;
    wait_main_contains(&h, &format!("The paths from {V1} to {V2}")).await;
    h.wait_css("main .fui-change-added").await;
    h.shot(1, "compare").await;

    h.wait_css("#cmp-from").await.select_by_value(V2).await;
    common::submit(&h, "form:has(#cmp-from) button[type=submit]").await;
    h.wait_url_contains(&format!("from={V2}")).await;
    wait_main_contains(&h, "Both sides name the same release").await;
    h.shot(2, "compare-same").await;

    clean(&cdr).await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}
