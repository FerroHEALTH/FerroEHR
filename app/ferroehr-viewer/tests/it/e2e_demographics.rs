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
//! End-to-end journeys over Parties: the openEHR Demographic API plus this
//! CDR's `PARTY_RELATIONSHIP` extension.
//!
//! - the party lifecycle: New party, the party's page, Edit committing a
//!   second version, and both versions on the Versions tab;
//! - relating two parties from the PERSON's Relationships tab, with both ends
//!   linked from the relationship's page;
//! - tags: setting and removing a party tag, and finding it in the tag index
//!   on Parties;
//! - the no-WebAssembly contract: Open as a plain GET form, and an unknown
//!   kind answered as a not-found page.
//!
//! Isolation: every journey creates its OWN parties (over the viewer for the
//! lifecycle one, over ITS-REST for the others), so none touches the fixtures
//! the other journeys or the documentation-screenshot pass depend on.

use crate::common;

use common::{Harness, fill, login_basic, wait_main_contains, wait_text_contains};

/// The CDR base URL the harness exports for REST-side test setup; `None` skips
/// with a reason.
fn cdr_url() -> Option<String> {
    if let Some(url) = common::env("UI_E2E_CDR_URL") {
        Some(url)
    } else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// The Basic credentials the composed stack seeds (the same defaults the shared
/// harness login uses).
fn basic_credentials() -> (String, String) {
    (
        common::env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned()),
        common::env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// A minimal RM-valid party body for `rm_type`, carrying `label` as its
/// identity's one ELEMENT value.
///
/// The shape is the one the CDR's own demographic corpus fixture uses
/// (`corpus/fixtures/demographic/person.v1.json`):
/// the ARCHETYPED block `PARTY`'s `Is_archetype_root` invariant requires, a root
/// `archetype_node_id` equal to the stringified `archetype_id`, and one
/// `PARTY_IDENTITY`.
fn party_body(rm_type: &str, segment: &str, label: &str) -> serde_json::Value {
    let archetype_id = format!("openEHR-DEMOGRAPHIC-{rm_type}.{segment}.v1");
    serde_json::json!({
        "_type": rm_type,
        "name": { "_type": "DV_TEXT", "value": rm_type },
        "archetype_node_id": archetype_id,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype_id },
            "rm_version": "1.1.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal identity" },
            "archetype_node_id": "at0001",
            "details": {
                "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "identity details" },
                "archetype_node_id": "at0002",
                "items": [{
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "archetype_node_id": "at0003",
                    "value": { "_type": "DV_TEXT", "value": label }
                }]
            }
        }]
    })
}

/// Create a party of `segment`/`rm_type` over ITS-REST and return its
/// versioned-object uid (the id every viewer route addresses).
///
/// # Panics
/// When the CDR refuses the create (a broken stack, not a skip).
async fn create_party_over_rest(
    http: &reqwest::Client,
    v1: &str,
    segment: &str,
    rm_type: &str,
    label: &str,
) -> String {
    let (user, pass) = basic_credentials();
    let response = http
        .post(format!("{v1}/demographic/{segment}"))
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Prefer", "return=representation")
        .body(
            serde_json::to_string(&party_body(rm_type, segment, label))
                .expect("serialize the party"),
        )
        .send()
        .await
        .expect("create a party");
    let status = response.status();
    let body: serde_json::Value = response.json().await.expect("party body");
    assert!(
        status.is_success(),
        "the CDR must accept the party create (got {status}): {body}"
    );
    let version_uid = body
        .get("uid")
        .and_then(|uid| uid.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the created version uid");
    container_of(version_uid)
}

/// The version container inside an `OBJECT_VERSION_ID` (`{uuid}::{system}::{n}`)
/// — the id the viewer's routes use.
fn container_of(version_uid: &str) -> String {
    version_uid
        .split("::")
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Give a party an inline `PARTY.relationships` entry pointing at
/// `target_container`, over ITS-REST.
///
/// The party's own `relationships` list and a standalone `PARTY_RELATIONSHIP`
/// resource are DISJOINT records in openEHR — neither is a view of the other —
/// so the viewer's Relationships tab, which reads the inline list, needs the
/// inline form put there explicitly. The update re-sends the served document
/// with one attribute added, exactly as the viewer's own edit does.
///
/// # Panics
/// When the read or the conditional update is refused (a broken stack).
async fn add_inline_relationship(
    http: &reqwest::Client,
    v1: &str,
    segment: &str,
    container: &str,
    target_kind: &str,
    target_container: &str,
) {
    let (user, pass) = basic_credentials();
    let mut body: serde_json::Value = http
        .get(format!("{v1}/demographic/{segment}/{container}"))
        .basic_auth(&user, Some(&pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the party")
        .json()
        .await
        .expect("party body");
    let version_uid = body
        .get("uid")
        .and_then(|uid| uid.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the served version uid")
        .to_owned();
    let party_ref = |rm_type: &str, id: &str| {
        serde_json::json!({
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": rm_type,
            "id": { "_type": "HIER_OBJECT_ID", "value": id }
        })
    };
    let inline = serde_json::json!([{
        "_type": "PARTY_RELATIONSHIP",
        "name": { "_type": "DV_TEXT", "value": "employment" },
        "archetype_node_id": "at0005",
        "source": party_ref("PERSON", container),
        "target": party_ref(target_kind, target_container)
    }]);
    drop(
        body.as_object_mut()
            .expect("the served party is a JSON object")
            .insert("relationships".to_owned(), inline),
    );
    let response = http
        .put(format!("{v1}/demographic/{segment}/{container}"))
        .basic_auth(&user, Some(&pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("If-Match", format!("\"{version_uid}\""))
        .body(serde_json::to_string(&body).expect("serialize the party"))
        .send()
        .await
        .expect("commit the inline relationship");
    let status = response.status();
    assert!(
        status.is_success(),
        "the inline-relationship update must be accepted (got {status}): {}",
        response.text().await.unwrap_or_default()
    );
}

/// The party page of `uid` of the kind `segment`.
fn party_path(segment: &str, uid: &str) -> String {
    format!("/parties/{segment}/{uid}")
}

/// The party's lifecycle through the viewer alone: create a PERSON on New
/// party, read it, commit a second version from Edit, and read both versions
/// on the Versions tab.
#[tokio::test]
async fn party_create_read_update_and_history() {
    let Some(h) = Harness::start("demographics-party").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/parties").await;
    h.wait_css("#p-kind").await;
    h.shot(1, "parties").await;

    // Start over swaps the starting document for the kind chosen.
    h.wait_css("main a[href='/parties/new']")
        .await
        .click()
        .await;
    h.wait_css("#pk")
        .await
        .select_by_value("organisation")
        .await;
    common::submit(&h, "button[value='kind']").await;
    let starting = h.wait_css("#document").await.prop("value").await;
    assert!(
        starting.contains("\"_type\": \"ORGANISATION\""),
        "Start over puts the organisation's starting document in the editor: {starting}"
    );
    h.wait_css("#pk").await.select_by_value("person").await;
    common::submit(&h, "button[value='kind']").await;

    let label = format!("viewer-person-{}", jitter());
    let body = serde_json::to_string(&party_body("PERSON", "person", &label))
        .expect("serialize the party");
    fill(&h, "#document", &body).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/parties/person/").await;
    h.wait_css(".fui-toast-text").await;
    wait_main_contains(&h, &label).await;
    wait_main_contains(&h, "version 1").await;
    h.shot(2, "party-created").await;
    let party = current_party_uid(&h).await;

    h.wait_css(&format!(
        "main a[href='{}/edit']",
        party_path("person", &party)
    ))
    .await
    .click()
    .await;
    let document = h.wait_css("#document").await.prop("value").await;
    let edited = document.replacen(&label, &format!("{label} edited"), 1);
    assert_ne!(edited, document, "the opened document carries the identity");
    fill(&h, "#document", &edited).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_not_contains("/edit").await;
    wait_main_contains(&h, "version 2").await;
    wait_main_contains(&h, &format!("{label} edited")).await;
    h.shot(3, "party-updated").await;

    h.goto(&format!("{}?tab=versions", party_path("person", &party)))
        .await;
    wait_main_contains(&h, "Version 2").await;
    h.wait_css("main a.fui-row-link[href*='version=1']").await;
    h.goto(&format!(
        "{}?tab=versions&version=1",
        party_path("person", &party)
    ))
    .await;
    wait_main_contains(&h, "None: this is the first version").await;
    h.shot(4, "party-history").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Relating two parties: a relationship created from the PERSON's
/// Relationships tab links both ends from its own page, and the PERSON's
/// relationships list then links its target.
#[tokio::test]
async fn relating_two_parties_links_both_ends() {
    let Some(h) = Harness::start("demographics-relationship").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let person =
        create_party_over_rest(&http, &v1, "person", "PERSON", "relationship-source").await;
    let organisation =
        create_party_over_rest(&http, &v1, "organisation", "ORGANISATION", "employer").await;

    login_basic(&h).await;
    h.goto(&format!(
        "{}?tab=relationships",
        party_path("person", &person)
    ))
    .await;
    wait_main_contains(&h, "cannot list the ones that point at it").await;
    h.shot(1, "relationships-empty").await;

    h.wait_css("main a.fui-button[href^='/parties/relationships/new']")
        .await
        .click()
        .await;
    h.wait_url_contains("/parties/relationships/new?").await;
    // Discard leaves without asking anything of the empty fields.
    common::submit(&h, "#editor-dock button[value='discard']").await;
    h.wait_url_contains("tab=relationships").await;
    h.wait_css("main a.fui-button[href^='/parties/relationships/new']")
        .await
        .click()
        .await;

    fill(&h, "#r-type", "employment").await;
    h.wait_css("#r-tk")
        .await
        .select_by_value("organisation")
        .await;
    fill(&h, "#r-tid", &organisation).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/parties/relationships/").await;
    h.wait_css(".fui-toast-text").await;
    h.wait_css(&format!("main a[href='{}']", party_path("person", &person)))
        .await;
    let target = format!(
        "main a[href='{}']",
        party_path("organisation", &organisation)
    );
    h.wait_css(&target).await;
    h.shot(2, "relationship-created").await;

    h.wait_css(&target).await.click().await;
    h.wait_url_contains(&party_path("organisation", &organisation))
        .await;
    wait_main_contains(&h, "Organisation").await;
    h.shot(3, "relationship-target-party").await;

    add_inline_relationship(&http, &v1, "person", &person, "ORGANISATION", &organisation).await;
    h.goto(&format!(
        "{}?tab=relationships",
        party_path("person", &person)
    ))
    .await;
    // A relationship held inline in the party has no page of its own, so its
    // row opens the target.
    h.wait_css(&format!(
        "main a.fui-row-link[href='{}']",
        party_path("organisation", &organisation)
    ))
    .await
    .click()
    .await;
    h.wait_url_contains(&party_path("organisation", &organisation))
        .await;
    h.shot(4, "inline-relationship-followed").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Tags on a party: set one on its Tags tab, find it in the tag index on
/// Parties, open the party from that row, and remove the tag.
#[tokio::test]
async fn party_tags_are_set_indexed_and_deleted() {
    let Some(h) = Harness::start("demographics-tags").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let person = create_party_over_rest(&http, &v1, "person", "PERSON", "tagged-person").await;
    let key = format!("viewer-tag-{}", jitter());

    login_basic(&h).await;
    let tags = format!("{}?tab=tags", party_path("person", &person));
    h.goto(&tags).await;
    fill(&h, "#tk", &key).await;
    fill(&h, "#tv", "follow-up").await;
    common::submit(&h, "button[value='set-tag']").await;
    h.wait_css(".fui-toast-text").await;
    wait_text_contains(&h, "main table", &key).await;
    h.shot(1, "tag-set").await;

    h.goto("/parties").await;
    wait_text_contains(&h, "main table", &key).await;
    h.shot(2, "tag-index").await;
    h.wait_css(&format!(
        "main table a[href^='{}']",
        party_path("person", &person)
    ))
    .await
    .click()
    .await;
    h.wait_url_contains(&party_path("person", &person)).await;

    h.goto(&tags).await;
    common::submit(&h, &format!("button[value='remove-tag:{key}']")).await;
    h.wait_css(".fui-toast-text").await;
    wait_main_contains(&h, "carries no tag").await;
    h.shot(3, "tag-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The no-WebAssembly contract: Open on Parties is a plain GET form that
/// opens the party, and an unknown kind is a not-found page rather than a wall
/// of failing reads.
#[tokio::test]
async fn lookup_and_unknown_kind_work_without_javascript() {
    let Some(h) = Harness::start_without_javascript("demographics-nojs").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let person = create_party_over_rest(&http, &v1, "person", "PERSON", "nojs-person").await;

    login_basic(&h).await;
    h.goto("/parties").await;
    fill(&h, "#p-id", &person).await;
    common::submit(&h, "form[action='/parties'] button[type=submit]").await;
    h.wait_url_contains(&party_path("person", &person)).await;
    wait_main_contains(&h, "nojs-person").await;
    h.shot(1, "nojs-lookup").await;

    h.goto(&party_path("patients", &person)).await;
    wait_main_contains(&h, "404").await;
    h.shot(2, "nojs-unknown-kind").await;
    h.finish().await;
}

/// A run-unique suffix for the fixtures a journey creates, so a shared stack
/// can hold several runs' parties and tags without collision.
///
/// The process id distinguishes runs and the counter distinguishes calls within
/// one — the harness needs distinctness, not entropy, and no clock (the
/// wall-clock reads a `disallowed-methods` ban anyway).
fn jitter() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{seq}", std::process::id())
}

/// The versioned-object uid of the party the browser is currently on, read from
/// the URL path.
///
/// # Panics
/// When the current URL is not a party detail route.
async fn current_party_uid(h: &Harness) -> String {
    let url = h.current_url().await;
    url.split(['?', '#'])
        .next()
        .unwrap_or(&url)
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .expect("a party detail path")
        .to_owned()
}
