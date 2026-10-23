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
//! End-to-end journeys over the stored-query lifecycle: the reverse lift of
//! a stored definition back into Build, and the parameterised server-side run
//! of a stored query through the three openEHR version-resolution forms.
//!
//! Both journeys save their own stored query on Query's AQL tab and delete it
//! again at the end, so they touch nothing another journey (or the
//! documentation-screenshot pass) depends on. Deleting a stored query needs
//! the CDR's ADMIN role, so they sign in as the quickstart admin user
//! (`UI_E2E_ADMIN_USER`/`UI_E2E_ADMIN_PASS`, defaulting to
//! `ferroehr-admin`/`ferroehr` from `docker/ferroehr.dev.toml`).

use crate::common;

use common::{Harness, confirm_in_dialog, env, fill, login_basic_as, wait_main_contains};

/// The namespace half of every stored-query name these journeys save.
const QUERY_NAMESPACE: &str = "org.example";

/// The bare name of the query the LIFT journey expects the builder to accept.
const LIFTABLE_BARE_NAME: &str = "e2e-lift-representable";
/// Its qualified name.
const LIFTABLE_QUERY_NAME: &str = "org.example::e2e-lift-representable";
/// The bare name of the query the LIFT journey expects the builder to REFUSE.
const REFUSED_BARE_NAME: &str = "e2e-lift-refused";
/// Its qualified name.
const REFUSED_QUERY_NAME: &str = "org.example::e2e-lift-refused";
/// The bare name of the query the RUN journey executes with parameters.
const RUNNER_BARE_NAME: &str = "e2e-parameterised-run";
/// Its qualified name.
const RUNNER_QUERY_NAME: &str = "org.example::e2e-parameterised-run";

/// The template `scripts/ui-e2e.sh` seeds, with compositions committed against
/// it — the value the RUN journey binds to the query's `$template` parameter.
const SEEDED_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// A query in EXACTLY the shape the builder's lowering emits (fixed
/// `FROM EHR e CONTAINS COMPOSITION c`, a template restriction, a `LIMIT`), so
/// the reverse lift must accept it and re-lower it to this same text.
const LIFTABLE_AQL: &str = "SELECT c FROM EHR e CONTAINS COMPOSITION c \
     WHERE c/archetype_details/template_id/value='minimal_evaluation.en.v1' LIMIT 50";

/// A query OUTSIDE the builder's envelope: it binds a `$parameter`, which the
/// point-and-click editor has no field for. It must open with the refusal notice
/// and an empty builder, never a lossy half-lift.
const REFUSED_AQL: &str = "SELECT c/name/value AS name FROM EHR e CONTAINS COMPOSITION c \
     WHERE c/archetype_details/template_id/value=$template";

/// The parameterised query the RUN journey stores and executes.
const RUNNER_AQL: &str = "SELECT c/name/value AS name FROM EHR e CONTAINS COMPOSITION c \
     WHERE c/archetype_details/template_id/value=$template";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`) — the stored-query
/// delete used for self-cleaning is RBAC-gated.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Save `aql` as version 1.0.0 of `bare_name` in [`QUERY_NAMESPACE`] from
/// Query's AQL tab.
async fn store_query(h: &Harness, bare_name: &str, aql: &str) {
    h.goto("/query/aql").await;
    fill(h, "#aql", aql).await;
    h.wait_css("button[commandfor='save-query']")
        .await
        .click()
        .await;
    common::wait_visible(h, "#sv-n").await;
    fill(h, "#sv-ns", QUERY_NAMESPACE).await;
    fill(h, "#sv-n", bare_name).await;
    fill(h, "#sv-v", "1.0.0").await;
    common::submit(h, "#save-query button[value='save']").await;
    h.wait_url_contains("version=1.0.0").await;
}

/// Delete version 1.0.0 of `qualified_name` from its page.
async fn delete_stored_query(h: &Harness, qualified_name: &str) {
    h.goto(&format!(
        "/queries/{}/1.0.0",
        urlencoding::encode(qualified_name)
    ))
    .await;
    h.wait_css("summary[aria-controls='stored-more']")
        .await
        .click()
        .await;
    confirm_in_dialog(
        h,
        "button[commandfor='delete-stored-query']",
        "#delete-stored-query button[type=submit]",
    )
    .await;
    h.wait_url_contains("/queries").await;
}

/// The AQL with its whitespace collapsed, for a comparison that ignores line
/// breaks.
fn normalized(aql: &str) -> String {
    aql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The reverse lift: a stored query inside the builder's envelope opens in
/// Build and lowers back to byte-identical AQL, while one outside it offers no
/// way into Build at all.
#[tokio::test]
async fn stored_query_lifts_back_into_the_builder() {
    let Some(h) = Harness::start("stored-query-reverse-lift").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    store_query(&h, LIFTABLE_BARE_NAME, LIFTABLE_AQL).await;
    common::submit(&h, "button[value='lift']").await;
    h.wait_url_contains("/query").await;
    let lowered = h
        .wait_css("#query-form input[name='aql']")
        .await
        .prop("value")
        .await;
    assert_eq!(
        normalized(&lowered),
        normalized(LIFTABLE_AQL),
        "the lifted builder state must re-lower to the stored query verbatim"
    );
    assert_eq!(
        h.wait_css("#q-t").await.prop("value").await,
        SEEDED_TEMPLATE_ID,
        "the lift picks the query's template"
    );
    h.shot(1, "stored-query-lifted-into-builder").await;

    store_query(&h, REFUSED_BARE_NAME, REFUSED_AQL).await;
    assert!(
        !common::is_present(&h, "button[value='lift']").await,
        "a query outside the builder's envelope offers no Open in Build"
    );
    h.shot(2, "stored-query-lift-refused").await;

    delete_stored_query(&h, LIFTABLE_QUERY_NAME).await;
    delete_stored_query(&h, REFUSED_QUERY_NAME).await;
    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The parameterised runner: a stored query with a `$parameter` runs with its
/// value bound through each of the three openEHR version-resolution forms
/// (exact, prefix, latest), each saying the request it sends.
#[tokio::test]
async fn stored_query_runs_with_parameters_in_every_resolution_form() {
    let Some(h) = Harness::start("stored-query-parameterised-run").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    store_query(&h, RUNNER_BARE_NAME, RUNNER_AQL).await;

    let run = format!(
        "/queries/{}/1.0.0/run",
        urlencoding::encode(RUNNER_QUERY_NAME)
    );
    h.goto("/queries").await;
    h.wait_css(&format!("main a[href='{run}']")).await;
    for (mode, version, sends, slug) in [
        (
            "exact",
            "1.0.0",
            format!("POST /query/{RUNNER_QUERY_NAME}/1.0.0"),
            "exact",
        ),
        (
            "prefix",
            "1",
            format!("POST /query/{RUNNER_QUERY_NAME}/1"),
            "prefix",
        ),
        (
            "latest",
            "",
            format!("POST /query/{RUNNER_QUERY_NAME}"),
            "latest",
        ),
    ] {
        h.goto(&run).await;
        h.wait_css(&format!("label[for='res-{mode}']"))
            .await
            .click()
            .await;
        fill(&h, "#rv", version).await;
        fill(&h, "#p-template", SEEDED_TEMPLATE_ID).await;
        common::submit(&h, "#stored-query-run button[type=submit]").await;
        wait_main_contains(&h, &sends).await;
        h.wait_css("main table tbody tr").await;
        h.shot(1, &format!("stored-query-run-{slug}")).await;
    }

    delete_stored_query(&h, RUNNER_QUERY_NAME).await;
    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}
