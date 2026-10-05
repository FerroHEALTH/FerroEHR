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
//! End-to-end journeys over the **stored-query lifecycle** the viewer closes:
//! the reverse LIFT of a stored definition back into the point-and-click
//! builder, and the parameterised server-side RUN of a stored query through the
//! three openEHR version-resolution forms.
//!
//! Both journeys save their own stored query through the real raw-editor UI and
//! delete it again at the end, so they touch nothing another journey (or the
//! documentation-screenshot pass) depends on. Deleting a stored query needs the
//! CDR's ADMIN role, so they sign in as the quickstart admin user
//! (`UI_E2E_ADMIN_USER`/`UI_E2E_ADMIN_PASS`, defaulting to
//! `ferroehr-admin`/`ferroehr` from `docker/ferroehr.dev.toml`).
//!
//! Hardening that is load-bearing here: after ANY save, wait for its outcome
//! toast BEFORE navigating — navigation aborts the in-flight server-fn fetch and
//! the query is then not stored when the next step looks for it.

use crate::common;

use std::time::Duration;

use common::{
    Harness, confirm_in_dialog, count_matching, env, login_basic_as, poll_until, poll_until_for,
    wait_css_absent,
};

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

/// Store `aql` at version 1.0.0 under `bare_name` through the real raw-editor
/// UI, then WAIT for the save's own outcome toast before returning — navigating
/// earlier aborts the in-flight server-fn call.
///
/// The Save button gates on a name plus a storable version triple, so its
/// enabling doubles as the hydration signal; the typing is retried until it
/// enables (the sibling journeys' precedent).
///
/// # Panics
/// When the Save button never enables, or the save never reports an outcome.
async fn store_query(h: &Harness, bare_name: &str, aql: &str) {
    h.goto("/queries/aql").await;
    // The Save dispatch is hydrated behaviour; a click landing before
    // hydration is silently lost (#2285's class).
    h.wait_hydrated().await;
    let mut clicked = false;
    for _ in 0..5 {
        let editor = h.wait_css("#aql-editor").await;
        let namespace = h.wait_css("#aql-save-namespace").await;
        let name = h.wait_css("#aql-save-name").await;
        let version = h.wait_css("#aql-save-version").await;
        // Load-bearing on a RETRY: without it a second typing pass appends to
        // what the first left behind.
        for field in [&editor, &namespace, &name, &version] {
            field.clear().await;
        }
        editor.send_keys(aql).await;
        namespace.send_keys(QUERY_NAMESPACE).await;
        name.send_keys(bare_name).await;
        version.send_keys("1.0.0").await;
        let save = h.wait_xpath("//button[normalize-space(.)='Save']").await;
        if poll_until_for(Duration::from_secs(1), async || save.enabled().await).await {
            save.click().await;
            clicked = true;
            break;
        }
    }
    assert!(
        clicked,
        "the Save button never enabled for `{bare_name}` — the screen never hydrated"
    );
    // The outcome toast (success or failure) proves the request completed.
    h.wait_xpath("//*[contains(normalize-space(.), 'Query saved')]")
        .await;
}

/// Delete a stored-query version through the admin confirmation dialog and
/// assert its row is gone.
///
/// # Panics
/// On any interaction failure, or when the row survives.
async fn delete_stored_query(h: &Harness, qualified_name: &str) {
    h.goto("/queries").await;
    let key = format!("[data-query-delete=\"{qualified_name}@1.0.0\"]");
    h.wait_css(&key).await;
    confirm_in_dialog(h, &key, "stored-query-delete-confirm").await;
    wait_css_absent(h, &key).await;
}

/// The reverse-lift round trip: a stored query whose AQL is in the builder's
/// envelope opens IN the builder and previews byte-identical AQL (so saving it
/// unchanged would rewrite the same query), while one outside the envelope opens
/// with an explicit refusal and an empty builder.
#[tokio::test]
async fn stored_query_lifts_back_into_the_builder() {
    let Some(h) = Harness::start("stored-query-reverse-lift").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    // ── the representable half ───────────────────────────────────────────────
    store_query(&h, LIFTABLE_BARE_NAME, LIFTABLE_AQL).await;
    h.goto("/queries").await;
    let builder_link = format!("[data-open-in-builder=\"{LIFTABLE_QUERY_NAME}@1.0.0\"]");
    h.wait_css(&builder_link).await.click().await;
    h.wait_url_contains("/queries/builder").await;

    // The builder's live preview re-lowers whatever state the lift produced, so
    // matching the stored text IS the round-trip assertion.
    let mut previewed = String::new();
    poll_until(async || {
        previewed = h
            .wait_css("pre")
            .await
            .text()
            .await
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        previewed == normalized(LIFTABLE_AQL)
    })
    .await;
    assert_eq!(
        previewed,
        normalized(LIFTABLE_AQL),
        "the lifted builder state must re-lower to the stored query verbatim"
    );
    assert!(
        count_matching(&h, "[data-lift-refused]").await == 0,
        "a query inside the builder's envelope must not report a refusal"
    );
    // The lift also seeded the save fields: the loaded version is immutable, so
    // the NEXT one is proposed (as on the raw editor).
    let proposed = h.wait_css("#qb-save-version").await.prop("value").await;
    assert_eq!(
        proposed, "1.1.0",
        "lifting version 1.0.0 must propose 1.1.0 in the save version field"
    );
    h.shot(1, "stored-query-lifted-into-builder").await;

    // ── the refused half ─────────────────────────────────────────────────────
    store_query(&h, REFUSED_BARE_NAME, REFUSED_AQL).await;
    h.goto("/queries").await;
    let refused_link = format!("[data-open-in-builder=\"{REFUSED_QUERY_NAME}@1.0.0\"]");
    h.wait_css(&refused_link).await.click().await;
    h.wait_url_contains("/queries/builder").await;
    // An explicit, actionable notice — never a silently partial builder.
    h.wait_css("[data-lift-refused]").await;
    h.wait_xpath("//*[contains(normalize-space(.), 'Open it in the raw AQL editor instead')]")
        .await;
    h.shot(2, "stored-query-lift-refused").await;

    // Self-cleaning.
    delete_stored_query(&h, LIFTABLE_QUERY_NAME).await;
    delete_stored_query(&h, REFUSED_QUERY_NAME).await;

    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The parameterised runner: a stored query with a `$parameter` is executed
/// server-side with its value bound, through each of the three openEHR
/// version-resolution forms (exact / prefix / latest), and its `RESULT_SET` lands
/// in the shared results pane.
#[tokio::test]
async fn stored_query_runs_with_parameters_in_every_resolution_form() {
    let Some(h) = Harness::start("stored-query-parameterised-run").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    store_query(&h, RUNNER_BARE_NAME, RUNNER_AQL).await;

    // Reach the runner the way an operator does: the row's Run action.
    h.goto("/queries").await;
    let run_link = format!("[data-run-stored=\"{RUNNER_QUERY_NAME}@1.0.0\"]");
    h.wait_css(&run_link).await.click().await;
    h.wait_url_contains("/queries/stored").await;

    // The link's version seeds the EXACT form, and the query's one placeholder
    // is prompted by name. `prop:value` lands at hydration, so poll like the
    // versioning journey's proposed-version read — asserting immediately races
    // the WASM load on a full-page navigation.
    let version_field = h.wait_css("#stored-run-version").await;
    let mut seeded = String::new();
    poll_until(async || {
        seeded = version_field.prop("value").await;
        seeded == "1.0.0"
    })
    .await;
    assert_eq!(
        seeded, "1.0.0",
        "the runner must open at the version the link named"
    );
    h.wait_css("[data-stored-param=\"template\"]").await;
    h.shot(1, "stored-query-runner").await;

    // Exact: `POST query/{name}/1.0.0`.
    select_mode(&h, "exact", Some("1.0.0"), "exactly that version").await;
    assert_request(&h, &format!("POST query/{RUNNER_QUERY_NAME}/1.0.0")).await;
    run_and_expect_results(&h, "exact").await;
    h.shot(2, "stored-query-run-exact").await;

    // Prefix: a partial pattern. The viewer's job is to compose
    // `POST query/{name}/1` and to SAY that the CDR resolves it to the latest
    // matching version, which is what this step asserts; RESOLVING the prefix is
    // the server's behaviour and belongs to the CDR's own conformance run, not to
    // a viewer journey.
    fresh_runner(&h).await;
    select_mode(&h, "prefix", Some("1"), "latest version matching it").await;
    assert_request(&h, &format!("POST query/{RUNNER_QUERY_NAME}/1")).await;
    h.shot(3, "stored-query-run-prefix").await;

    // Latest: no version segment at all.
    fresh_runner(&h).await;
    select_mode(&h, "latest", None, "the latest one").await;
    assert_request(&h, &format!("POST query/{RUNNER_QUERY_NAME}")).await;
    run_and_expect_results(&h, "latest").await;
    h.shot(4, "stored-query-run-latest").await;

    delete_stored_query(&h, RUNNER_QUERY_NAME).await;

    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Land on a fresh runner page, so the results pane starts empty and its later
/// appearance is an unambiguous "this run answered" signal.
async fn fresh_runner(h: &Harness) {
    h.goto(&format!("/queries/stored?load={RUNNER_QUERY_NAME}@1.0.0"))
        .await;
    h.wait_css("#stored-run-mode").await;
}

/// Select a version-resolution form (and type its version, when the form needs
/// one), asserting the resolution note then states the request that choice will
/// send — which is what makes the three forms *labelled* rather than merely
/// selectable.
///
/// Clicking the `<option>` both selects it and fires `change`. Bounded retry: a
/// click landing before hydration mutates the DOM select without firing any
/// listener, so the note would never change (the builder template-picker
/// precedent).
///
/// # Panics
/// On any interaction failure, or when the note never states `note_fragment`.
async fn select_mode(h: &Harness, mode: &str, version: Option<&str>, note_fragment: &str) {
    let option_css = format!("#stored-run-mode option[value='{mode}']");
    let mut selected = false;
    for _ in 0..5 {
        h.wait_css(&option_css).await.click().await;
        if let Some(version) = version {
            let field = h.wait_css("#stored-run-version").await;
            field.clear().await;
            field.send_keys(version).await;
        }
        // NOTE: probe tier — the note re-renders as the resolution form
        // changes, so a handle detached between the find and the read is
        // "not yet" for this poll rather than a failure.
        selected = poll_until_for(Duration::from_secs(2), async || {
            h.wait_css("[data-resolution-note]")
                .await
                .read_text()
                .await
                .is_some_and(|note| note.contains(note_fragment))
        })
        .await;
        if selected {
            break;
        }
    }
    assert!(
        selected,
        "the `{mode}` resolution form never took, or its note never said `{note_fragment}`"
    );
}

/// Assert the resolution note names exactly `request` as the request the current
/// choice will send. The note keeps the path in its own `<span>`, so the match is
/// EXACT and a prefix form can never pass for the exact one it is a prefix of.
///
/// # Panics
/// When no element states exactly that request.
async fn assert_request(h: &Harness, request: &str) {
    h.wait_xpath(&format!("//span[normalize-space(.)='{request}']"))
        .await;
}

/// Bind the query's `$template` parameter, run, and assert the shared results
/// pane rendered instead of an inline error — i.e. the CDR answered a `RESULT_SET`
/// for the parameterised stored execution.
///
/// # Panics
/// On any interaction failure, or when the run reports an error.
async fn run_and_expect_results(h: &Harness, mode: &str) {
    let parameter = h.wait_css("[data-stored-param=\"template\"]").await;
    parameter.clear().await;
    parameter.send_keys(SEEDED_TEMPLATE_ID).await;
    h.wait_css("#stored-run").await.click().await;
    h.wait_css("[data-stored-results]").await;
    assert!(
        count_matching(h, "[role=\"alert\"]").await == 0,
        "the `{mode}` run must return a result set, not an error"
    );
}

/// Collapse runs of whitespace so a multi-line source constant compares against
/// the single-line AQL the preview renders.
fn normalized(aql: &str) -> String {
    aql.split_whitespace().collect::<Vec<_>>().join(" ")
}
