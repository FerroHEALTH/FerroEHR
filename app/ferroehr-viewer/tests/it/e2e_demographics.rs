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
//! End-to-end journeys over the viewer's **demographics** section — the
//! openEHR Demographic API plus this CDR's `PARTY_RELATIONSHIP` extension:
//!
//! - **the party lifecycle**: creating a PERSON from the viewer's minimal
//!   skeleton, reading it, committing a second version through the edit form,
//!   and finding both versions in the History tab, each opening its own
//!   document;
//! - **relating two parties**: creating an ORGANISATION, relating the PERSON to
//!   it from the party's Relationships tab, and confirming the relationship
//!   resource names BOTH ends and that each end's link resolves to that party's
//!   screen;
//! - **tags**: setting and deleting a party tag, and finding that tag in the
//!   space-wide demographic tag index, whose "Open party" resolves the tagged
//!   id back to its kind;
//! - **the no-JavaScript contract**: the by-id lookup as a plain HTML round
//!   trip, and an unknown `:kind` answered as a not-found screen.
//!
//! Isolation: every journey creates its OWN parties (over the viewer for the
//! lifecycle one, over ITS-REST for the others), so none touches the fixtures
//! the other journeys or the documentation-screenshot pass depend on.

use crate::common;

use common::{
    Harness, click_until_css, login_basic, poll_until, read_prop, read_text, retype, wait_enabled,
    wait_text, wait_text_suffix,
};

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

/// Wait until the browser screen has re-rendered for `plural` (its page
/// heading) AND its create card seeds that kind's document, returning whether
/// both landed.
///
/// Both halves matter: the heading proves the screen re-rendered at all, and the
/// seeded skeleton proves the UNCONTROLLED create textarea was replaced rather
/// than left holding the previous kind's document — which is what a
/// patched-in-place rebuild leaves behind.
async fn wait_kind_screen(h: &Harness, plural: &str, rm_type: &str) -> bool {
    let needle = format!("\"_type\": \"{rm_type}\"");
    poll_until(async || {
        // NOTE: a heading or body detached by a re-render is "not yet" for this
        // poll; a driver that fails to answer panics inside read_text/read_prop.
        let heading = read_text(h, "h1").await.unwrap_or_default();
        heading.trim() == plural
            && read_prop(h, "#party-create-body", "value")
                .await
                .unwrap_or_default()
                .contains(&needle)
    })
    .await
}

/// The party lifecycle through the viewer alone: create a PERSON from the
/// seeded skeleton, read it, commit a second version, and walk both versions in
/// the History tab.
#[tokio::test]
async fn party_create_read_update_and_history() {
    let Some(h) = Harness::start("demographics-party").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/demographics/person").await;
    // The kind switcher and the wire-honest lookup are the screen's spine.
    h.wait_css("[data-kind='organisation']").await;
    h.wait_css("#party-lookup").await;
    h.shot(1, "party-browser").await;

    // Switching kind is a SAME-ROUTE navigation, which leptos_router answers by
    // updating the params without re-running the page body — so the whole screen
    // has to be driven off the param reactively. Clicking a pill and reading the
    // create skeleton back is what proves it: a screen that read its kind once
    // would still say PERSON here.
    h.wait_css("[data-kind='organisation']").await.click().await;
    h.wait_url_contains("/demographics/organisation").await;
    assert!(
        wait_kind_screen(&h, "Organisations", "ORGANISATION").await,
        "the kind switcher must re-render the screen for the new kind: {}",
        h.evidence_dump("kind-switch").await
    );
    h.shot(2, "kind-switched").await;
    // …and back, so the rest of the journey runs on the person screen.
    h.wait_css("[data-kind='person']").await.click().await;
    h.wait_url_contains("/demographics/person").await;
    assert!(
        wait_kind_screen(&h, "People", "PERSON").await,
        "switching back must re-render the PERSON screen"
    );

    // Create from the viewer's own minimal skeleton, with a unique identity so
    // the created party is recognizable in its own document.
    let label = format!("viewer-person-{}", jitter());
    let body = serde_json::to_string(&party_body("PERSON", "person", &label))
        .expect("serialize the party");
    retype(&h, "#party-create-body", &body).await;
    h.wait_css("#party-create-submit").await.click().await;
    assert!(
        wait_text(&h, "Party created").await,
        "creating a PERSON never reported a committed version: {}",
        h.evidence_dump("party-create").await
    );
    h.wait_url_contains("/demographics/person/").await;
    h.wait_css("#party-facts").await;
    // A first version, and the document really carries what was created.
    wait_text_suffix(&h, "[data-demographic-fact='version']", "::1").await;
    h.wait_css("#party-document").await;
    assert!(
        wait_text(&h, &label).await,
        "the created identity is not in the served document"
    );
    h.shot(2, "party-created").await;

    // Commit a second version through the edit form: `details` is the party's
    // optional ITEM_STRUCTURE, and the form re-sends everything else verbatim.
    h.wait_toasts_cleared().await;
    // The form stays disabled until it is seeded; typing before that would be
    // overwritten by the seed.
    wait_enabled(&h, "#party-identities").await;
    wait_enabled(&h, "#party-details").await;
    retype(
        &h,
        "#party-details",
        r#"{"_type":"ITEM_TREE","name":{"_type":"DV_TEXT","value":"details"},"archetype_node_id":"at0004","items":[]}"#,
    )
    .await;
    h.wait_css("#party-save").await.click().await;
    assert!(
        wait_text(&h, "Party updated").await,
        "the edit never reported a committed version: {}",
        h.evidence_dump("party-update").await
    );
    // The refetched facts report the NEW version — the write really landed.
    wait_text_suffix(&h, "[data-demographic-fact='version']", "::2").await;
    h.shot(3, "party-updated").await;

    // History: the versioned family, never the current-party route.
    h.wait_toasts_cleared().await;
    let party = current_party_uid(&h).await;
    h.goto(&format!("/demographics/person/{party}?tab=history"))
        .await;
    h.wait_css("[data-demographic-fact='object-uid']").await;
    // Two versions: the create and the edit.
    h.wait_css("[data-demographic-version$='::2']").await;
    h.wait_css("[data-demographic-version$='::1']").await;
    assert!(
        click_until_css(
            &h,
            "[data-demographic-version$='::1']",
            "#demographic-version-document",
        )
        .await,
        "opening version 1 never rendered its document"
    );
    wait_text_suffix(&h, "[data-demographic-fact='version-id']", "::1").await;
    // The version's envelope names its contribution, linked to its own viewer.
    h.wait_css("[data-demographic-fact='contribution']").await;
    h.shot(4, "party-history").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Relating two parties: an ORGANISATION target, a relationship created from
/// the PERSON's Relationships tab, and both ends linked from the relationship
/// resource — then the same link followed party → party from the inline list.
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
    h.goto(&format!("/demographics/person/{person}?tab=relationships"))
        .await;
    // A fresh party is the source of nothing yet, and the tab says why the other
    // direction cannot be listed at all.
    h.wait_css("#party-relationships").await;
    assert!(
        wait_text(&h, "point AT this party").await,
        "the tab must state that incoming relationships are not enumerable"
    );
    h.shot(1, "relationships-empty").await;

    // "Relate this party" carries the source into the create form as URL state.
    h.wait_css("#party-relate").await.click().await;
    h.wait_url_contains("/demographics/relationship?source=")
        .await;
    let prefilled = h.wait_css("#relationship-source").await.prop("value").await;
    assert_eq!(
        prefilled, person,
        "the source party must arrive prefilled from the URL"
    );

    retype(&h, "#relationship-type", "employment").await;
    retype(&h, "#relationship-target", &organisation).await;
    h.wait_css("#relationship-target-kind")
        .await
        .select_by_value("organisation")
        .await;
    h.wait_css("#relationship-create-submit")
        .await
        .click()
        .await;
    assert!(
        wait_text(&h, "Relationship created").await,
        "creating the relationship never reported a committed version: {}",
        h.evidence_dump("relationship-create").await
    );
    h.wait_url_contains("/demographics/relationship/").await;

    // The relationship resource names BOTH ends, each linked to its party.
    h.wait_css("#relationship-facts").await;
    let source_link = h
        .wait_css("[data-relationship-end='source']")
        .await
        .attr("href")
        .await;
    let target_link = h
        .wait_css("[data-relationship-end='target']")
        .await
        .attr("href")
        .await;
    assert_eq!(source_link, format!("/demographics/person/{person}"));
    assert_eq!(
        target_link,
        format!("/demographics/organisation/{organisation}")
    );
    h.shot(2, "relationship-created").await;

    // …and following the target end lands on that party's own screen.
    h.wait_toasts_cleared().await;
    h.wait_css("[data-relationship-end='target']")
        .await
        .click()
        .await;
    h.wait_url_contains(&format!("/demographics/organisation/{organisation}"))
        .await;
    h.wait_css("#party-facts").await;
    assert!(
        wait_text(&h, "ORGANISATION").await,
        "the target party's own screen must load"
    );
    h.shot(3, "relationship-target-party").await;

    // The inline relationship on the source party's tab links party → party —
    // the one same-route navigation on the party detail, which leptos_router
    // answers by updating params without re-running the page body. A screen that
    // read its kind once would keep addressing the previous family, so drive the
    // link and assert the new screen really is the ORGANISATION's. The inline
    // list is a DIFFERENT record from the resource created above (the two
    // representations are disjoint), so it is put there over REST first.
    add_inline_relationship(&http, &v1, "person", &person, "ORGANISATION", &organisation).await;
    h.wait_toasts_cleared().await;
    h.goto(&format!("/demographics/person/{person}?tab=relationships"))
        .await;
    let inline = h.wait_css("[data-relationship-end='inline-target']").await;
    assert_eq!(
        inline.attr("href").await,
        format!("/demographics/organisation/{organisation}"),
        "the party's own relationships list must link its target"
    );
    inline.click().await;
    h.wait_url_contains(&format!("/demographics/organisation/{organisation}"))
        .await;
    wait_text_suffix(&h, "[data-demographic-fact='type']", "ORGANISATION").await;
    h.shot(4, "inline-relationship-followed").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Tags: set one on a party, find it in the space-wide index, open the party
/// back from that index row, and delete it.
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
    // A unique key, so the index filter finds exactly this journey's tag.
    let key = format!("viewer-tag-{}", jitter());

    login_basic(&h).await;
    h.goto(&format!("/demographics/person/{person}?tab=tags"))
        .await;
    h.wait_css("#party-tag-set").await;
    retype(&h, "#tag-key", &key).await;
    retype(&h, "#tag-value", "follow-up").await;
    h.wait_css("#tag-save").await.click().await;
    assert!(
        wait_text(&h, "Tag saved").await,
        "saving a tag never reported the replaced collection: {}",
        h.evidence_dump("tag-save").await
    );
    h.wait_css(&format!("[data-tag-key='{key}']")).await;
    h.shot(1, "tag-set").await;

    // The space-wide index lists it, and "Open party" resolves the tagged id
    // back to the kind that holds it.
    h.wait_toasts_cleared().await;
    h.goto(&format!("/demographics/person?tag_key={key}")).await;
    h.wait_css("#demographic-tag-index").await;
    let row = format!("[data-tag-target='{person}']");
    h.wait_css(&row).await;
    h.shot(2, "tag-index").await;
    h.wait_css(&row).await.click().await;
    h.wait_url_contains(&format!("/demographics/person/{person}"))
        .await;
    h.wait_css("#party-facts").await;

    // Deleting addresses the key alone, which is what the openEHR tag delete
    // does — and the party is then untagged.
    h.goto(&format!("/demographics/person/{person}?tab=tags"))
        .await;
    h.wait_css(&format!("[data-tag-delete='{key}']"))
        .await
        .click()
        .await;
    assert!(
        wait_text(&h, "Tag deleted").await,
        "deleting the tag never reported: {}",
        h.evidence_dump("tag-delete").await
    );
    assert!(
        wait_text(&h, "No tags on this party").await,
        "the party must be untagged after the delete"
    );
    h.shot(3, "tag-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The progressive-enhancement contract: the by-id lookup is a plain HTML GET
/// that works with JavaScript disabled, and an unknown `:kind` is a not-found
/// screen rather than a wall of failing reads.
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
    h.goto("/demographics/person").await;
    h.wait_css("#party-lookup").await.send_keys(&person).await;
    h.wait_css("#party-find").await.click().await;
    // With no JavaScript this is a full GET to `?find=…`, which the screen
    // answers with a server-side redirect to the party's own route.
    h.wait_url_contains(&format!("/demographics/person/{person}"))
        .await;
    h.wait_css("#party-facts").await;
    h.shot(1, "nojs-lookup").await;

    // A `:kind` outside the closed five-kind set is a not-found screen.
    h.goto("/demographics/patients").await;
    h.wait_css("#demographics-unknown-kind").await;
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
