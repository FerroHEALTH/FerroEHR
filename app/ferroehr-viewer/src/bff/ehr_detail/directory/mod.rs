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
//! `master05-directory_package`. Every read below authenticates the
//! viewer session first and the CDR credential never reaches client-visible
//! state.
//!
//! This module owns the DIRECTORY reads and writes, the shared wire types, and
//! the `directory_section` orchestrator; the view is split across
//! `tree` (the structured editor), `panels` (history / time / path /
//! delete), `create` (the empty-directory create flow), and the pure
//! [`edit`] helpers.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

pub mod edit;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::uid::uid_value_of;

use crate::error::ViewerError;
use crate::locale::records::RecordsMsg;
use crate::locale::tr_served;

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

/// The EHR's directory (root `FOLDER`) as a [`DirectoryState`], or `None` when
/// the CDR has no directory for this EHR (a `404` is a first-class empty
/// state, not an error).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_directory(
    // The EHR whose directory to read.
    ehr_id: String,
) -> Result<Option<DirectoryState>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
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
/// JSON FOLDER `body`.
///
/// `Content-Type: application/json`, `Prefer: return=representation`, `Accept:
/// application/json`; the new version uid (`uid.value` of the returned FOLDER)
/// is returned (empty when the CDR returned no representation). Directory
/// operations per ITS-REST `specifications/operations/directory_create.yaml`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body; CDR transport errors pass
/// through; a non-2xx CDR answer (its validation diagnostics, which the UI
/// renders verbatim, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn create_directory(
    // The EHR to create the directory in.
    ehr_id: String,
    // The root FOLDER document to commit, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::DirectoryBodyEmpty,
        )));
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
/// canonical JSON FOLDER `body`.
///
/// The version the seeding read's `ETag` named (the `preceding_version_uid`) is
/// sent quoted in `If-Match`, per ITS-REST
/// `specifications/operations/directory_update.yaml`; `Prefer:
/// return=representation` yields the new version uid. A stale uid is answered
/// `412` by the CDR and surfaces as `is_conflict`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body or missing current version uid;
/// CDR transport errors pass through; a non-2xx CDR answer (its validation
/// diagnostics, which the UI renders verbatim, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn update_directory(
    // The EHR holding the directory.
    ehr_id: String,
    // The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    // The replacement root FOLDER document, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::DirectoryBodyEmpty,
        )));
    }
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::CurrentDirectoryVersionUidRequiredUpdate,
        )));
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

/// Delete the EHR's directory (`DELETE /ehr/{ehr_id}/directory`).
///
/// The version the seeding read's `ETag` named (the `preceding_version_uid`) is
/// sent quoted in `If-Match`, per ITS-REST
/// `specifications/operations/directory_delete.yaml`; a stale uid is answered
/// `412` and surfaces as `is_conflict`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a missing current version uid; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn delete_directory(
    // The EHR holding the directory.
    ehr_id: String,
    // The version being deleted, sent as `If-Match`.
    current_version_uid: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::CurrentDirectoryVersionUidRequiredDelete,
        )));
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
/// NOTE: ITS-REST 1.1.0 exposes no `VERSIONED_FOLDER` revision history (register
/// AMB-24, upstream report #1490), so the uid list is synthesized from the
/// latest version number rather than read from the CDR.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx, non-404 CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn list_directory_versions(
    // The EHR whose directory version list to read.
    ehr_id: String,
    // How many of the newest versions to read.
    window: u32,
) -> Result<DirectoryHistory, ViewerError> {
    use futures::stream::{StreamExt, TryStreamExt};

    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
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
fn version_window(latest_n: i32, window: u32) -> Vec<i32> {
    let window = i32::try_from(window).unwrap_or(i32::MAX).max(1);
    let oldest = latest_n.saturating_sub(window).saturating_add(1).max(1);
    (oldest..=latest_n).rev().collect()
}

/// Read the directory FOLDER as it stood at `version_at_time` (`GET
/// /ehr/{ehr_id}/directory?version_at_time=…`, ITS-REST
/// `specifications/operations/directory_get_at_time.yaml`).
///
/// `200` → [`DirectoryAtTime::Present`], `204` →
/// [`DirectoryAtTime::DeletedAtTime`], `404` → [`DirectoryAtTime::NoneAtTime`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty time; CDR transport errors pass
/// through; any other non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_directory_at_time(
    // The EHR holding the directory.
    ehr_id: String,
    // The instant to resolve the directory at.
    time: String,
) -> Result<DirectoryAtTime, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let time = time.trim();
    if time.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::DateTimeRequired,
        )));
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
///
/// `200` → [`DirectorySubtree::Found`], `404` → [`DirectorySubtree::Missing`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty path; CDR transport errors pass
/// through; any other non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn fetch_directory_subtree(
    // The EHR holding the directory.
    ehr_id: String,
    // The folder path within the directory to read.
    path: String,
) -> Result<DirectorySubtree, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let path = path.trim();
    if path.is_empty() {
        return Err(ViewerError::Invalid(tr_served(RecordsMsg::APathIsRequired)));
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
fn split_version_uid(uid: &str) -> Option<(String, i32)> {
    let (prefix, version) = uid.rsplit_once("::")?;
    let number = version.parse::<i32>().ok()?;
    if prefix.is_empty() || number < 1 {
        return None;
    }
    Some((prefix.to_owned(), number))
}

/// Summarize a fetched directory FOLDER version body into a [`DirectoryVersion`].
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

#[cfg(test)]
mod tests {
    use super::{
        DIRECTORY_ARCHETYPE, FOLDER_NODE_ID, directory_state, empty_root_folder, folder_json,
        split_version_uid, summarize_directory_version, version_window,
    };

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
        assert_eq!(root["items"], serde_json::json!([]));
    }

    #[test]
    fn empty_root_folder_is_an_empty_directory_root() {
        let root = empty_root_folder();
        assert_eq!(root["_type"], "FOLDER");
        assert_eq!(root["archetype_node_id"], DIRECTORY_ARCHETYPE);
        assert_eq!(root["folders"], serde_json::json!([]));
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
}
