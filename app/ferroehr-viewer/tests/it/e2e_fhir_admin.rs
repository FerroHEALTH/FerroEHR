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
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]
//! End-to-end journeys over the viewer's **FHIR connector** screen (`/fhir`):
//! the mapping-store round trip, the validate-only dry run in all three of its
//! verdicts, and the read-path viewer.
//!
//! The screen is probe-gated on the CDR serving its FHIR connector, which the
//! composed E2E stack enables (`docker/viewer/e2e-env.yml` sets
//! `FERROEHR__FHIR__API_ENABLED`); a stack without it hides the nav entry
//! entirely, and these journeys say so in their failure message rather than
//! passing vacuously. The hidden-when-absent half is unit-tested in
//! `ferroehr_viewer::fhir` against a probe that found no surface — the same
//! split as the admin-group and management journeys.
//!
//! **Nothing here commits a FHIR resource through the connector, because the
//! viewer offers no path that could.** The read scene needs committed content
//! to read, so it seeds an EHR and a COMPOSITION over the openEHR REST API
//! directly — out of band, the way this harness seeds everything else — and the
//! connector only ever READS it back.
//!
//! The mapping store is Admin-classed by the CDR's coarse RBAC (it is mounted
//! under `/admin`), so every scene that reads or writes it signs in as the ADMIN
//! dev user; the last scene signs in as the ordinary one to pin the refusal
//! copy.
//!
//! Isolation: each scene owns its own fixture names and removes exactly those
//! before AND after itself, so scenes running concurrently against one stack
//! never clobber each other.

use crate::common;

use reqwest::StatusCode;

use common::{
    Harness, confirm_in_dialog, count_matching, env, is_present, login_basic, login_basic_as,
    retype, wait_css_absent, wait_enabled, wait_text, wait_text_contains,
};

/// The template every fixture mapping builds under. `scripts/ui-e2e.sh` already
/// uploads it while seeding the stack; [`seed_template`] re-sends it so a
/// hand-composed stack works too.
const TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The mapping the CRUD round trip owns.
const ROUND_TRIP_MAPPING: &str = "e2e-viewer-fhir";

/// The valid mapping the dry-run scene owns.
const DRY_RUN_MAPPING: &str = "e2e-viewer-fhir-dry";

/// The mapping the dry-run scene owns whose context sets an invalid territory,
/// so the CDR's own validator refuses the COMPOSITION it builds.
const DRY_RUN_BAD_MAPPING: &str = "e2e-viewer-fhir-dry-bad";

/// The mapping the read scene owns.
const READ_MAPPING: &str = "e2e-viewer-fhir-read";

/// The `meta.profile` each fixture mapping matches on: the connector resolves a
/// mapping by resource type + profile, so a per-scene profile keeps concurrent
/// scenes from resolving each other's mappings.
fn profile_of(mapping: &str) -> String {
    format!("http://example.org/StructureDefinition/{mapping}")
}

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`): the mapping store
/// sits under `/admin`, so the RBAC gate classes every fixture call as admin
/// work.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The mapping-store base URL on the CDR under test.
fn store_url(cdr: &str) -> String {
    format!("{cdr}/ferroehr/rest/openehr/v1/admin/fhir_mapping")
}

/// The ITS-REST v1 base URL on the CDR under test.
fn rest_v1(cdr: &str) -> String {
    format!("{cdr}/ferroehr/rest/openehr/v1")
}

/// One mapping definition document, as the viewer's textarea carries it.
fn definition(mapping: &str, territory: &str) -> serde_json::Value {
    serde_json::json!({
        "resource_type": "Observation",
        "profile_url": profile_of(mapping),
        "template_id": TEMPLATE_ID,
        "subject": {
            "reference_path": "subject.reference",
            "namespace": "fhir",
            "strip_prefix": "Patient/"
        },
        "context": {
            "ctx/language": "en",
            "ctx/territory": territory,
            "ctx/composer_name": "fhir-connector",
            "ctx/time": "2026-02-03T04:05:06Z"
        },
        "entries": [
            {
                "openehr_path": "minimal/minimal:0/quantity",
                "fhir_path": "valueQuantity.value",
                "transform": { "kind": "quantity", "unit_path": "valueQuantity.unit" }
            }
        ]
    })
}

/// A FHIR Observation carrying the profile of `mapping`, so the connector
/// resolves exactly that mapping for it.
fn observation(mapping: &str, patient: &str, magnitude: f64) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Observation",
        "id": "e2e-viewer-obs",
        "meta": { "versionId": "1", "profile": [profile_of(mapping)] },
        "status": "final",
        "subject": { "reference": format!("Patient/{patient}") },
        "valueQuantity": { "value": magnitude, "unit": "kg" }
    })
}

/// Upload the fixture template (idempotent for a journey: `201` created, `409`
/// already there) — a mapping's `template_id` is a foreign key, so the store
/// refuses `400` without it.
///
/// # Panics
/// On any other answer.
async fn seed_template(cdr: &str) {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/minimal_evaluation.opt"
    ))
    .expect("the shared OPT fixture exists");
    let (user, pass) = admin_credentials();
    let status = reqwest::Client::new()
        .post(format!("{}/definition/template/adl1.4", rest_v1(cdr)))
        .basic_auth(&user, Some(&pass))
        .header("Content-Type", "application/xml")
        .body(source)
        .send()
        .await
        .expect("seed the fixture template")
        .status();
    assert!(
        status == StatusCode::CREATED || status == StatusCode::CONFLICT,
        "template seed -> {status}"
    );
}

/// Remove exactly the named fixture mappings from the store (absent = nothing
/// to do), so a scene both starts and ends from a known state.
///
/// Scene-scoped on purpose: journeys run concurrently against one stack, so a
/// blanket "delete every fixture" sweep would remove another scene's mapping
/// mid-run.
///
/// # Panics
/// On any answer other than `200` to the listing, or `204`/`404` to a delete.
async fn remove_mappings(cdr: &str, names: &[&str]) {
    let http = reqwest::Client::new();
    let (user, pass) = admin_credentials();
    let response = http
        .get(store_url(cdr))
        .basic_auth(&user, Some(&pass))
        .send()
        .await
        .expect("list the FHIR mapping store");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the FHIR mapping store must be served for these journeys to mean anything"
    );
    let rows = response
        .json::<serde_json::Value>()
        .await
        .expect("the store answers a JSON array");
    let ids: Vec<String> = rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.get("name")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|name| names.contains(&name))
                })
                .filter_map(|row| row.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        let status = http
            .delete(format!("{}/{id}", store_url(cdr)))
            .basic_auth(&user, Some(&pass))
            .send()
            .await
            .expect("delete a fixture mapping")
            .status();
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND,
            "mapping cleanup -> {status}"
        );
    }
}

/// Store one fixture mapping over the store API (test setup deliberately
/// bypasses the UI, whose create path has its own scene).
///
/// # Panics
/// When the CDR refuses it.
async fn seed_mapping(cdr: &str, name: &str, territory: &str) {
    let (user, pass) = admin_credentials();
    let response = reqwest::Client::new()
        .post(store_url(cdr))
        .basic_auth(&user, Some(&pass))
        .json(&serde_json::json!({
            "name": name,
            "enabled": true,
            "definition": definition(name, territory),
        }))
        .send()
        .await
        .expect("seed a fixture mapping");
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "mapping seed -> {status}: {body}"
    );
}

/// Commit one COMPOSITION for `patient` over the **openEHR** REST API, so the
/// read facade has something to reverse-map.
///
/// Out of band on purpose: the connector's own ingest door commits, and the
/// viewer offers no path to it — the read scene needs data, not a viewer
/// write.
///
/// # Panics
/// When the CDR refuses the EHR or the composition.
async fn seed_committed_observation(cdr: &str, patient: &str, magnitude: f64) {
    let http = reqwest::Client::new();
    let (user, pass) = admin_credentials();
    let ehr = http
        .post(format!("{}/ehr", rest_v1(cdr)))
        .basic_auth(&user, Some(&pass))
        .header("Prefer", "return=representation")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "_type": "EHR_STATUS",
            "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
            "name": { "_type": "DV_TEXT", "value": "EHR Status" },
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                "rm_version": "1.1.0"
            },
            // The mapping's subject binding strips `Patient/` and reads the
            // remainder in the `fhir` namespace, so the EHR subject must match
            // exactly that for the facade to scope to it.
            "subject": {
                "_type": "PARTY_SELF",
                "external_ref": {
                    "_type": "PARTY_REF",
                    "namespace": "fhir",
                    "type": "PERSON",
                    "id": { "_type": "GENERIC_ID", "value": patient, "scheme": "fhir" }
                }
            },
            "is_modifiable": true,
            "is_queryable": true
        }))
        .send()
        .await
        .expect("create the fixture EHR");
    assert_eq!(ehr.status(), StatusCode::CREATED, "fixture EHR");
    let ehr_id = ehr
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| {
            body.get("ehr_id")
                .and_then(|id| id.get("value"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .expect("the created EHR reports its id");

    let status = http
        .post(format!("{}/ehr/{ehr_id}/composition", rest_v1(cdr)))
        .basic_auth(&user, Some(&pass))
        .header("Content-Type", "application/openehr.wt.flat+json")
        .header("Accept", "application/json")
        .header("openehr-template-id", TEMPLATE_ID)
        .header("Prefer", "return=minimal")
        .body(
            serde_json::json!({
                "ctx/language": "en",
                "ctx/territory": "US",
                "ctx/composer_name": "e2e viewer seed",
                "ctx/time": "2026-07-14T08:00:00Z",
                "minimal/minimal/quantity|magnitude": magnitude,
                "minimal/minimal/quantity|unit": "kg"
            })
            .to_string(),
        )
        .send()
        .await
        .expect("commit the fixture composition")
        .status();
    assert_eq!(status, StatusCode::CREATED, "fixture composition");
}

/// Land on `/fhir` with the connector actually served, or fail naming the
/// switch that turns it on.
///
/// # Panics
/// When the CDR under test runs with the FHIR connector disabled — the screen
/// then renders its disabled card and every assertion below would be vacuous.
async fn open_connector(h: &Harness) {
    // The nav entry is the probe's own verdict: it renders only when the CDR
    // answered the mapping-store probe with something other than a 404.
    h.wait_css("a[href='/fhir']").await;
    h.goto("/fhir").await;
    h.wait_css("#fhir-screen").await;
    assert!(
        !is_present(h, "#fhir-disabled").await,
        "the CDR under test runs with the FHIR connector disabled — set \
         FERROEHR__FHIR__API_ENABLED=true on the composed `ferroehr` service"
    );
}

/// The CSS selector of one store row's cell.
fn row_cell(name: &str, cell: &str) -> String {
    format!("tr[data-fhir-mapping='{name}'] [data-fhir-cell='{cell}']")
}

/// Fill the create card and send it. The submit is inert until the draft can
/// actually be sent, so `wait_enabled` IS the "the form is complete" condition —
/// never a sleep.
///
/// # Panics
/// On any interaction failure.
async fn store_mapping(h: &Harness, name: &str, territory: &str) {
    retype(h, "#fhir-create-name", name).await;
    retype(
        h,
        "#fhir-create-definition",
        &definition(name, territory).to_string(),
    )
    .await;
    wait_enabled(h, "#fhir-create-submit").await;
    h.wait_css("#fhir-create-submit").await.click().await;
}

/// Run the dry-run panel over one resource body and wait for its verdict.
///
/// # Panics
/// On any interaction failure.
async fn dry_run(h: &Harness, resource: &str) {
    retype(h, "#fhir-dry-run-type", "Observation").await;
    retype(h, "#fhir-dry-run-resource", resource).await;
    wait_enabled(h, "#fhir-dry-run-submit").await;
    h.wait_css("#fhir-dry-run-submit").await.click().await;
}

/// The mapping-store round trip: store a mapping through the JSON document
/// editor, see its row, edit the document, then delete it through the
/// confirmation dialog and watch the row go.
#[tokio::test]
async fn the_mapping_store_creates_edits_and_deletes_a_mapping() {
    let Some(h) = Harness::start("fhir-store").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP fhir-store: fixture cleanup needs UI_E2E_CDR_URL");
        h.finish().await;
        return;
    };
    seed_template(&cdr).await;
    remove_mappings(&cdr, &[ROUND_TRIP_MAPPING]).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_connector(&h).await;

    store_mapping(&h, ROUND_TRIP_MAPPING, "US").await;
    // The row is the CDR's answer, not the form's: the listing refetches on the
    // action's version and renders what the store now holds, with the resource
    // type and template PROJECTED out of the definition document by the CDR.
    wait_text_contains(
        &h,
        &row_cell(ROUND_TRIP_MAPPING, "resource-type"),
        "Observation",
    )
    .await;
    wait_text_contains(&h, &row_cell(ROUND_TRIP_MAPPING, "template"), TEMPLATE_ID).await;
    wait_text_contains(&h, &row_cell(ROUND_TRIP_MAPPING, "enabled"), "enabled").await;
    assert!(
        wait_text(&h, "Mapping stored").await,
        "a successful store must toast"
    );
    h.shot(1, "fhir-mapping-stored").await;

    // Edit: the row's own document seeds the editor, and saving a changed one
    // is reflected in the projected columns.
    h.wait_toasts_cleared().await;
    h.wait_css(&format!("[data-fhir-edit='{ROUND_TRIP_MAPPING}']"))
        .await
        .click()
        .await;
    h.wait_css("#fhir-edit").await;
    let mut edited = definition(ROUND_TRIP_MAPPING, "US");
    edited["template_id"] = serde_json::Value::from(TEMPLATE_ID);
    edited["profile_url"] =
        serde_json::Value::from("http://example.org/StructureDefinition/edited");
    retype(&h, "#fhir-edit-definition", &edited.to_string()).await;
    wait_enabled(&h, "#fhir-edit-save").await;
    h.wait_css("#fhir-edit-save").await.click().await;
    wait_text_contains(
        &h,
        &row_cell(ROUND_TRIP_MAPPING, "profile"),
        "StructureDefinition/edited",
    )
    .await;
    // The editor closes on success, so the screen is back to its resting state.
    wait_css_absent(&h, "#fhir-edit").await;
    h.shot(2, "fhir-mapping-edited").await;

    // Delete: two steps — the row's button opens the shared confirmation
    // dialog, and only the dialog's own button dispatches.
    confirm_in_dialog(
        &h,
        &format!("[data-fhir-delete='{ROUND_TRIP_MAPPING}']"),
        "fhir-delete-confirm",
    )
    .await;
    wait_css_absent(&h, &format!("tr[data-fhir-mapping='{ROUND_TRIP_MAPPING}']")).await;
    assert!(
        wait_text(&h, "Mapping deleted").await,
        "a successful delete must toast"
    );
    h.shot(3, "fhir-mapping-deleted").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_mappings(&cdr, &[ROUND_TRIP_MAPPING]).await;
    h.finish().await;
}

/// A malformed document is refused by the CDR, and its diagnostic reaches the
/// reader VERBATIM — inline beside the failure toast.
#[tokio::test]
async fn a_rejected_mapping_document_surfaces_the_diagnostic_verbatim() {
    let Some(h) = Harness::start("fhir-store-rejected").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_connector(&h).await;

    // A document the viewer can send (valid JSON, an object) but the CDR
    // refuses: no template_id at all.
    retype(&h, "#fhir-create-name", "e2e-viewer-fhir-rejected").await;
    retype(
        &h,
        "#fhir-create-definition",
        r#"{"resource_type":"Observation"}"#,
    )
    .await;
    wait_enabled(&h, "#fhir-create-submit").await;
    h.wait_css("#fhir-create-submit").await.click().await;

    // The CDR's own words, unedited — the viewer never paraphrases a
    // diagnostic it did not author.
    wait_text_contains(
        &h,
        "#fhir-create-diagnostic",
        "invalid FHIR mapping definition",
    )
    .await;
    wait_text_contains(&h, "#fhir-create-diagnostic", "template_id").await;
    // The refusal ALSO toasts: an inline-only failure reads as "nothing
    // happened" (the viewer's mutation-feedback rule).
    assert!(
        wait_text(&h, "Mapping not stored").await,
        "a refused store must toast as well as render the diagnostic inline"
    );
    h.shot(1, "fhir-mapping-rejected").await;

    // Nothing was stored: no row carries the refused name.
    wait_css_absent(&h, "tr[data-fhir-mapping='e2e-viewer-fhir-rejected']").await;

    // The 400 the CDR answered the server fn with is the point of this journey.
    h.assert_console_clean(&["400", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The dry run in all three of its verdicts: a resource the CDR would accept, a
/// resource whose mapped COMPOSITION it would refuse (still a completed
/// validation), and one the operation could not validate at all. Nothing is
/// committed by any of them.
#[tokio::test]
async fn the_dry_run_reports_every_verdict_and_commits_nothing() {
    let Some(h) = Harness::start("fhir-dry-run").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP fhir-dry-run: fixture seeding needs UI_E2E_CDR_URL");
        h.finish().await;
        return;
    };
    seed_template(&cdr).await;
    remove_mappings(&cdr, &[DRY_RUN_MAPPING, DRY_RUN_BAD_MAPPING]).await;
    seed_mapping(&cdr, DRY_RUN_MAPPING, "US").await;
    // The same template with an invalid ISO 3166-1 territory: the mapping is
    // storable, and the COMPOSITION it builds is what the validator refuses.
    seed_mapping(&cdr, DRY_RUN_BAD_MAPPING, "ZZ").await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_connector(&h).await;

    // 1. A completed validation that passed.
    dry_run(
        &h,
        &observation(DRY_RUN_MAPPING, "e2e-viewer-fhir-p1", 118.0).to_string(),
    )
    .await;
    h.wait_css("#fhir-dry-run-verdict[data-fhir-verdict='valid']")
        .await;
    wait_text_contains(&h, "#fhir-dry-run-outcome", "nothing was committed").await;
    wait_text_contains(&h, "#fhir-dry-run-outcome", TEMPLATE_ID).await;
    h.shot(1, "fhir-dry-run-valid").await;

    // 2. A completed validation that refused — still HTTP 200, the verdict
    //    rides the outcome, and the openEHR validator's message is verbatim.
    dry_run(
        &h,
        &observation(DRY_RUN_BAD_MAPPING, "e2e-viewer-fhir-p1", 118.0).to_string(),
    )
    .await;
    h.wait_css("#fhir-dry-run-verdict[data-fhir-verdict='invalid']")
        .await;
    wait_text_contains(&h, "#fhir-dry-run-outcome", "ZZ").await;
    h.shot(2, "fhir-dry-run-invalid").await;

    // 3. A resource the operation could not validate at all: the CDR never
    //    reached a verdict, and the screen says so instead of claiming one.
    let mut garbage = observation(DRY_RUN_MAPPING, "e2e-viewer-fhir-p1", 118.0);
    garbage["valueQuantity"]["value"] = serde_json::Value::from("not-a-number");
    dry_run(&h, &garbage.to_string()).await;
    h.wait_css("#fhir-dry-run-verdict[data-fhir-verdict='not-run']")
        .await;
    wait_text_contains(&h, "#fhir-dry-run-outcome", "is not a number").await;
    h.shot(3, "fhir-dry-run-not-run").await;

    // The dry run commits nothing, so the subject it names has no EHR: the read
    // facade still answers an empty searchset for it.
    let facade = reqwest::Client::new()
        .get(format!(
            "{}/fhir/r4/Observation?patient=e2e-viewer-fhir-p1",
            rest_v1(&cdr)
        ))
        .basic_auth(&user, Some(&pass))
        .send()
        .await
        .expect("read the facade after the dry runs")
        .json::<serde_json::Value>()
        .await
        .expect("a FHIR Bundle");
    assert_eq!(
        facade.get("total").and_then(serde_json::Value::as_i64),
        Some(0),
        "three dry runs committed nothing: {facade}"
    );

    // The 400 the third dry run was answered with is the point of scene 3.
    h.assert_console_clean(&["400", "Failed to load resource"])
        .await;
    remove_mappings(&cdr, &[DRY_RUN_MAPPING, DRY_RUN_BAD_MAPPING]).await;
    h.finish().await;
}

/// The read-path viewer: an empty answer for a patient with nothing committed,
/// the reverse-mapped resource for one with data, and an `OperationOutcome`
/// rendered verbatim when the connector refuses the read.
#[tokio::test]
async fn the_read_path_viewer_answers_for_a_patient() {
    let Some(h) = Harness::start("fhir-read").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP fhir-read: fixture seeding needs UI_E2E_CDR_URL");
        h.finish().await;
        return;
    };
    seed_template(&cdr).await;
    remove_mappings(&cdr, &[READ_MAPPING]).await;
    seed_mapping(&cdr, READ_MAPPING, "US").await;
    // Committed out of band over the openEHR API: the viewer has no committing
    // path, and the facade needs stored content to reverse-map.
    let patient = "e2e-viewer-fhir-read-p1";
    seed_committed_observation(&cdr, patient, 118.0).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_connector(&h).await;

    // A patient with nothing committed: an empty searchset Bundle, which is an
    // answer rather than an error.
    retype(&h, "#fhir-read-type", "Observation").await;
    retype(&h, "#fhir-read-patient", "e2e-viewer-fhir-nobody").await;
    h.wait_css("#fhir-read-submit").await.click().await;
    // The scope lands in the URL, so the read is shareable and refresh-safe.
    h.wait_url_contains("resource_type=Observation").await;
    wait_text_contains(&h, "#fhir-read-result", "\"total\": 0").await;
    h.shot(1, "fhir-read-empty").await;

    // The seeded patient: the committed COMPOSITION comes back reverse-mapped
    // through the stored mapping.
    retype(&h, "#fhir-read-patient", patient).await;
    h.wait_css("#fhir-read-submit").await.click().await;
    wait_text_contains(&h, "#fhir-read-result", &format!("Patient/{patient}")).await;
    wait_text_contains(&h, "#fhir-read-result", "valueQuantity").await;
    h.shot(2, "fhir-read-bundle").await;

    // A refused read: the connector carries no mapping machinery for this type,
    // and its OperationOutcome is rendered verbatim, inline, never as a toast.
    retype(&h, "#fhir-read-type", "MedicationRequest").await;
    h.wait_css("#fhir-read-submit").await.click().await;
    wait_text_contains(
        &h,
        "#fhir-read-outcome",
        "is not supported by the connector",
    )
    .await;
    assert!(
        count_matching(&h, ".thaw-toast-body").await == 0,
        "a failed read reports inline only — a toast would be the mutation rule leaking"
    );
    h.shot(3, "fhir-read-outcome").await;

    // The 501 the facade answered the server fn with is the point of scene 3.
    h.assert_console_clean(&["501", "Failed to load resource"])
        .await;
    remove_mappings(&cdr, &[READ_MAPPING]).await;
    h.finish().await;
}

/// A session without the ADMIN role still SEES the connector entry — capability
/// is not authorization — and the refusal reaches it as actionable copy on the
/// screen that asked, never as a missing screen or a bare "forbidden".
#[tokio::test]
async fn a_session_without_the_admin_role_reads_the_refusal_on_the_screen() {
    let Some(h) = Harness::start("fhir-refused").await else {
        return;
    };
    // The ordinary dev user (USER role): the CDR mounts the group, so the probe
    // says present and the nav entry renders, but the mapping store is answered
    // 403.
    login_basic(&h).await;
    open_connector(&h).await;

    wait_text_contains(&h, "#fhir-refused", "may not administer").await;
    wait_text_contains(&h, "#fhir-refused", "ADMIN-role").await;
    // The CDR's own diagnostic travels with it, unedited.
    wait_text_contains(&h, "#fhir-refused", "ADMIN").await;
    h.shot(1, "fhir-refused").await;

    // A refused READ never toasts (the viewer's one feedback rule).
    assert!(
        count_matching(&h, ".thaw-toast-body").await == 0,
        "a refused read reports inline only — a toast would be the mutation rule leaking"
    );

    // The 403 the CDR answered the server fn with is the point of this journey.
    h.assert_console_clean(&["403", "Failed to load resource"])
        .await;
    h.finish().await;
}
