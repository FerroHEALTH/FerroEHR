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
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end directory journeys: Create the directory, the folder editor
//! (add a folder, save with `If-Match`), the reference picker's add and remove
//! round trip, the version history with restore and Older versions, and Delete
//! the directory with Create the directory coming back. Each journey creates
//! its own EHR (never the seeded one: these journeys change and delete the
//! directory) and fails on any browser console error.
//!
//! The editor is a form the server answers with the tree changed: each add,
//! rename and remove posts it, and the tree travels in a hidden field until
//! Save sends it to the CDR.
//!
//! The item-picker journey seeds its two compositions over ITS-REST
//! (`UI_E2E_CDR_URL`) rather than through the UI: the commit form has its own
//! journey, and what this one is about is which REFERENCE survives a sibling
//! removal.

use crate::common;

use common::{Harness, confirm_in_dialog, env, login_basic, wait_css_absent, wait_main_contains};

/// The folder tree of the editor and of the tab.
const TREE: &str = "#directory-tree";

/// Create an EHR in the viewer and return its id.
async fn create_ehr(h: &Harness) -> String {
    h.goto("/ehrs/new").await;
    common::submit(h, "#editor-dock button[value='save']").await;
    let url = h.current_url().await;
    url.split("/ehrs/")
        .nth(1)
        .map(|tail| tail.chars().take(36).collect::<String>())
        .filter(|id| id.len() == 36)
        .unwrap_or_else(|| panic!("EHR creation never landed on an EHR (last url {url})"))
}

/// Create the EHR's empty directory from the EHR's More menu.
async fn create_empty_directory(h: &Harness, ehr_id: &str) {
    h.goto(&format!("/ehrs/{ehr_id}")).await;
    h.wait_css("summary[aria-controls='ehr-more']").await.click().await;
    common::submit(h, "#ehr-more form button").await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}/directory")).await;
    h.wait_css(TREE).await;
}

/// Open the editor of the directory.
async fn open_editor(h: &Harness, ehr_id: &str) {
    h.goto(&format!("/ehrs/{ehr_id}/directory/edit")).await;
    h.wait_css("#directory-edit").await;
}

/// The credentials and CDR the seeding needs, or `None` with a printed skip.
fn seeding() -> Option<(String, String, String)> {
    match (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) {
        (Some(cdr), Some(user), Some(pass)) => Some((cdr, user, pass)),
        _ => {
            println!("SKIP: seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
            None
        }
    }
}

/// The folder editor: add a folder at the root, save with the opened
/// version's `If-Match`, and the folder survives the reread.
#[tokio::test]
async fn directory_tree_edit_journey() {
    let Some(h) = Harness::start("directory-edit").await else {
        return;
    };
    login_basic(&h).await;
    let ehr_id = create_ehr(&h).await;
    create_empty_directory(&h, &ehr_id).await;

    open_editor(&h, &ehr_id).await;
    common::submit(&h, "button[name='add_folder'][value='n0']").await;
    common::wait_text_contains(&h, TREE, "New folder").await;
    h.shot(1, "directory-folder-added").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_not_contains("/edit").await;
    common::wait_text_contains(&h, TREE, "New folder").await;
    wait_main_contains(&h, "Version 2").await;
    h.shot(2, "directory-saved").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// The template the picker's fixture compositions use.
const PICKER_TEMPLATE: &str = "minimal_evaluation.en.v1";


/// Commit one FLAT composition into `ehr_id` and return its VERSIONED-OBJECT
/// id — the `HIER_OBJECT_ID` the directory's `OBJECT_REF` items carry, and
/// therefore the identity a picker row and an item row are matched on.
///
/// # Panics
/// When the CDR refuses the commit or answers without a `uid.value`.
async fn seed_picker_composition(
    cdr: &str,
    user: &str,
    pass: &str,
    ehr_id: &str,
    composer: &str,
    time: &str,
) -> String {
    let body: serde_json::Value = reqwest::Client::new()
        .post(format!(
            "{cdr}/ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition"
        ))
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/openehr.wt.flat+json")
        .header("Accept", "application/json")
        .header("openehr-template-id", PICKER_TEMPLATE)
        .header("Prefer", "return=representation")
        .json(&serde_json::json!({
            "ctx/language": "en",
            "ctx/territory": "US",
            "ctx/composer_name": composer,
            "ctx/time": time,
            "minimal/minimal/quantity|magnitude": 37.2,
            "minimal/minimal/quantity|unit": "kg",
        }))
        .send()
        .await
        .expect("commit a picker fixture composition")
        .error_for_status()
        .expect("the CDR accepted the picker fixture composition")
        .json()
        .await
        .expect("the committed composition");
    let version_uid = body
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("the committed composition's uid");
    version_uid
        .split("::")
        .next()
        .expect("a version uid has a container half")
        .to_owned()
}


/// The item ids the CDR's own directory read reports, in document order.
///
/// The browser half of an assertion can only prove what the SCREEN shows; this
/// is the other half — what the repository actually holds after the save.
///
/// # Panics
/// When the CDR refuses the read or answers a body that is not a FOLDER.
async fn stored_item_ids(cdr: &str, user: &str, pass: &str, ehr_id: &str) -> Vec<String> {
    let folder: serde_json::Value = reqwest::Client::new()
        .get(format!(
            "{cdr}/ferroehr/rest/openehr/v1/ehr/{ehr_id}/directory"
        ))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the directory back")
        .error_for_status()
        .expect("the CDR served the directory")
        .json()
        .await
        .expect("the directory FOLDER");
    folder
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.pointer("/id/value")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Add a reference to the composition `object_id` at the root.
async fn add_reference(h: &Harness, object_id: &str) {
    common::submit(h, "button[name='add_reference'][value='n0']").await;
    h.wait_css("#d-reference").await.select_by_value(object_id).await;
    common::submit(h, "button[name='intent'][value='edit']").await;
}

/// The picker adds two references and removes one, and the reference that
/// survives is the other one, on screen and in the CDR: removing a sibling is
/// where a position-keyed row would hand its neighbour's identity to the
/// remove.
#[tokio::test]
async fn directory_item_picker_add_remove_journey() {
    let Some(h) = Harness::start("directory-picker").await else {
        return;
    };
    let Some((cdr, user, pass)) = seeding() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    let ehr_id = create_ehr(&h).await;
    let kept = seed_picker_composition(&cdr, &user, &pass, &ehr_id, "Picker kept", "2026-04-01T08:00:00Z").await;
    let removed = seed_picker_composition(&cdr, &user, &pass, &ehr_id, "Picker removed", "2026-04-02T08:00:00Z").await;
    assert_ne!(kept, removed, "the two fixtures must be distinct objects");
    create_empty_directory(&h, &ehr_id).await;

    open_editor(&h, &ehr_id).await;
    add_reference(&h, &removed).await;
    add_reference(&h, &kept).await;
    let tail = |id: &str| id.get(id.len() - 4..).expect("a long id").to_owned();
    common::wait_text_contains(&h, TREE, &tail(&removed)).await;
    common::wait_text_contains(&h, TREE, &tail(&kept)).await;
    h.shot(1, "directory-two-references").await;

    let remove = h
        .wait_xpath(&format!(
            "//ul[@id='directory-tree']//li[contains(., '{}')][not(.//li[contains(., '{}')])]//button[@name='remove']",
            tail(&removed),
            tail(&removed)
        ))
        .await;
    remove.click().await;
    h.wait_css("#directory-edit").await;
    common::poll_until(async || {
        common::read_text(&h, TREE)
            .await
            .is_some_and(|text| !text.contains(&tail(&removed)))
    })
    .await;
    common::wait_text_contains(&h, TREE, &tail(&kept)).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_not_contains("/edit").await;
    h.shot(2, "directory-one-reference").await;

    assert_eq!(
        stored_item_ids(&cdr, &user, &pass, &ehr_id).await,
        vec![kept.clone()],
        "the CDR keeps exactly the reference that was not removed"
    );

    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// The version history: after an edit there are two versions; version 1 reads
/// on its own and restores, written back as version 3 with the latest
/// version's `If-Match`.
#[tokio::test]
async fn directory_history_and_restore_journey() {
    let Some(h) = Harness::start("directory-history").await else {
        return;
    };
    login_basic(&h).await;
    let ehr_id = create_ehr(&h).await;
    create_empty_directory(&h, &ehr_id).await;
    open_editor(&h, &ehr_id).await;
    common::submit(&h, "button[name='add_folder'][value='n0']").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, "Version 2").await;

    h.goto(&format!("/ehrs/{ehr_id}/directory")).await;
    h.wait_css("main a[aria-label='Restore version 1']").await.click().await;
    h.wait_url_contains("restore=1").await;
    wait_main_contains(&h, "You are reading version 1 of 2").await;
    h.shot(1, "directory-version-1").await;
    confirm_in_dialog(
        &h,
        "button[command='show-modal'][commandfor='restore-directory']",
        "#restore-directory button[type=submit]",
    )
    .await;
    h.wait_css(".fui-toast-text").await;
    wait_main_contains(&h, "Version 3").await;
    let tree = h.wait_css(TREE).await.text().await;
    assert!(!tree.contains("New folder"), "version 1's empty tree is back: {tree}");
    h.shot(2, "directory-restored").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}


/// Commit one version of the EHR's directory over ITS-REST and return the new
/// `OBJECT_VERSION_ID`. `previous` is the `If-Match` precondition — `None`
/// creates the directory (v1), `Some(uid)` supersedes that version.
///
/// Seeding a deep history through the tree editor would be dozens of hydrated
/// click/save cycles; what this journey is about is the READ side of that
/// history, so the versions arrive at the wire.
///
/// # Panics
/// When the CDR refuses the write or answers without a `uid.value`.
async fn commit_directory_version(
    cdr: &str,
    user: &str,
    pass: &str,
    ehr_id: &str,
    name: &str,
    previous: Option<&str>,
) -> String {
    let client = reqwest::Client::new();
    let url = format!("{cdr}/ferroehr/rest/openehr/v1/ehr/{ehr_id}/directory");
    let request = match previous {
        None => client.post(&url),
        Some(uid) => client.put(&url).header("If-Match", format!("\"{uid}\"")),
    };
    let body: serde_json::Value = request
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Prefer", "return=representation")
        .json(&serde_json::json!({
            "_type": "FOLDER",
            "archetype_node_id": "openEHR-EHR-FOLDER.directory.v1",
            "name": { "_type": "DV_TEXT", "value": name },
            "folders": [],
            "items": [],
        }))
        .send()
        .await
        .expect("commit a directory version")
        .error_for_status()
        .expect("the CDR accepted the directory version")
        .json()
        .await
        .expect("the committed directory");
    body.pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("the committed directory's uid")
        .to_owned()
}

/// A history deeper than one window: the tab reads only the newest versions,
/// Older versions reaches the ones below, and a version reached that way still
/// restores. ITS-REST 1.1.0 has no FOLDER revision history, so the viewer reads
/// a window of versions back from the latest.
#[tokio::test]
async fn directory_history_load_older_journey() {
    let Some(h) = Harness::start("directory-history-older").await else {
        return;
    };
    let Some((cdr, user, pass)) = seeding() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    let ehr_id = create_ehr(&h).await;
    let deepest = 12_u32;
    let mut previous: Option<String> = None;
    for number in 1..=deepest {
        let uid = commit_directory_version(
            &cdr,
            &user,
            &pass,
            &ehr_id,
            &format!("revision {number}"),
            previous.as_deref(),
        )
        .await;
        previous = Some(uid);
    }

    let tab = format!("/ehrs/{ehr_id}/directory");
    h.goto(&tab).await;
    wait_main_contains(&h, &format!("Version {deepest}")).await;
    assert!(
        !common::is_present(&h, &format!("main a.fui-row-link[href='{tab}?version=1']")).await,
        "the first read stops at the newest window, but version 1 of {deepest} is listed"
    );
    h.shot(1, "history-first-window").await;

    h.wait_css(&format!("main a[href='{tab}?versions=20']")).await.click().await;
    h.wait_css(&format!("main a.fui-row-link[href='{tab}?version=1']"))
        .await;
    h.wait_css("main a[aria-label='Restore version 1']").await.click().await;
    wait_main_contains(&h, &format!("You are reading version 1 of {deepest}")).await;
    h.shot(2, "history-older-loaded").await;
    confirm_in_dialog(
        &h,
        "button[command='show-modal'][commandfor='restore-directory']",
        "#restore-directory button[type=submit]",
    )
    .await;
    wait_main_contains(&h, &format!("Version {}", deepest + 1)).await;
    h.shot(3, "history-older-restored").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// Delete the directory: confirm, the tab offers Create the directory again,
/// and a new directory can then be created (RM `common` §Logical deletion, and
/// the CDR's one-live-directory rule).
#[tokio::test]
async fn directory_delete_and_recreate_journey() {
    let Some(h) = Harness::start("directory-delete").await else {
        return;
    };
    login_basic(&h).await;
    let ehr_id = create_ehr(&h).await;
    create_empty_directory(&h, &ehr_id).await;

    h.wait_css("summary[aria-controls='directory-more']")
        .await
        .click()
        .await;
    confirm_in_dialog(
        &h,
        "button[commandfor='delete-directory']",
        "#delete-directory button[type=submit]",
    )
    .await;
    h.wait_css(".fui-toast-text").await;
    wait_css_absent(&h, TREE).await;
    h.shot(1, "directory-deleted").await;

    create_empty_directory(&h, &ehr_id).await;
    h.shot(2, "directory-recreated").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}
