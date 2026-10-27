// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/ehrs/{ehr_id}/compositions/{uid}` screen — the composition viewer.
//!
//! A view of one COMPOSITION: a format toggle (canonical JSON/XML and the
//! Simplified FLAT/STRUCTURED renderings — the CDR converts, the BFF forwards
//! and pretty-prints), a version selector fed by the versioned object's
//! revision history, a per-version audit card, the `VERSIONED_COMPOSITION`
//! container + selected-VERSION envelope card, and the two write paths (commit
//! a new version, logically delete the latest one). The document resource is
//! keyed on `(version, format)` so either switch refetches, under a
//! `<Transition>`.
//!
//! One reader per claim: the document CONTENT comes from the COMPOSITION
//! resource (the only one that negotiates the simplified formats), the commit
//! history from the revision history, and the VERSION's own envelope facts from
//! the `VERSIONED_COMPOSITION` version read.
//!
//! No openEHR spec governs the viewer — our own design; the wire it reads is
//! spec-bound (`Accept` negotiation per
//! `docs/specs/openehr/ITS-REST/docs/simplified_formats/`). Every co-located
//! `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

#[cfg(not(target_arch = "wasm32"))]
use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

use crate::bff::wire::VersionEntry;
use crate::error::ViewerError;
use crate::format::ReprFormat;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::{Msg, tr_with};
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::{tr_served, tr_served_with};

/// The revision history of a versioned composition, newest-first.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the history is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_versions(
    /// The EHR holding the versioned composition.
    ehr_id: String,
    /// The `VERSIONED_COMPOSITION` uid whose revision history to read.
    uid: String,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/versioned_composition/{}/revision_history",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    // NOTE: REVISION_HISTORY.items arrives most-recent-LAST on the wire (RM
    // common, generic package — the `items` attribute documentation); the
    // selector presents newest-first, so reverse here.
    parse_versions(&response.body).map(|mut entries| {
        entries.reverse();
        entries
    })
}

/// One representation of a composition version, pretty-printed for display, or
/// `None` when the addressed version is deleted. `version_uid` may be the bare
/// versioned-object id (the latest version) or a full `OBJECT_VERSION_ID` (that
/// exact version).
///
/// The `204` branch is `composition_get`'s own declared answer for a version
/// deleted at the specified time
/// (`docs/specs/openehr/ITS-REST/specifications/responses/204_deleted_at_time.yaml`,
/// referenced from `operations/composition_get.yaml`), so it is a rendered
/// absence rather than a failure.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `406` for a declined
/// representation included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_composition(
    /// The EHR holding the composition.
    ehr_id: String,
    /// The version to read: a full `OBJECT_VERSION_ID`, or the object uid
    /// for the latest version.
    version_uid: String,
    /// Which representation to negotiate for the document body.
    format: ReprFormat,
) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&version_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, format.media_type())
        .await?;
    if response.is(http::StatusCode::NO_CONTENT) {
        return Ok(None);
    }
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(Some(crate::bff::pretty_body(&response.body, format)))
}

/// Resolve the `OBJECT_VERSION_ID` of the VERSION of a `VERSIONED_COMPOSITION`
/// that was extant at `at_time` (a browser `datetime-local` value):
/// `GET /ehr/{ehr}/versioned_composition/{uid}/version?version_at_time=…`
/// (ITS-REST `VERSIONED_COMPOSITION` API `versioned_composition_version_get_at_time`
/// — "if `version_at_time` is supplied, retrieves the VERSION extant at
/// specified time"). The 200 body is a VERSION envelope whose `uid.value` is
/// the `OBJECT_VERSION_ID` (RM common — a VERSION's `uid` is an
/// `OBJECT_VERSION_ID`); that string is returned so the caller can select it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when `at_time` is empty; CDR transport errors pass
/// through; a non-2xx CDR answer (a `404` for no version at that time included,
/// which the UI renders as an inline note) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_version_at_time(
    /// The EHR holding the versioned composition.
    ehr_id: String,
    /// The `VERSIONED_COMPOSITION` uid to read a version of.
    versioned_object_uid: String,
    /// The instant to resolve, as a `datetime-local` input value.
    at_time: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let at_time = crate::format::datetime_local_to_rfc3339(&at_time);
    if at_time.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::PickDateTimeTravel)));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/versioned_composition/{}/version?version_at_time={}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&versioned_object_uid),
        urlencoding::encode(&at_time),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    // The VERSION envelope's `uid.value` is the `OBJECT_VERSION_ID`.
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Commit a new version of a COMPOSITION
/// (`PUT /ehr/{ehr_id}/composition/{versioned_object_uid}`). `If-Match` carries
/// the CURRENT (latest) `version_uid` — the `preceding_version_uid` — so the
/// update is conditional (ITS-REST COMPOSITION API `composition_update`; the
/// header value is the `version_uid` enclosed in double quotes). Canonical
/// JSON body; `Prefer: return=representation` yields the new `uid.value`.
///
/// This is the one update whose precondition is NOT a served `ETag`: the
/// document pane may be showing an older version, so the value comes from the
/// revision history's newest entry instead — the `ETag` of the read that filled
/// the editor would name the wrong version.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body, a missing current version, or a
/// `412` mid-air collision (prefixed with a reload hint, the CDR diagnostic
/// appended); CDR transport errors pass through; any other non-2xx CDR answer
/// (its validation diagnostics included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_composition(
    /// The EHR holding the composition.
    ehr_id: String,
    /// The `VERSIONED_COMPOSITION` uid being updated.
    versioned_object_uid: String,
    /// The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    /// The replacement composition document, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::CompositionBodyEmpty)));
    }
    let current_version_uid = current_version_uid.trim();
    if current_version_uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::NoCurrentVersionKnownReloadPage,
        )));
    }
    // If-Match value is the version_uid in double quotes (composition_update).
    let if_match = format!("\"{current_version_uid}\"");
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&versioned_object_uid)
    ));
    let response = state
        .cdr
        .put(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[("Prefer", "return=representation"), ("If-Match", &if_match)],
            body,
        )
        .await?;
    // 412 Precondition Failed = a mid-air collision (the latest version moved
    // since this page loaded); give it a friendly prefix, the CDR diagnostic
    // appended verbatim.
    if response.is(http::StatusCode::PRECONDITION_FAILED) {
        let detail = crate::cdr::CdrClient::expect_success(response)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        return Err(ViewerError::Invalid(tr_served_with(
            Msg::CompositionChangedSincePageLoadedReload,
            &[("detail", detail.clone().into())],
        )));
    }
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Logically delete a COMPOSITION
/// (`DELETE /ehr/{ehr_id}/composition/{version_uid}`).
///
/// `version_uid` MUST be a full `OBJECT_VERSION_ID` — the path id is "in a form
/// of an OBJECT_VERSION_ID identifier taken from the last (most recent)
/// VERSION.uid.value, representing the `preceding_version_uid` to be deleted"
/// (`docs/specs/openehr/ITS-REST/specifications/operations/composition_delete.yaml`)
/// — so a bare versioned-object id is rejected here rather than sent. No
/// `If-Match` is sent: the operation declares none, and latest-ness is enforced
/// by its own `409`, "supplied `uid_based_id` doesn't match the latest version".
/// The success answer is `204`; an already-deleted version is `400`.
///
/// This is the openEHR LOGICAL delete: a new deleted-lifecycle version is
/// committed and the history stays readable.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when no full `OBJECT_VERSION_ID` is known; CDR
/// transport errors pass through; a non-2xx CDR answer (the `409` stale-uid
/// branch and the `400` for an already-deleted version included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_composition(
    /// The EHR holding the composition.
    ehr_id: String,
    /// The latest version's full `OBJECT_VERSION_ID`, which the delete requires.
    version_uid: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let version_uid = version_uid.trim();
    if version_uid.is_empty() || !version_uid.contains("::") {
        return Err(ViewerError::Invalid(tr_served(
            Msg::LatestVersionSFullObjectVersion,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/composition/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(version_uid)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

/// The `VERSIONED_COMPOSITION` container plus the selected VERSION's envelope
/// facts, flattened for the versioned-object card.
///
/// The attributes are the RM classes' own (files under
/// `docs/specs/openehr/RM/docs/UML/classes/`): `VERSIONED_OBJECT._uid_`,
/// `_owner_id_` and `_time_created_` (`org.openehr.rm.common.versioned_object.adoc`);
/// `VERSION._contribution_` and `_signature_` ("`OpenPGP` digital signature or
/// digest of content committed in this Version") plus
/// `_preceding_version_uid_`, whose invariant
/// `Preceding_version_uid_validity` makes it absent exactly for a first
/// version (`org.openehr.rm.common.version.adoc`); and
/// `ORIGINAL_VERSION._lifecycle_state_`
/// (`org.openehr.rm.common.original_version.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VersionedCompositionDetails {
    /// `VERSIONED_OBJECT.uid.value` — the versioned-object id.
    pub object_uid: String,
    /// `VERSIONED_OBJECT.owner_id.id.value` — the owning EHR.
    pub owner_id: String,
    /// `VERSIONED_OBJECT.time_created.value` — when the object's first version
    /// was committed.
    pub time_created: String,
    /// The read VERSION's `uid.value` (`OBJECT_VERSION_ID`).
    pub version_id: String,
    /// `ORIGINAL_VERSION.lifecycle_state.value` (the coded text's rubric).
    pub lifecycle_state: String,
    /// `VERSION.preceding_version_uid.value` — empty for a first version.
    pub preceding_version_uid: String,
    /// `VERSION.contribution.id.value` — the CONTRIBUTION this version was
    /// committed under.
    pub contribution_uid: String,
    /// Whether the VERSION carries a `signature`.
    pub signed: bool,
    /// Whether the VERSION carries `data` — a logically deleted version does
    /// not (`docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
    /// §Logical Deletion).
    pub has_data: bool,
}

/// Read the `VERSIONED_COMPOSITION` container and one of its VERSIONS.
///
/// Two reads, one resource: `GET /ehr/{ehr}/versioned_composition/{uid}` for the
/// container (`versioned_composition_get`) and the direct VERSION read for the
/// envelope — `…/version/{version_uid}` for an explicitly selected version
/// (`versioned_composition_version_get_by_id`) or `…/version` for the current
/// one when the selector says "Latest": with no `version_at_time`, that
/// operation "retrieves the _latest_ VERSION"
/// (`docs/specs/openehr/ITS-REST/specifications/operations/versioned_composition_version_get_at_time.yaml`
/// §description).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `404` for an unknown
/// object/version included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when either body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_versioned_composition(
    /// The EHR holding the versioned composition.
    ehr_id: String,
    /// The `VERSIONED_COMPOSITION` uid whose envelope facts to read.
    versioned_object_uid: String,
    /// The specific version within that versioned object.
    version_uid: String,
) -> Result<VersionedCompositionDetails, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let ehr = urlencoding::encode(&ehr_id);
    let object = urlencoding::encode(&versioned_object_uid);
    let object_url = state
        .cdr
        .rest_v1(&format!("ehr/{ehr}/versioned_composition/{object}"));
    let object_response = state
        .cdr
        .get(&session.credential, &object_url, "application/json")
        .await?;
    let object_body = crate::cdr::CdrClient::expect_success(object_response)?.body;

    let version_uid = version_uid.trim();
    let version_url = if version_uid.is_empty() {
        state
            .cdr
            .rest_v1(&format!("ehr/{ehr}/versioned_composition/{object}/version"))
    } else {
        state.cdr.rest_v1(&format!(
            "ehr/{ehr}/versioned_composition/{object}/version/{}",
            urlencoding::encode(version_uid)
        ))
    };
    let version_response = state
        .cdr
        .get(&session.credential, &version_url, "application/json")
        .await?;
    let version_body = crate::cdr::CdrClient::expect_success(version_response)?.body;
    parse_versioned_details(&object_body, &version_body)
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a `VERSIONED_COMPOSITION` body plus a VERSION body into
/// [`VersionedCompositionDetails`]. Defensive throughout — an absent attribute
/// reads as empty rather than failing the card.
///
/// # Errors
/// [`ViewerError::Internal`] when either body is not valid JSON.
fn parse_versioned_details(
    object_body: &str,
    version_body: &str,
) -> Result<VersionedCompositionDetails, ViewerError> {
    let object: Value = serde_json::from_str(object_body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::VersionedCompositionJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let version: Value = serde_json::from_str(version_body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::VersionJson, &[("e", (e).to_string().into())]))
    })?;
    Ok(VersionedCompositionDetails {
        object_uid: crate::uid::uid_value_of_document(&object),
        owner_id: json_str(&object, &["owner_id", "id", "value"]),
        time_created: json_str(&object, &["time_created", "value"]),
        version_id: crate::uid::uid_value_of_document(&version),
        lifecycle_state: json_str(&version, &["lifecycle_state", "value"]),
        preceding_version_uid: json_str(&version, &["preceding_version_uid", "value"]),
        contribution_uid: json_str(&version, &["contribution", "id", "value"]),
        signed: version
            .get("signature")
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.is_empty()),
        has_data: version.get("data").is_some_and(|data| !data.is_null()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Parse a `REVISION_HISTORY` body (either the canonical `{ "items": [...] }`
/// wrapper or a bare array) into version entries. Defensive throughout — a
/// missing field reads as empty rather than failing.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_versions(body: &str) -> Result<Vec<VersionEntry>, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::RevisionHistoryJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let items = doc
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| doc.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items.iter().map(version_entry).collect())
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten one `REVISION_HISTORY_ITEM` (`version_id` + `audits`) into a
/// [`VersionEntry`], reading the first audit for the commit metadata.
fn version_entry(item: &Value) -> VersionEntry {
    let version_id = json_str(item, &["version_id", "value"]);
    let audit = item
        .get("audits")
        .and_then(Value::as_array)
        .and_then(|audits| audits.first());
    VersionEntry {
        version_id,
        committed: audit
            .map(|a| json_str(a, &["time_committed", "value"]))
            .unwrap_or_default(),
        change_type: audit
            .map(|a| json_str(a, &["change_type", "value"]))
            .unwrap_or_default(),
        committer: audit
            .map(|a| json_str(a, &["committer", "name"]))
            .unwrap_or_default(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Follow a chain of object keys and return the terminal string value, or an
/// empty string if any hop is missing / not a string.
fn json_str(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_owned()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{parse_versioned_details, parse_versions};

    /// A `VERSIONED_COMPOSITION` container body, as the wire carries it.
    const VERSIONED_OBJECT: &str = r#"{
        "_type": "VERSIONED_COMPOSITION",
        "uid": {"_type": "HIER_OBJECT_ID", "value": "7d44aa01"},
        "owner_id": {
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "EHR",
            "id": {"_type": "HIER_OBJECT_ID", "value": "e1"}
        },
        "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
    }"#;

    #[test]
    fn parses_the_versioned_object_and_its_version_envelope() {
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::2"},
            "contribution": {
                "_type": "OBJECT_REF",
                "id": {"_type": "HIER_OBJECT_ID", "value": "c9"}
            },
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "complete"},
            "preceding_version_uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::1"},
            "signature": "-----BEGIN PGP SIGNATURE-----",
            "data": {"_type": "COMPOSITION"}
        }"#;
        let details = parse_versioned_details(VERSIONED_OBJECT, version).expect("valid bodies");
        assert_eq!(details.object_uid, "7d44aa01");
        assert_eq!(details.owner_id, "e1");
        assert_eq!(details.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(details.version_id, "7d44aa01::sys::2");
        assert_eq!(details.lifecycle_state, "complete");
        assert_eq!(details.preceding_version_uid, "7d44aa01::sys::1");
        assert_eq!(details.contribution_uid, "c9");
        assert!(details.signed);
        assert!(details.has_data);
    }

    #[test]
    fn a_deleted_first_version_carries_no_data_no_preceding_and_no_signature() {
        // RM common master06 §Logical Deletion: the deleted version has no
        // `data`; a first version has no `preceding_version_uid`.
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::sys::1"},
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "deleted"}
        }"#;
        let details = parse_versioned_details(VERSIONED_OBJECT, version).expect("valid bodies");
        assert_eq!(details.lifecycle_state, "deleted");
        assert_eq!(details.preceding_version_uid, "");
        assert_eq!(details.contribution_uid, "");
        assert!(!details.signed);
        assert!(!details.has_data);
    }

    #[test]
    fn versioned_details_reject_a_non_json_body_on_either_side() {
        assert!(parse_versioned_details("not json", "{}").is_err());
        assert!(parse_versioned_details(VERSIONED_OBJECT, "not json").is_err());
    }

    #[test]
    fn parses_revision_history_wrapper_and_bare_array() {
        let wrapper = r#"{
            "items": [{
                "version_id": {"value": "7d44::sys::2"},
                "audits": [{
                    "time_committed": {"value": "2026-07-12T10:00:00Z"},
                    "change_type": {"value": "creation"},
                    "committer": {"name": "Dr Bob"}
                }]
            }]
        }"#;
        let entries = parse_versions(wrapper).expect("valid history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version_id, "7d44::sys::2");
        assert_eq!(entries[0].committed, "2026-07-12T10:00:00Z");
        assert_eq!(entries[0].change_type, "creation");
        assert_eq!(entries[0].committer, "Dr Bob");

        // A bare array (no wrapper) parses identically.
        let bare = r#"[{"version_id":{"value":"a::b::1"},"audits":[]}]"#;
        let entries = parse_versions(bare).expect("valid bare history");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version_id, "a::b::1");
        assert_eq!(entries[0].committed, "");
    }
}
