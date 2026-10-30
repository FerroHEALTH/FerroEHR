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
    reason = "test fixtures posted through a REST seam are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]
// A capture pass, not an assertive journey: it drives the viewer and writes
// the screenshots. Gated behind UI_E2E_DOCS_SHOTS so the normal E2E run (and
// plain `cargo nextest`) skips it.
//! Documentation-screenshot capture.
//!
//! Every console route at 1440 and 390 px, light and dark, under
//! `{UI_E2E_SHOTS_DIR}/routes/` for the family screenshot review (#3910), and
//! the book's screenshots under `website/book/src/viewer/img/`: each route at
//! 1440 px light, a few in dark beside it, and the states a reader only
//! reaches by a click (the user menu, a dialog, a run). Run via
//! `scripts/ui-e2e.sh` with `UI_E2E_DOCS_SHOTS` set; skips with a printed
//! reason when the harness environment or the gate flag is absent.

use crate::common;

use reqwest::StatusCode;
use std::path::{Path, PathBuf};

use common::{Harness, PHONE_WIDTH, env, fill, login_basic_as};

/// The ADL 1.4 template every capture uses.
const TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The stem of the ADL2 template the capture seeds.
const ADL2_STEM: &str = "openEHR-EHR-COMPOSITION.cnf_adl2_versioned";

/// The stored query the capture seeds.
const QUERY_NAME: &str = "org.example.vitals::latest-weights";

/// The subscription the capture seeds.
const SUBSCRIPTION: &str = "vitals-feed";

/// The REST root of the CDR.
fn v1(cdr: &str) -> String {
    format!("{cdr}/ferroehr/rest/openehr/v1")
}

/// Send `request` as the capture user and return its status and JSON body.
async fn send(
    request: reqwest::RequestBuilder,
    user: &str,
    pass: &str,
) -> (StatusCode, serde_json::Value) {
    let response = request
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("the CDR answers the capture seed");
    let status = response.status();
    let body = response.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// What the capture seeds beyond what `scripts/ui-e2e.sh` did.
struct Seeded {
    /// The seeded EHR.
    ehr: String,
    /// Its two-version composition.
    composition: String,
    /// One of its contributions, empty when the listing named none.
    contribution: String,
    /// A PERSON, empty when the CDR refused it.
    party: String,
}

/// The JSON of the PERSON the capture seeds.
fn person() -> serde_json::Value {
    serde_json::json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "name": { "_type": "DV_TEXT", "value": "PERSON" },
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v1",
            "name": { "_type": "DV_TEXT", "value": "name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "Details" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0002",
                    "name": { "_type": "DV_TEXT", "value": "Name" },
                    "value": { "_type": "DV_TEXT", "value": "Ada Lovelace" }
                }]
            }
        }]
    })
}

/// The JSON of the directory the capture seeds, referencing `composition`.
fn directory(composition: &str) -> serde_json::Value {
    serde_json::json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.directory.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.directory.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": "root" },
        "folders": [{
            "_type": "FOLDER",
            "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" },
                "rm_version": "1.1.0"
            },
            "name": { "_type": "DV_TEXT", "value": "Episodes" },
            "items": [{
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": composition }
            }]
        }]
    })
}

/// Seed the fixtures the captures show: the OPT and the ADL2 releases, a
/// stored query, a subscription, a directory and a tag on the seeded EHR, and
/// a party. Every seed is idempotent for a capture pass, and its status is
/// printed rather than asserted: a missing fixture shows as an empty screen.
#[expect(
    clippy::too_many_lines,
    reason = "one seed per fixture, in the order the captures read them"
)]
async fn seed(cdr: &str, user: &str, pass: &str, ehr: String, composition: String) -> Seeded {
    let http = reqwest::Client::new();
    let root = v1(cdr);
    let opt = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/minimal_evaluation.opt"
    ))
    .expect("the OPT fixture exists");
    let (status, _) = send(
        http.post(format!("{root}/definition/template/adl1.4"))
            .header("Content-Type", "application/xml")
            .body(opt),
        user,
        pass,
    )
    .await;
    println!("opt seed -> {status}");
    for release in ["versioned.v1_0_0.adls", "versioned.v1_1_0.adls"] {
        let source = std::fs::read_to_string(format!(
            "{}/../../corpus/fixtures/adl2/opt/{release}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("the ADL2 corpus fixture exists");
        let (status, _) = send(
            http.post(format!("{root}/definition/template/adl2"))
                .header("Content-Type", "text/plain")
                .body(source),
            user,
            pass,
        )
        .await;
        println!("adl2 seed {release} -> {status}");
    }
    let aql = format!(
        "SELECT c/context/start_time/value AS observed, \
         c/content[openEHR-EHR-EVALUATION.minimal.v1]/data[at0001]/items[at0002]/value/magnitude AS weight \
         FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/archetype_details/template_id/value = '{TEMPLATE_ID}' AND e/ehr_id/value = $ehr_id"
    );
    let (status, _) = send(
        http.put(format!("{root}/definition/query/{QUERY_NAME}/1.0.0"))
            .header("Content-Type", "text/plain")
            .body(aql),
        user,
        pass,
    )
    .await;
    println!("stored query seed -> {status}");
    let (status, _) = send(
        http.post(format!(
            "{cdr}/ferroehr/rest/extensions/v1/admin/event_subscription"
        ))
        .json(&serde_json::json!({
            "name": SUBSCRIPTION,
            "kind": "COMPOSITION",
            "change_type": "249",
            "template_id": TEMPLATE_ID,
            "enabled": true
        })),
        user,
        pass,
    )
    .await;
    println!("subscription seed -> {status}");
    let (status, _) = send(
        http.post(format!("{root}/ehr/{ehr}/directory"))
            .json(&directory(&composition)),
        user,
        pass,
    )
    .await;
    println!("directory seed -> {status}");
    let (status, _) = send(
        http.put(format!("{root}/ehr/{ehr}/composition/{composition}/tags"))
            .json(&serde_json::json!([{ "key": "reviewed", "value": "true" }])),
        user,
        pass,
    )
    .await;
    println!("composition tag seed -> {status}");
    let (_, listed) = send(
        http.get(format!(
            "{cdr}/ferroehr/rest/extensions/v1/ehr/{ehr}/contribution?offset=0&fetch=1"
        )),
        user,
        pass,
    )
    .await;
    let contribution = ["/contributions/0/uid/value", "/items/0/uid/value"]
        .iter()
        .find_map(|pointer| listed.pointer(pointer))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (status, created) = send(
        http.post(format!("{root}/demographic/person"))
            .header("Prefer", "return=representation")
            .json(&person()),
        user,
        pass,
    )
    .await;
    println!("party seed -> {status}");
    let party = created
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .and_then(|uid| uid.split("::").next())
        .unwrap_or_default()
        .to_owned();
    Seeded {
        ehr,
        composition,
        contribution,
        party,
    }
}

/// The website book's screenshot directory, resolved from this crate's
/// manifest dir (`app/ferroehr-viewer`).
fn book_img_dir() -> PathBuf {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .join("website/book/src/viewer/img");
    std::fs::create_dir_all(&dir).expect("create the book screenshot dir");
    dir
}

/// Write the current window to `{dir}/{slug}.png`.
async fn shot_to(h: &Harness, dir: &Path, slug: &str) {
    let out = dir.join(format!("{slug}.png"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create the screenshot section dir");
    }
    h.screenshot_to(&out).await;
}

/// One route the pass captures.
struct Route {
    /// The address.
    path: String,
    /// The image's path under a screenshot root, without `.png`.
    slug: &'static str,
    /// Whether the book shows it at all.
    in_book: bool,
    /// Whether the book shows it in dark beside light.
    dark_in_book: bool,
}

/// A route the book shows in light only.
fn light(path: impl Into<String>, slug: &'static str) -> Route {
    Route {
        path: path.into(),
        slug,
        in_book: true,
        dark_in_book: false,
    }
}

/// A route only the review captures.
fn review(path: impl Into<String>, slug: &'static str) -> Route {
    Route {
        path: path.into(),
        slug,
        in_book: false,
        dark_in_book: false,
    }
}

/// A route the book shows in both themes.
fn both(path: impl Into<String>, slug: &'static str) -> Route {
    Route {
        path: path.into(),
        slug,
        in_book: true,
        dark_in_book: true,
    }
}

/// Every route of the console, over the fixtures `seeded` names.
#[expect(
    clippy::too_many_lines,
    reason = "one list: every route the console serves, each on its own line"
)]
fn routes(seeded: &Seeded) -> Vec<Route> {
    let ehr = format!("/ehrs/{}", seeded.ehr);
    let composition = format!("{ehr}/compositions/{}", seeded.composition);
    let template = format!("/templates/{TEMPLATE_ID}");
    let query = format!("/queries/{}/1.0.0", urlencoding::encode(QUERY_NAME));
    let mut all = vec![
        both("/", "dashboard/dashboard"),
        both("/ehrs", "ehrs/ehrs"),
        review("/ehrs/new", "ehrs/new"),
        both(ehr.clone(), "ehrs/ehr"),
        review(format!("{ehr}/status"), "ehrs/status"),
        review(format!("{ehr}/status/edit"), "ehrs/status-edit"),
        light(format!("{ehr}/directory"), "ehrs/directory"),
        review(format!("{ehr}/directory/edit"), "ehrs/directory-edit"),
        review(format!("{ehr}/contributions"), "ehrs/contributions"),
        review(format!("{ehr}/contribution/new"), "ehrs/contribution-new"),
        review(format!("{ehr}/tags"), "ehrs/tags"),
        review(format!("{ehr}/compositions/new"), "compositions/new"),
        both(composition.clone(), "compositions/reading"),
        review(
            format!("{composition}?tab=document"),
            "compositions/document",
        ),
        review(
            format!("{composition}?tab=versions"),
            "compositions/versions",
        ),
        review(format!("{composition}?tab=tags"), "compositions/tags"),
        review(format!("{composition}/edit"), "compositions/edit"),
        both("/parties", "parties/parties"),
        review("/parties/new", "parties/new"),
        both("/templates", "templates/templates"),
        review("/templates?family=adl2", "templates/templates-adl2"),
        light("/templates/upload", "templates/upload"),
        light(template.clone(), "templates/template"),
        light(format!("{template}?tab=paths"), "templates/template-paths"),
        review(
            format!("{template}?tab=source"),
            "templates/template-source",
        ),
        review(
            format!("{template}?tab=example"),
            "templates/template-example",
        ),
        light(
            format!("/templates/adl2/{ADL2_STEM}"),
            "templates/template-adl2",
        ),
        both("/terminology", "terminology/terminology"),
        review("/terminology/openehr", "terminology/openehr"),
        light(
            "/terminology/lookup?terminology=openehr&code=249",
            "terminology/lookup",
        ),
        light(
            "/terminology/valuesets?terminology=openehr&set=audit_change_type",
            "terminology/valuesets",
        ),
        review(
            "/terminology/subsumption?terminology=openehr&broader=249&narrower=250",
            "terminology/subsumption",
        ),
        light("/queries", "queries/queries"),
        light(query.clone(), "queries/stored-query"),
        light(format!("{query}/run"), "queries/stored-query-run"),
        review(
            format!("/queries/{}/versions", urlencoding::encode(QUERY_NAME)),
            "queries/stored-query-versions",
        ),
        both(format!("/query?template={TEMPLATE_ID}"), "queries/build"),
        light("/query/aql", "queries/aql"),
        both("/server", "server/overview"),
        light("/server/health", "server/health"),
        light(
            "/server/metrics?metric=db_pool_connections",
            "server/metrics",
        ),
        review("/server/logging", "server/logging"),
        review("/server/api", "server/api"),
        review("/server/smart", "server/smart"),
        both("/audit", "audit/audit"),
        review("/audit?patient=no-such-patient", "audit/audit-empty"),
        both("/subscriptions", "subscriptions/subscriptions"),
        light("/subscriptions/new", "subscriptions/new"),
        review(
            format!("/subscriptions/{SUBSCRIPTION}/edit"),
            "subscriptions/edit",
        ),
        review("/configuration", "configuration/configuration"),
        light("/preferences", "preferences/preferences"),
        review("/preferences/reading", "preferences/reading"),
        review("/preferences/keyboard", "preferences/keyboard"),
        light("/access", "access/access"),
        review("/about", "about/about"),
    ];
    if !seeded.contribution.is_empty() {
        all.push(review(
            format!("{ehr}/contributions/{}", seeded.contribution),
            "ehrs/contribution",
        ));
    }
    if !seeded.party.is_empty() {
        let party = format!("/parties/person/{}", seeded.party);
        all.push(light(party.clone(), "parties/party"));
        all.push(review(
            format!("{party}?tab=versions"),
            "parties/party-versions",
        ));
        all.push(review(format!("{party}/edit"), "parties/party-edit"));
    }
    all
}

/// The four layouts every route is captured in: width, theme and file suffix.
const LAYOUTS: [(u32, bool, &str); 4] = [
    (1440, false, "1440-light"),
    (1440, true, "1440-dark"),
    (PHONE_WIDTH, false, "390-light"),
    (PHONE_WIDTH, true, "390-dark"),
];

/// Capture every route in every layout, and the book's subset.
async fn capture_routes(h: &Harness, routes: &[Route], review: &Path, book: &Path) {
    for (width, dark, suffix) in LAYOUTS {
        h.emulate(width, dark).await;
        for route in routes {
            h.goto(&route.path).await;
            h.wait_css("main").await;
            shot_to(h, review, &format!("{}-{suffix}", route.slug)).await;
            match (width, dark) {
                (1440, false) if route.in_book => shot_to(h, book, route.slug).await,
                (1440, true) if route.dark_in_book => {
                    shot_to(h, book, &format!("{}-dark", route.slug)).await;
                }
                _ => {}
            }
        }
    }
    h.emulate(1440, false).await;
}

/// The states a reader reaches only by a click, at 1440 px light, for the
/// book.
async fn capture_states(h: &Harness, seeded: &Seeded, book: &Path) {
    h.goto("/").await;
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu:popover-open").await;
    shot_to(h, book, "dashboard/user-menu").await;

    h.goto(&format!("/ehrs/{}", seeded.ehr)).await;
    h.wait_css("summary[aria-controls='ehr-more']")
        .await
        .click()
        .await;
    h.wait_css("button[command='show-modal'][commandfor='delete-ehr']")
        .await
        .click()
        .await;
    common::wait_visible(h, "#delete-confirm").await;
    shot_to(h, book, "ehrs/ehr-delete").await;

    h.goto(&format!("/templates/{TEMPLATE_ID}?confirm=delete"))
        .await;
    h.wait_css("main form.fui-cluster").await;
    shot_to(h, book, "templates/template-delete").await;

    h.goto("/query/aql").await;
    fill(
        h,
        "#aql",
        "SELECT c/context/start_time/value AS observed, \
         c/content[openEHR-EHR-EVALUATION.minimal.v1]/data[at0001]/items[at0002]/value/magnitude AS weight \
         FROM EHR e CONTAINS COMPOSITION c \
         WHERE c/archetype_details/template_id/value = 'minimal_evaluation.en.v1'",
    )
    .await;
    common::submit(h, "header.fui-page-header button[form='query-form']").await;
    h.wait_css("main table tbody tr").await;
    shot_to(h, book, "queries/aql-results").await;
    common::submit(h, "main button[name='shown'][value='chart']").await;
    h.wait_css("#results-chart-plot").await;
    shot_to(h, book, "queries/results-chart").await;

    h.goto("/audit").await;
    h.wait_css("main table tbody a.fui-row-link")
        .await
        .click()
        .await;
    h.wait_css("#audit-record-text").await;
    shot_to(h, book, "audit/audit-record").await;
}

/// Capture the console's screenshots: every route in four layouts for the
/// review, and the book's set.
#[tokio::test]
async fn capture_documentation_screenshots() {
    let Some(h) = Harness::start("docs-shots").await else {
        return;
    };
    if env("UI_E2E_DOCS_SHOTS").is_none() {
        println!("SKIP docs-shots: UI_E2E_DOCS_SHOTS unset (run scripts/ui-e2e.sh with it)");
        h.finish().await;
        return;
    }
    let (Some(cdr), Some(ehr), Some(composition)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_SEEDED_EHR_ID"),
        env("UI_E2E_SEEDED_VO_ID"),
    ) else {
        println!("SKIP docs-shots: the seeded stack env is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let user = env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned());
    let pass = env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    let seeded = seed(&cdr, &user, &pass, ehr, composition).await;
    let book = book_img_dir();
    let review = PathBuf::from(
        env("UI_E2E_SHOTS_DIR").unwrap_or_else(|| "target/ui-e2e/screenshots".to_owned()),
    )
    .join("routes");

    // The sign-in page, before any session and after one ends.
    h.goto("/login").await;
    h.wait_css("#sign-in-username").await;
    shot_to(&h, &book, "login/login").await;
    h.goto("/login?expired=1&next=%2Fehrs").await;
    h.wait_css("#sign-in-username").await;
    shot_to(&h, &book, "login/login-expired").await;

    login_basic_as(&h, &user, &pass).await;
    let all = routes(&seeded);
    capture_routes(&h, &all, &review, &book).await;
    capture_states(&h, &seeded, &book).await;

    h.assert_console_clean(&["401", "403", "404", "Failed to load resource"])
        .await;
    h.finish().await;
}
