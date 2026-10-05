// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Directory tab.
//!
//! A complete directory experience over the ITS-REST DIRECTORY API — a
//! structured `FOLDER` tree editor (add / rename / delete folders, add /
//! remove item references), an advanced raw-JSON mode, version history with
//! time-travel and restore, a `version_at_time` view, a `?path=` subtree
//! query, and directory deletion — plus the create-empty path for an EHR that
//! has no directory yet.
//!
//! No openEHR spec governs the viewer — our own design; the wire it
//! reads/writes IS spec-bound: the DIRECTORY operations (ITS-REST
//! `specifications/operations/directory_*.yaml`), the `FOLDER` schema
//! (`specifications/schemas/ehr/Folder.yaml`), and RM common
//! `master05-directory_package`. Every `#[server]` fn below authenticates the
//! viewer session first and the CDR credential never reaches client-visible
//! state.
//!
//! This module owns the DIRECTORY `#[server]` fns, the shared wire types, and
//! the `directory_section` orchestrator; the view is split across
//! [`tree`] (the structured editor), [`panels`] (history / time / path /
//! delete), [`create`] (the empty-directory create flow), and the pure
//! [`edit`] helpers.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

pub mod create;
pub mod edit;
pub mod panels;
pub mod tree;

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "ssr")]
use crate::uid::uid_value_of;

use crate::components::data_table::table_skeleton;
use crate::error::ViewerError;
use crate::pages::ehr_detail::directory::create::create_section;
use crate::pages::ehr_detail::directory::panels::directory_toolbar;
use crate::pages::ehr_detail::directory::tree::{EditorState, seed, tree_editor};
use crate::pages::ehrs::ResultPage;

/// The EHR's directory as its canonical FOLDER JSON body plus the current
/// version uid (the FOLDER's `uid.value`, an `OBJECT_VERSION_ID`).
///
/// The uid is the `If-Match` value on update/delete, and it is the one the
/// CDR's own `ETag` named; the returned FOLDER body's `uid` is the fallback for
/// a server that sent no `ETag` — a FOLDER is `VERSIONABLE` and always carries
/// one (ITS-REST `specifications/schemas/ehr/Folder.yaml`; RM common
/// `master05-directory_package`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryState {
    /// The canonical FOLDER JSON body.
    pub body: String,
    /// The served version's `OBJECT_VERSION_ID`, or empty when neither the
    /// `ETag` nor the body carries one.
    pub version_uid: String,
}

/// One row of the directory version history: a past (or the current) FOLDER
/// version summarized for the history panel.
///
/// Carries fixed-size ints only so it is WASM-safe over the server-fn
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryVersion {
    /// The version's `OBJECT_VERSION_ID` (`uid.value`).
    pub version_uid: String,
    /// The version tree number (`1`, `2`, …).
    pub number: i32,
    /// The root FOLDER's display name at this version.
    pub root_name: String,
    /// Total descendant folders (excluding the root) at this version.
    pub folder_count: i32,
    /// Total item references across the tree at this version.
    pub item_count: i32,
    /// The full canonical FOLDER JSON body of this version.
    pub body: String,
    /// Whether this is the latest (current) version.
    pub is_latest: bool,
}

/// One window of the directory's version history: the rows the panel shows
/// plus whether older versions remain beyond them.
///
/// The default is the empty history (no rows, nothing older), which is what an
/// unopened panel and an EHR without a directory both read as.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryHistory {
    /// The fetched versions, newest first.
    pub versions: Vec<DirectoryVersion>,
    /// Whether versions older than the last row exist — what the panel's
    /// "load older" affordance is gated on.
    pub has_older: bool,
}

/// The outcome of a `version_at_time` directory read: present, deleted at that
/// time (`204`), or no directory at that time (`404`) — the three distinct
/// states the time-travel panel renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectoryAtTime {
    /// A directory FOLDER existed at the requested time.
    Present(DirectoryState),
    /// The directory was (logically) deleted at the requested time (`204`).
    DeletedAtTime,
    /// No directory existed at the requested time (`404`).
    NoneAtTime,
}

/// The outcome of a `?path=` subtree query: the matched sub-FOLDER body, or a
/// miss (`404` — no folder at that path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectorySubtree {
    /// The sub-FOLDER matched at the requested path (canonical JSON body).
    Found(String),
    /// No sub-FOLDER exists at the requested path.
    Missing,
}

/// The standard openEHR directory root archetype id (the FOLDER example in
/// ITS-REST `specifications/schemas/ehr/Folder.yaml`).
pub(crate) const DIRECTORY_ARCHETYPE: &str = "openEHR-EHR-FOLDER.directory.v1";

/// Node id reused by child folders; siblings differ by `name` (RM common
/// `master05-directory_package` §Paths — uniqueness modifiers).
pub(crate) const FOLDER_NODE_ID: &str = "at0001";

/// Build a canonical FOLDER JSON node from `archetype_node_id`, `name`, and
/// child `folders`. `_type` + mandatory `archetype_node_id`/`name` (from
/// LOCATABLE) + the FOLDER `folders`/`items` arrays per
/// `specifications/schemas/ehr/Folder.yaml`; no `uid` — the CDR assigns the
/// `OBJECT_VERSION_ID` on create.
pub(crate) fn folder_json(archetype_node_id: &str, name: &str, folders: &[Value]) -> Value {
    json!({
        "_type": "FOLDER",
        "archetype_node_id": archetype_node_id,
        "name": { "_type": "DV_TEXT", "value": name },
        "folders": folders,
        "items": [],
    })
}

/// An empty directory root FOLDER (the "empty root" create choice).
pub(crate) fn empty_root_folder() -> Value {
    folder_json(DIRECTORY_ARCHETYPE, "root", &[])
}

/// The toast detail for a directory write: the new version, or a generic line
/// when the CDR returned no representation body.
pub(crate) fn directory_toast_detail(uid: &str) -> String {
    if uid.is_empty() {
        "The directory was committed.".to_owned()
    } else {
        format!("New version {uid}")
    }
}

/// Whether an error is the CDR's optimistic-concurrency conflict (`412` — the
/// `If-Match` version is stale), which the UI surfaces as a distinct
/// "reload" toast rather than the generic inline diagnostic.
pub(crate) fn is_conflict(error: &ViewerError) -> bool {
    error.status_code() == Some(http::StatusCode::PRECONDITION_FAILED)
}

/// The EHR's directory (root `FOLDER`) as a [`DirectoryState`], or `None` when
/// the CDR has no directory for this EHR (a `404` is a first-class empty
/// state, not an error).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_directory(
    /// The EHR whose directory to read.
    ehr_id: String,
) -> Result<Option<DirectoryState>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/directory", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    // 404 = the EHR has no directory; 204 = the latest version is a logical
    // delete (`204_deleted_at_time` — ITS-REST `directory_get_at_time.yaml`).
    // Both are the first-class "no live directory" state: the create flow
    // renders, and creating opens a NEW hierarchy (the deleted one's history
    // stays readable by version_uid).
    if response.is(http::StatusCode::NOT_FOUND) || response.is(http::StatusCode::NO_CONTENT) {
        return Ok(None);
    }
    Ok(Some(directory_state(
        crate::cdr::CdrClient::expect_success(response)?,
    )))
}

#[cfg(feature = "ssr")]
/// Take a served FOLDER answer as the viewer's [`DirectoryState`], reading the
/// version the CDR named in its `ETag` and falling back to the body's `uid`.
///
/// The `ETag` "value is usually taken from e.g. `VERSIONED_OBJECT.uid.value`,
/// `VERSION.uid.value`" and is the identifier the conditional round-trip is
/// built on (ITS-REST
/// `specifications/docs/overview/Requests_and_responses.md` §`ETag` and
/// `Last-Modified`), so it — not a value re-derived from the document — is what
/// an `If-Match` echoes back.
fn directory_state(response: crate::cdr::CdrResponse) -> DirectoryState {
    let version_uid = response
        .etag_version_uid()
        .unwrap_or_else(|| uid_value_of(&response.body));
    DirectoryState {
        body: response.body,
        version_uid,
    }
}

/// Create the EHR's directory (`POST /ehr/{ehr_id}/directory`) from a canonical
/// JSON FOLDER `body`. `Content-Type: application/json`,
/// `Prefer: return=representation`, `Accept: application/json`; the new
/// version uid (`uid.value` of the returned FOLDER) is returned (empty when the
/// CDR returned no representation). Directory operations per ITS-REST
/// `specifications/operations/directory_create.yaml`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body; CDR transport errors pass
/// through; a non-2xx CDR answer (its validation diagnostics, which the UI
/// renders verbatim, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_directory(
    /// The EHR to create the directory in.
    ehr_id: String,
    /// The root FOLDER document to commit, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "the directory body is empty".to_owned(),
        ));
    }
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/directory", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[("Prefer", "return=representation")],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(uid_value_of(&response.body))
}

/// Update the EHR's directory (`PUT /ehr/{ehr_id}/directory`) with a new
/// canonical JSON FOLDER `body`. The version the seeding read's `ETag` named
/// (the `preceding_version_uid`) is sent quoted in `If-Match`, per ITS-REST
/// `specifications/operations/directory_update.yaml`;
/// `Prefer: return=representation` yields the new version uid. A stale uid is
/// answered `412` by the CDR and surfaces as `is_conflict`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body or missing current version uid;
/// CDR transport errors pass through; a non-2xx CDR answer (its validation
/// diagnostics, which the UI renders verbatim, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_directory(
    /// The EHR holding the directory.
    ehr_id: String,
    /// The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    /// The replacement root FOLDER document, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "the directory body is empty".to_owned(),
        ));
    }
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(
            "the current directory version uid is required to update".to_owned(),
        ));
    }
    let if_match = format!("\"{current}\"");
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/directory", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .put(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[
                ("If-Match", if_match.as_str()),
                ("Prefer", "return=representation"),
            ],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(uid_value_of(&response.body))
}

/// Delete the EHR's directory (`DELETE /ehr/{ehr_id}/directory`). The version
/// the seeding read's `ETag` named (the `preceding_version_uid`) is sent quoted
/// in `If-Match`, per ITS-REST
/// `specifications/operations/directory_delete.yaml`; a stale uid is answered
/// `412` and surfaces as `is_conflict`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a missing current version uid; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_directory(
    /// The EHR holding the directory.
    ehr_id: String,
    /// The version being deleted, sent as `If-Match`.
    current_version_uid: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(
            "the current directory version uid is required to delete".to_owned(),
        ));
    }
    let if_match = format!("\"{current}\"");
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/directory", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .delete(
            &session.credential,
            &url,
            &[("If-Match", if_match.as_str())],
        )
        .await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// Read the newest `window` versions of the EHR's directory, newest first.
///
/// The current FOLDER is read to obtain the latest `version_uid`
/// (`object_id::system::N`); the window's version numbers are then read by
/// constructed `version_uid` (`GET /ehr/{ehr_id}/directory/{version_uid}`,
/// ITS-REST `specifications/operations/directory_get_by_version_id.yaml`),
/// concurrently with the shared bound
/// ([`FANOUT_CONCURRENCY`](crate::cdr::FANOUT_CONCURRENCY)) and truncated at
/// the first `404`. An empty list means the EHR has no directory.
///
/// The request count is therefore `1 + min(window, N)` regardless of how deep
/// the history runs; the rest is reached by asking for a wider window.
///
/// NOTE: ITS-REST 1.1.0 exposes no VERSIONED_FOLDER revision history (register
/// AMB-24, upstream report #1490), so the uid list is synthesized from the
/// latest version number rather than read from the CDR.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_directory_versions(
    /// The EHR whose directory version list to read.
    ehr_id: String,
    /// How many of the newest versions to read.
    window: u32,
) -> Result<DirectoryHistory, ViewerError> {
    use futures::stream::{StreamExt, TryStreamExt};

    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let base = state
        .cdr
        .rest_v1(&format!("ehr/{}/directory", urlencoding::encode(&ehr_id)));
    let current = state
        .cdr
        .get(&session.credential, &base, "application/json")
        .await?;
    // 204 = a logically deleted directory (the same split `fetch_directory`
    // makes): there is no live version to anchor the walk, so the panel
    // renders the empty state instead of a bogus row built from the empty
    // body.
    if current.is(http::StatusCode::NOT_FOUND) || current.is(http::StatusCode::NO_CONTENT) {
        return Ok(DirectoryHistory::default());
    }
    let current_body = crate::cdr::CdrClient::expect_success(current)?.body;
    let latest_uid = uid_value_of(&current_body);
    let Some((prefix, latest_n)) = split_version_uid(&latest_uid) else {
        // A non-integer version tree id (a branch) — surface just the latest.
        return Ok(DirectoryHistory {
            versions: vec![summarize_directory_version(&current_body, 1, true)],
            has_older: false,
        });
    };
    let numbers = version_window(latest_n, window);
    let requested = numbers.len();
    let oldest_requested = numbers.last().copied().unwrap_or(latest_n);
    let (state, session, ehr_id, prefix) = (&state, &session, &ehr_id, &prefix);
    let fetched: Vec<Option<DirectoryVersion>> =
        futures::stream::iter(numbers.into_iter().map(|number| async move {
            let uid = format!("{prefix}::{number}");
            let url = state.cdr.rest_v1(&format!(
                "ehr/{}/directory/{}",
                urlencoding::encode(ehr_id),
                urlencoding::encode(&uid)
            ));
            let response = state
                .cdr
                .get(&session.credential, &url, "application/json")
                .await?;
            if response.is(http::StatusCode::NOT_FOUND) {
                return Ok(None);
            }
            let body = crate::cdr::CdrClient::expect_success(response)?.body;
            Ok::<_, ViewerError>(Some(summarize_directory_version(
                &body,
                number,
                number == latest_n,
            )))
        }))
        .buffered(crate::cdr::FANOUT_CONCURRENCY)
        .try_collect()
        .await?;
    // A `404` inside the window ends the history the same way the serial walk
    // did: nothing below a missing synthesized uid was ever committed.
    let versions: Vec<DirectoryVersion> = fetched
        .into_iter()
        .take_while(Option::is_some)
        .flatten()
        .collect();
    let truncated = versions.len() < requested;
    Ok(DirectoryHistory {
        has_older: !truncated && oldest_requested > 1,
        versions,
    })
}

/// The version numbers one history window covers, newest first: at most
/// `window` numbers ending at `latest_n`, never below `1`.
#[cfg(feature = "ssr")]
fn version_window(latest_n: i32, window: u32) -> Vec<i32> {
    let window = i32::try_from(window).unwrap_or(i32::MAX).max(1);
    let oldest = latest_n.saturating_sub(window).saturating_add(1).max(1);
    (oldest..=latest_n).rev().collect()
}

/// Read the directory FOLDER as it stood at `version_at_time`
/// (`GET /ehr/{ehr_id}/directory?version_at_time=…`, ITS-REST
/// `specifications/operations/directory_get_at_time.yaml`). `200` →
/// [`DirectoryAtTime::Present`], `204` → [`DirectoryAtTime::DeletedAtTime`],
/// `404` → [`DirectoryAtTime::NoneAtTime`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty time; CDR transport errors pass
/// through; any other non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_directory_at_time(
    /// The EHR holding the directory.
    ehr_id: String,
    /// The instant to resolve the directory at.
    time: String,
) -> Result<DirectoryAtTime, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let time = time.trim();
    if time.is_empty() {
        return Err(ViewerError::Invalid(
            "a date and time is required".to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/directory?version_at_time={}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(time)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    match response.status {
        http::StatusCode::NO_CONTENT => Ok(DirectoryAtTime::DeletedAtTime),
        http::StatusCode::NOT_FOUND => Ok(DirectoryAtTime::NoneAtTime),
        _ => Ok(DirectoryAtTime::Present(directory_state(
            crate::cdr::CdrClient::expect_success(response)?,
        ))),
    }
}

/// Read the sub-FOLDER at `path` of the current directory
/// (`GET /ehr/{ehr_id}/directory?path=…`, ITS-REST
/// `specifications/operations/directory_get_at_time.yaml` `path` parameter).
/// `200` → [`DirectorySubtree::Found`], `404` → [`DirectorySubtree::Missing`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty path; CDR transport errors pass
/// through; any other non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_directory_subtree(
    /// The EHR holding the directory.
    ehr_id: String,
    /// The folder path within the directory to read.
    path: String,
) -> Result<DirectorySubtree, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let path = path.trim();
    if path.is_empty() {
        return Err(ViewerError::Invalid("a path is required".to_owned()));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/directory?path={}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(path)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(DirectorySubtree::Missing);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(DirectorySubtree::Found(body))
}

/// Split an `OBJECT_VERSION_ID` into its `object_id::system` prefix and its
/// integer version tree number (`abc::sys::3` → `("abc::sys", 3)`). Returns
/// `None` for a branched (non-integer) version tree id.
#[cfg(feature = "ssr")]
fn split_version_uid(uid: &str) -> Option<(String, i32)> {
    let (prefix, version) = uid.rsplit_once("::")?;
    let number = version.parse::<i32>().ok()?;
    if prefix.is_empty() || number < 1 {
        return None;
    }
    Some((prefix.to_owned(), number))
}

/// Summarize a fetched directory FOLDER version body into a [`DirectoryVersion`].
#[cfg(feature = "ssr")]
fn summarize_directory_version(body: &str, number: i32, is_latest: bool) -> DirectoryVersion {
    let doc: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let version_uid = uid_value_of(body);
    let root_name = doc
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("(folder)")
        .to_owned();
    let (folder_count, item_count) = edit::count_tree(&doc);
    DirectoryVersion {
        version_uid,
        number,
        root_name,
        folder_count,
        item_count,
        body: body.to_owned(),
        is_latest,
    }
}

/// Directory tab: the toolbar (history / time / path / delete), the main
/// content (the structured tree editor for an existing directory, or the
/// create-empty section when the CDR 404s), and the history / time-travel /
/// path panels. The directory resource depends on every write action's version,
/// so a successful create/update/delete/restore refetches it. Every read
/// resource is created ONCE here, never inside a `Suspend`, and is gated on the
/// active tab plus its own trigger.
#[expect(
    clippy::too_many_lines,
    reason = "the tab's resources/actions/panels are wired as one unit; splitting would separate state from its wiring"
)]
pub(super) fn directory_section(ehr_id: Signal<String>, selected: Memo<String>) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();

    // Write actions. `restore` reuses the directory update wire but is a
    // distinct action so its toast reads "restored".
    let create = Action::new(|(ehr_id, body): &(String, String)| {
        let (ehr_id, body) = (ehr_id.clone(), body.clone());
        async move { create_directory(ehr_id, body).await }
    });
    let update = Action::new(|(ehr_id, uid, body): &(String, String, String)| {
        let (ehr_id, uid, body) = (ehr_id.clone(), uid.clone(), body.clone());
        async move { update_directory(ehr_id, uid, body).await }
    });
    let delete = Action::new(|(ehr_id, uid): &(String, String)| {
        let (ehr_id, uid) = (ehr_id.clone(), uid.clone());
        async move { delete_directory(ehr_id, uid).await }
    });
    let restore = Action::new(|(ehr_id, uid, body): &(String, String, String)| {
        let (ehr_id, uid, body) = (ehr_id.clone(), uid.clone(), body.clone());
        async move { update_directory(ehr_id, uid, body).await }
    });
    // Informed overwrite after a `412`: fetch the CURRENT latest version uid
    // and save the user's tree against it. Dispatched only from the conflict
    // banner's explicit "Save anyway" — the user is choosing to supersede the
    // concurrent change (blindly rebasing on every save would defeat the
    // lost-update protection If-Match exists for).
    let force_save = Action::new(|(ehr_id, body): &(String, String)| {
        let (ehr_id, body) = (ehr_id.clone(), body.clone());
        async move {
            match fetch_directory(ehr_id.clone()).await? {
                Some(current) => update_directory(ehr_id, current.version_uid, body).await,
                None => Err(ViewerError::Internal(
                    "the directory disappeared while resolving the conflict".to_owned(),
                )),
            }
        }
    });
    // An explicit user-driven reload ("discard my edits, load the server
    // version") — part of the directory resource's source.
    let reload = RwSignal::new(0u32);

    // The directory editor's long-lived reactive state, created ONCE here —
    // ABOVE the `<Transition>`/`Suspend` — and re-seeded idempotently per loaded
    // version. Signals created INSIDE the Suspend would die with it: a Suspend
    // re-runs on every resource notification and disposes the previous run's
    // owner, leaving mounted DOM handlers pointing at dead signals.
    let editor = EditorState::new(update);

    // Toast EVERY write outcome (outside-world side-effect; the viewer's
    // mutation-feedback rule): a `412` conflict gets the distinct "reload
    // or save anyway" toast, any other failure the shared actionable copy.
    // The CDR diagnostic ALSO stays inline in the relevant feedback pane,
    // beside the folder tree it refused.
    write_toast(
        toaster,
        create,
        "Directory created",
        "Create failed",
        directory_toast_detail,
    );
    write_toast(
        toaster,
        update,
        "Directory updated",
        "Save failed",
        directory_toast_detail,
    );
    write_toast(
        toaster,
        restore,
        "Directory restored",
        "Restore failed",
        directory_toast_detail,
    );
    write_toast(
        toaster,
        force_save,
        "Directory updated",
        "Save failed",
        directory_toast_detail,
    );
    delete_toast(toaster, delete);

    // A version bump on any SUCCESSFUL write is the shared refetch trigger.
    // `Action::version` increments on failures too; refetching on a failed
    // save (a `412` conflict, a validation reject) would re-seed the working
    // tree from the server and silently discard the user's unsaved edits.
    // Each stamp Memo therefore sticks to its previous value on a failed
    // completion (the Memo's `prev` parameter), so only completed writes
    // reload.
    let create_ok = completed_version(create);
    let update_ok = completed_version(update);
    let delete_ok = completed_version(delete);
    let restore_ok = completed_version(restore);
    let force_ok = completed_version(force_save);
    let write_version = Memo::new(move |_| {
        (
            create_ok.get(),
            update_ok.get(),
            delete_ok.get(),
            restore_ok.get(),
            force_ok.get(),
        )
    });

    let directory = Resource::new(
        move || {
            let versions = write_version.get();
            let requested = reload.get();
            (selected.get() == "directory").then(|| (ehr_id.get(), versions, requested))
        },
        |active| async move {
            match active {
                Some((id, _, _)) => fetch_directory(id).await,
                None => Ok(None),
            }
        },
    );

    // Composition picker source for "add item" — created here (outside the
    // Suspend), gated on the target being set, and read inside the editor.
    let picker_target = RwSignal::new(Option::<String>::None);
    let picker = Resource::new(
        move || picker_target.with(Option::is_some).then(|| ehr_id.get()),
        |active| async move {
            match active {
                Some(id) => crate::pages::ehr_detail::compositions::all_compositions(id, 0)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );

    // Panel open-state (client-only chrome) and their gated read resources.
    let history_open = RwSignal::new(false);
    let time_open = RwSignal::new(false);
    let path_open = RwSignal::new(false);
    let time_input = RwSignal::new(String::new());
    let path_input = RwSignal::new(String::new());

    // How much of the history the panel currently holds. It starts at one
    // viewer page and grows by another page per "load older" click, so the
    // initial load is bounded no matter how deep the version tree runs
    // (the read side of AMB-24 / #1490 — see `list_directory_versions`).
    let history_window = RwSignal::new(crate::components::data_table::PAGE_SIZE);
    let versions = Resource::new(
        move || {
            let refresh = write_version.get();
            let window = history_window.get();
            (selected.get() == "directory" && history_open.get())
                .then(|| (ehr_id.get(), refresh, window))
        },
        |active| async move {
            match active {
                Some((id, _, window)) => list_directory_versions(id, window).await,
                None => Ok(DirectoryHistory::default()),
            }
        },
    );
    let at_time = Resource::new(
        move || {
            let time = crate::format::datetime_local_to_rfc3339(&time_input.get());
            (time_open.get() && !time.is_empty()).then(|| (ehr_id.get(), time))
        },
        |active| async move {
            match active {
                Some((id, time)) => fetch_directory_at_time(id, time).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let at_path = Resource::new(
        move || {
            let path = path_input.get().trim().to_owned();
            (path_open.get() && !path.is_empty()).then(|| (ehr_id.get(), path))
        },
        |active| async move {
            match active {
                Some((id, path)) => fetch_directory_subtree(id, path).await.map(Some),
                None => Ok(None),
            }
        },
    );

    let toolbar = directory_toolbar(
        ehr_id,
        directory,
        delete,
        history_open,
        time_open,
        path_open,
    );
    let history = panels::history_panel(
        ehr_id,
        directory,
        versions,
        restore,
        history_open,
        history_window,
    );
    let time = panels::time_travel_panel(at_time, time_input, time_open);
    let path = panels::path_panel(at_path, path_input, path_open);

    // `<Transition>` (not `<Suspense>`): the directory resource reloads after
    // every write, and the old tree must stay visible instead of flashing the
    // skeleton (book async/12).
    let main = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match directory.await {
                    Ok(Some(state)) => {
                        match seed(&editor, &state, update) {
                            Ok(()) => {
                                tree_editor(
                                    &editor,
                                    ehr_id,
                                    update,
                                    force_save,
                                    reload,
                                    picker,
                                    picker_target,
                                )
                            }
                            Err(e) => crate::components::notice::inline_error(&e),
                        }
                    }
                    Ok(None) => create_section(ehr_id, create),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();

    view! { <div class="flex flex-col gap-4">{toolbar} {main} {history} {time} {path}</div> }
        .into_any()
}

/// The noun phrase every directory-write failure toast is built around.
const DIRECTORY_OBJECT: &str = "the EHR's directory";

/// Toast a write action's outcome: success with the shared detail formatter, a
/// `412` conflict with the distinct reload/save-anyway toast, and every other
/// failure with the shared actionable copy under `failure_title`. The inline
/// feedback pane keeps the verbatim diagnostic as well.
/// Toasts the directory deletion's outcome — the one write whose success
/// carries no version uid, so it does not fit [`write_toast`].
fn delete_toast<I: Send + Sync + 'static>(
    toaster: thaw::ToasterInjection,
    delete: Action<I, Result<(), ViewerError>>,
) {
    Effect::new(move |_| match delete.value().get() {
        Some(Ok(())) => crate::components::toast::toast_success(
            toaster,
            "Directory deleted",
            "The directory was deleted.",
        ),
        Some(Err(error)) if is_conflict(&error) => conflict_toast(toaster),
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(
                toaster,
                "Delete failed",
                DIRECTORY_OBJECT,
                &error,
            );
        }
        None => {}
    });
}

/// One write action's version stamp, frozen at its previous value unless the
/// last completion SUCCEEDED — the refetch trigger for the reads it affects.
///
/// `Action::version` increments on failures too; refetching on a failed save
/// would re-seed the working tree from the server and silently discard the
/// user's unsaved edits. The stamp therefore sticks to its previous value on a
/// failed completion, so only completed writes reload.
fn completed_version<I, O>(action: Action<I, Result<O, ViewerError>>) -> Memo<usize>
where
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    Memo::new(move |prev: Option<&usize>| {
        let version = action.version().get();
        if action.value().with(|v| matches!(v, Some(Ok(_)))) {
            version
        } else {
            prev.copied().unwrap_or(0)
        }
    })
}

fn write_toast<I: Send + Sync + 'static>(
    toaster: thaw::ToasterInjection,
    action: Action<I, Result<String, ViewerError>>,
    title: &'static str,
    failure_title: &'static str,
    detail: fn(&str) -> String,
) {
    Effect::new(move |_| match action.value().get() {
        Some(Ok(uid)) => crate::components::toast::toast_success(toaster, title, &detail(&uid)),
        Some(Err(error)) if is_conflict(&error) => conflict_toast(toaster),
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(toaster, failure_title, DIRECTORY_OBJECT, &error);
        }
        None => {}
    });
}

/// The shared optimistic-concurrency conflict toast. The user's unsaved
/// edits are deliberately kept (the refetch trigger ignores failed writes);
/// the conflict banner in the editor offers the explicit choices.
fn conflict_toast(toaster: thaw::ToasterInjection) {
    crate::components::toast::toast_error(
        toaster,
        "Directory changed on the server",
        "Your unsaved changes are kept. Load the server version or save anyway from the banner.",
    );
}

/// Convenience: the composition-picker resource type shared by the editor.
pub(crate) type PickerResource = Resource<Result<Option<ResultPage>, ViewerError>>;

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{
        DIRECTORY_ARCHETYPE, FOLDER_NODE_ID, directory_state, directory_toast_detail,
        empty_root_folder, folder_json, is_conflict, split_version_uid,
        summarize_directory_version, version_window,
    };
    use crate::error::ViewerError;

    /// A served directory answer carrying `etag` and a FOLDER body whose own
    /// `uid` deliberately names a DIFFERENT version, so the two are told apart.
    fn served_directory(etag: Option<&str>) -> crate::cdr::CdrResponse {
        crate::cdr::CdrResponse {
            status: http::StatusCode::OK,
            content_type: Some("application/json".to_owned()),
            etag: etag.map(str::to_owned),
            location: None,
            last_modified: None,
            preference_applied: None,
            body: r#"{"_type":"FOLDER","uid":{"value":"7d44::sys::1"}}"#.to_owned(),
        }
    }

    #[test]
    fn the_directory_precondition_is_the_served_etag_not_the_body_uid() {
        // The `ETag` is the identifier the CDR offers for the conditional
        // round-trip (ITS-REST overview §`ETag` and `Last-Modified`), so the
        // update/delete `If-Match` echoes exactly it.
        let state = directory_state(served_directory(Some("W/\"7d44::sys::4\"")));
        assert_eq!(state.version_uid, "7d44::sys::4");
        // The body still travels verbatim — it is the tree editor's base.
        assert!(state.body.contains("7d44::sys::1"));
        // A server that sent no ETag leaves the document's own uid.
        assert_eq!(
            directory_state(served_directory(None)).version_uid,
            "7d44::sys::1"
        );
    }

    #[test]
    fn folder_json_builds_a_spec_valid_folder_node() {
        let leaf = folder_json(FOLDER_NODE_ID, "2026", &[]);
        let root = folder_json(DIRECTORY_ARCHETYPE, "episodes", &[leaf]);
        assert_eq!(root["_type"], "FOLDER");
        assert_eq!(root["archetype_node_id"], DIRECTORY_ARCHETYPE);
        assert_eq!(root["name"]["value"], "episodes");
        assert_eq!(root["name"]["_type"], "DV_TEXT");
        assert_eq!(root["folders"][0]["name"]["value"], "2026");
        assert!(root.get("uid").is_none());
        assert!(root["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn empty_root_folder_is_an_empty_directory_root() {
        let root = empty_root_folder();
        assert_eq!(root["_type"], "FOLDER");
        assert_eq!(root["archetype_node_id"], DIRECTORY_ARCHETYPE);
        assert!(root["folders"].as_array().unwrap().is_empty());
    }

    #[test]
    fn directory_toast_detail_names_the_version_or_a_generic_line() {
        assert_eq!(
            directory_toast_detail("7d44::sys::1"),
            "New version 7d44::sys::1"
        );
        assert_eq!(directory_toast_detail(""), "The directory was committed.");
    }

    #[test]
    fn split_version_uid_extracts_prefix_and_integer_version() {
        assert_eq!(
            split_version_uid("abc::example.org::3"),
            Some(("abc::example.org".to_owned(), 3))
        );
        // A branched version tree id is not a plain integer.
        assert_eq!(split_version_uid("abc::example.org::1.0.2"), None);
        assert_eq!(split_version_uid("nouid"), None);
    }

    #[test]
    fn summarize_reads_name_counts_and_latest_flag() {
        let body = r#"{
            "_type":"FOLDER",
            "name":{"_type":"DV_TEXT","value":"root"},
            "uid":{"_type":"OBJECT_VERSION_ID","value":"abc::sys::2"},
            "folders":[{"_type":"FOLDER","name":{"_type":"DV_TEXT","value":"a"},"folders":[],"items":[]}],
            "items":[]
        }"#;
        let v = summarize_directory_version(body, 2, true);
        assert_eq!(v.version_uid, "abc::sys::2");
        assert_eq!(v.number, 2);
        assert_eq!(v.root_name, "root");
        assert_eq!(v.folder_count, 1);
        assert_eq!(v.item_count, 0);
        assert!(v.is_latest);
    }

    #[test]
    fn a_history_window_covers_the_newest_versions_only() {
        // The panel's first page over a deep history: 25 numbers, newest first,
        // and the walk never reaches v1 — which is the whole point (#2606).
        let window = version_window(400, 25);
        assert_eq!(window.len(), 25);
        assert_eq!(window.first(), Some(&400));
        assert_eq!(window.last(), Some(&376));
    }

    #[test]
    fn a_history_window_stops_at_the_first_version() {
        // A window wider than the history yields the whole history once, never
        // a synthesized `::0` uid.
        assert_eq!(version_window(3, 25), vec![3, 2, 1]);
        assert_eq!(version_window(1, 25), vec![1]);
        // A zero window would fetch nothing and hide the history entirely.
        assert_eq!(version_window(4, 0), vec![4]);
        // A window past i32 clamps instead of overflowing the version numbers.
        assert_eq!(version_window(2, u32::MAX), vec![2, 1]);
    }

    #[test]
    fn is_conflict_detects_412_only() {
        assert!(is_conflict(&ViewerError::Cdr {
            status: 412,
            message: "stale".to_owned()
        }));
        assert!(!is_conflict(&ViewerError::Cdr {
            status: 409,
            message: "x".to_owned()
        }));
        assert!(!is_conflict(&ViewerError::Invalid("x".to_owned())));
    }
}
