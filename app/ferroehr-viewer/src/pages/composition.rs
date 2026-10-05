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

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssr")]
use serde_json::Value;

use crate::components::empty_state::EmptyState;
use crate::components::field::{
    BTN_DANGER, BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL, SELECT, TEXTAREA,
};
use crate::components::format_view::{DocumentPane, FormatSelector, PaneView};
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::components::toast::{toast_error, toast_success};
use crate::components::wire::VersionEntry;
// Server-side pretty-printing happens in the #[server] body only.
#[cfg(feature = "ssr")]
use crate::components::format_view::pretty_body;
use crate::error::ViewerError;
use crate::format::ReprFormat;

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
    Ok(Some(pretty_body(&response.body, format)))
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
        return Err(ViewerError::Invalid(
            "pick a date and time to travel to".to_owned(),
        ));
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
        return Err(ViewerError::Invalid(
            "the composition body is empty".to_owned(),
        ));
    }
    let current_version_uid = current_version_uid.trim();
    if current_version_uid.is_empty() {
        return Err(ViewerError::Invalid(
            "no current version is known — reload the page and retry".to_owned(),
        ));
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
        return Err(ViewerError::Invalid(format!(
            "the composition changed since this page loaded — reload and retry. {detail}"
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
        return Err(ViewerError::Invalid(
            "the latest version's full OBJECT_VERSION_ID is required to delete a composition — \
             reload this screen and retry"
                .to_owned(),
        ));
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

#[cfg(feature = "ssr")]
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
    let object: Value = serde_json::from_str(object_body)
        .map_err(|e| ViewerError::Internal(format!("versioned composition JSON: {e}")))?;
    let version: Value = serde_json::from_str(version_body)
        .map_err(|e| ViewerError::Internal(format!("version JSON: {e}")))?;
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

#[cfg(feature = "ssr")]
/// Parse a `REVISION_HISTORY` body (either the canonical `{ "items": [...] }`
/// wrapper or a bare array) into version entries. Defensive throughout — a
/// missing field reads as empty rather than failing.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_versions(body: &str) -> Result<Vec<VersionEntry>, ViewerError> {
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("revision history JSON: {e}")))?;
    let items = doc
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| doc.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items.iter().map(version_entry).collect())
}

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
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

/// The document pane's OPENING mode, from `?view=` — how the compositions
/// tab's rows link straight to the rendered clinical reading.
///
/// Read UNTRACKED at setup: the pane's own tabs own the mode from then on, and
/// an effect chasing the address bar would be the signal-writes-signal shape
/// the reactivity rules forbid. Untracked is sound because every way of
/// arriving with a `?view=` is a fresh render of this route. Anything that adds
/// an in-app link toggling `?view=` on THIS path must make the pane's mode
/// reactive instead.
fn pane_view_from_url() -> PaneView {
    leptos_router::hooks::use_query_map()
        .with_untracked(|q| q.get("view"))
        .as_deref()
        .and_then(PaneView::from_param)
        .unwrap_or_default()
}

/// Toasts the commit's outcome.
///
/// Both outcomes toast; the resources refetch via `update.version()` in their
/// own sources, and the CDR's diagnostic also stays inline in the editor.
fn wire_commit_toast<I: Send + Sync + 'static>(
    toaster: thaw::ToasterInjection,
    update: Action<I, Result<String, ViewerError>>,
) {
    Effect::new(move |_| match update.value().get() {
        Some(Ok(uid)) => {
            let detail = if uid.is_empty() {
                "A new version was committed.".to_owned()
            } else {
                format!("New version {uid}")
            };
            toast_success(toaster, "New version committed", &detail);
        }
        Some(Err(error)) => crate::feedback::toast_write_failure(
            toaster,
            "Commit failed",
            "the new composition version",
            &error,
        ),
        None => {}
    });
}

/// Toasts the logical delete's outcome and leaves the screen on success.
///
/// A success returns to the EHR's compositions tab, whose list then reloads
/// with the deleted composition gone. Navigation is an outside-world
/// side-effect, so an `Effect` is its correct home; it never runs on the
/// server.
fn wire_delete_outcome<I: Send + Sync + 'static>(
    toaster: thaw::ToasterInjection,
    delete: Action<I, Result<(), ViewerError>>,
    ehr_id: Signal<String>,
) {
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match delete.value().get() {
        Some(Ok(())) => {
            toast_success(
                toaster,
                "Composition deleted",
                "The composition was logically deleted — its version history stays readable.",
            );
            // The route param is percent-encoded through the shared href
            // builder — all percent-coding goes through `urlencoding`.
            navigate(
                &format!(
                    "{}?tab=compositions",
                    crate::pages::ehrs::ehr_detail_href(&ehr_id.get_untracked())
                ),
                leptos_router::NavigateOptions::default(),
            );
        }
        Some(Err(error)) => toast_error(
            toaster,
            "Delete failed",
            &crate::feedback::logical_delete_failure_copy("this composition", &error),
        ),
        None => {}
    });
}

/// The composition viewer screen.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "resource/action setup plus the erased section locals — one screen, one function (rules §1)"
)]
pub fn CompositionPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let ehr_id = Signal::derive(move || params.with(|p| p.get("ehr_id").unwrap_or_default()));
    let uid = Signal::derive(move || params.with(|p| p.get("uid").unwrap_or_default()));

    let format = RwSignal::new(ReprFormat::CanonicalJson);
    let initial_view = pane_view_from_url();
    // Empty = "latest" (fetch by the bare versioned-object id); a non-empty
    // value is a specific OBJECT_VERSION_ID.
    let selected_version = RwSignal::new(String::new());

    // The version_at_time picker: a `datetime-local` value resolves server-side
    // to the VERSION extant at that instant; on success its OBJECT_VERSION_ID
    // becomes the shared `selected_version` and the document pane refetches. A
    // `404` (no version at that time) stays an inline note in the toolbar.
    let at_time_input = RwSignal::new(String::new());
    let version_at_time = Action::new(
        move |(ehr_id, versioned_object_uid, at_time): &(String, String, String)| {
            let ehr_id = ehr_id.clone();
            let versioned_object_uid = versioned_object_uid.clone();
            let at_time = at_time.clone();
            async move {
                let resolved = fetch_version_at_time(ehr_id, versioned_object_uid, at_time).await;
                // The write rides the dispatched event's own continuation: an
                // event write, not an Effect write.
                if let Ok(version) = &resolved {
                    selected_version.set(version.clone());
                }
                resolved
            }
        },
    );

    // The "Edit as new version" affordance state and its commit action, created
    // before the resources so its `version()` can trigger their refetch.
    let edit_open = RwSignal::new(false);
    let editor_body = RwSignal::new(String::new());
    let update = Action::new(
        |(ehr_id, versioned_object_uid, current_version_uid, body): &(
            String,
            String,
            String,
            String,
        )| {
            let ehr_id = ehr_id.clone();
            let versioned_object_uid = versioned_object_uid.clone();
            let current_version_uid = current_version_uid.clone();
            let body = body.clone();
            async move {
                update_composition(ehr_id, versioned_object_uid, current_version_uid, body).await
            }
        },
    );

    // The logical-delete action + its confirmation state, created before the
    // resources so its `version()` is a refetch trigger.
    let confirming_delete = RwSignal::new(false);
    let delete = Action::new(|(ehr_id, version_uid): &(String, String)| {
        let ehr_id = ehr_id.clone();
        let version_uid = version_uid.clone();
        async move { delete_composition(ehr_id, version_uid).await }
    });

    let versions = Resource::new(
        move || (ehr_id.get(), uid.get(), update.version().get()),
        |(ehr_id, uid, _)| async move { fetch_versions(ehr_id, uid).await },
    );
    let versioned = Resource::new(
        move || {
            (
                ehr_id.get(),
                uid.get(),
                selected_version.get(),
                update.version().get(),
            )
        },
        |(ehr_id, uid, version_uid, _)| async move {
            fetch_versioned_composition(ehr_id, uid, version_uid).await
        },
    );
    let document = Resource::new(
        move || {
            let chosen = selected_version.get();
            let version_uid = if chosen.is_empty() { uid.get() } else { chosen };
            (
                ehr_id.get(),
                version_uid,
                format.get(),
                update.version().get(),
            )
        },
        |(ehr_id, version_uid, format, _)| async move {
            fetch_composition(ehr_id, version_uid, format).await
        },
    );

    // Both outcomes toast; the resources refetch via `update.version()` in
    // their sources above, and the CDR's diagnostic also stays inline in the
    // editor, next to the edited body.
    let toaster = thaw::ToasterInjection::expect_context();
    wire_commit_toast(toaster, update);
    wire_delete_outcome(toaster, delete, ehr_id);

    let toolbar = toolbar_section(
        format,
        versions,
        selected_version,
        ehr_id,
        uid,
        version_at_time,
        at_time_input,
    );
    let body = document_section(document, initial_view);
    let edit = edit_section(
        ehr_id,
        uid,
        format,
        versions,
        document,
        edit_open,
        editor_body,
        update,
    );
    let timeline = timeline_section(versions, selected_version);
    let audit = audit_section(versions, selected_version);
    let versioned_card = versioned_section(versioned);
    let delete_action = delete_section(ehr_id, versions, confirming_delete, delete);
    // The tag panel edits whichever collection the viewer is showing: the
    // VERSIONED_COMPOSITION container on "Latest", that one version once it is
    // pinned. The two are disjoint on the wire, so the addressed id is derived
    // from the same selection the document pane uses.
    let tags = crate::pages::ehr_tags::composition_tags_section(
        ehr_id,
        Signal::derive(move || {
            let chosen = selected_version.get();
            if chosen.is_empty() { uid.get() } else { chosen }
        }),
    );

    let title = Signal::derive(move || {
        let short: String = uid.get().chars().take(8).collect();
        format!("Composition {short}…")
    });

    // The parent-EHR crumb href/label reads the route param once at setup (a
    // composition route instance keeps its `ehr_id` for its lifetime).
    let ehr_id_value = ehr_id.get_untracked();
    let ehr_short: String = ehr_id_value.chars().take(8).collect();
    let crumbs = vec![
        Crumb::new("EHRs", "/ehrs"),
        Crumb::new(format!("EHR {ehr_short}…"), format!("/ehrs/{ehr_id_value}")),
    ];

    view! {
        <Title text="Composition" />
        <div class="p-6">
            <PageHeader title=Signal::derive(move || title.get()) crumbs=crumbs mono=true />
            {delete_action}
            {toolbar}
            {body}
            {edit}
            {timeline}
            {audit}
            {versioned_card}
            <div class="mt-4">{tags}</div>
        </div>
    }
}

/// The **Delete composition** affordance: the openEHR logical delete of the
/// LATEST version, behind the shared confirmation modal
/// ([`ConfirmDialog`](crate::components::confirm_dialog::ConfirmDialog)) whose
/// copy says what a logical delete does and does not destroy.
///
/// Not admin-gated: this is the ordinary COMPOSITION API operation every
/// openEHR client has, unlike the physical EHR delete on the EHR-detail screen.
/// The version it deletes is always the newest one the revision history reports
/// (the `preceding_version_uid` the spec requires in the path), never the
/// version the selector happens to be showing.
fn delete_section(
    ehr_id: Signal<String>,
    versions: Resource<Result<Vec<VersionEntry>, ViewerError>>,
    confirming: RwSignal<bool>,
    delete: Action<(String, String), Result<(), ViewerError>>,
) -> AnyView {
    // Reading a resource in an EVENT HANDLER is untracked — it takes the value
    // already loaded for the version selector. Deliberately not read into the
    // dialog copy: a resource read in a rendered signal would differ between the
    // server pass and hydration, so the copy names the object structurally.
    let latest = move || {
        versions
            .get()
            .and_then(Result::ok)
            .and_then(|entries| entries.first().map(|entry| entry.version_id.clone()))
            .unwrap_or_default()
    };
    let message = Signal::derive(|| {
        "Logically delete the latest version of this composition? The CDR commits a deleted \
         version on top of it: the composition stops resolving as current, while every earlier \
         version and its audit trail stay readable. It cannot be undone."
            .to_owned()
    });
    view! {
        <div class="mb-4 flex flex-wrap items-center justify-end gap-3">
            <span class="text-xs text-ink-muted">"Deletes the latest version."</span>
            <button
                id="composition-delete"
                type="button"
                class=BTN_DANGER
                disabled=Signal::derive(move || delete.pending().get())
                on:click=move |_| confirming.set(true)
            >
                <leptos_icons::Icon icon=icondata_lu::LuTrash width="14" height="14" />
                "Delete composition"
            </button>
            <crate::components::confirm_dialog::ConfirmDialog
                open=confirming
                title="Delete composition"
                message=message
                confirm_label="Delete composition"
                confirm_id="composition-delete-confirm"
                on_cancel=Callback::new(move |()| confirming.set(false))
                on_confirm=Callback::new(move |()| {
                    delete.dispatch((ehr_id.get_untracked(), latest()));
                    confirming.set(false);
                })
            />
        </div>
    }
    .into_any()
}

/// The versioned-object card: the `VERSIONED_COMPOSITION` container's own facts
/// (uid, owning EHR, first-version time) plus the SELECTED version's envelope
/// facts read directly from the VERSION resource — lifecycle state, preceding
/// version, contribution, whether it is signed, and whether it still carries
/// data (a logically deleted version does not).
/// A pure read under a `<Transition>` so the previous version's facts stay
/// visible while another version loads, resolving its `Result` INSIDE the
/// transition: a failure renders inline where the data would be, never through
/// an `<ErrorBoundary>`, whose SSR'd fallback mismatches at hydration in
/// leptos 0.8.
fn versioned_section(
    versioned: Resource<Result<VersionedCompositionDetails, ViewerError>>,
) -> AnyView {
    view! {
        <div class="mt-3">
            <Transition fallback=|| {
                view! {
                    <thaw::Skeleton>
                        <thaw::SkeletonItem class="h-24" />
                    </thaw::Skeleton>
                }
            }>
                {move || Suspend::new(async move {
                    match versioned.await {
                        Ok(details) => versioned_card(&details),
                        Err(e) => crate::components::notice::inline_error(&e),
                    }
                })}
            </Transition>
        </div>
    }
    .into_any()
}

/// Render the versioned-object + version-envelope facts as a card.
fn versioned_card(details: &VersionedCompositionDetails) -> AnyView {
    let content = if details.has_data {
        "present".to_owned()
    } else {
        "none (deleted version)".to_owned()
    };
    let signature = if details.signed {
        "present".to_owned()
    } else {
        "none".to_owned()
    };
    view! {
        <section class=CARD_PAD id="versioned-composition">
            <h2 class=CARD_TITLE>"Versioned object"</h2>
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {versioned_row("versioned object", "object-uid", details.object_uid.clone())}
                {versioned_row("owner EHR", "owner", details.owner_id.clone())}
                {versioned_row("created", "created", details.time_created.clone())}
                {versioned_row("version", "version", details.version_id.clone())}
                {versioned_row("lifecycle", "lifecycle", details.lifecycle_state.clone())}
                {versioned_row(
                    "preceding version",
                    "preceding",
                    details.preceding_version_uid.clone(),
                )} {versioned_row("contribution", "contribution", details.contribution_uid.clone())}
                {versioned_row("signature", "signature", signature)}
                {versioned_row("content", "content", content)}
            </div>
        </section>
    }
    .into_any()
}

/// One label/value line of the versioned-object card. `hook` is the row's
/// `data-versioned-fact` value — the stable E2E hook; an absent value shows an
/// em dash.
fn versioned_row(label: &'static str, hook: &'static str, value: String) -> AnyView {
    let shown = if value.is_empty() {
        "—".to_owned()
    } else {
        value
    };
    view! {
        <div>
            <span class="font-medium text-ink-muted mr-1">{label}":"</span>
            <span class="font-mono break-all text-ink" data-versioned-fact=hook>
                {shown}
            </span>
        </div>
    }
    .into_any()
}

/// The "Edit as new version" affordance: a toggle button opening a
/// prefilled-from-the-current-document editor that PUTs a new version. Editing
/// is offered only when the current format is canonical JSON (the other formats
/// show a switch hint). The `If-Match` always targets the NEWEST version (a
/// muted hint says so), never the version selected in the dropdown. Structure
/// is constant (visibility toggled with `class:hidden`) so server and client
/// views match.
#[expect(
    clippy::too_many_arguments,
    reason = "the section wires several page-level signals + two resources"
)]
fn edit_section(
    ehr_id: Signal<String>,
    uid: Signal<String>,
    format: RwSignal<ReprFormat>,
    versions: Resource<Result<Vec<VersionEntry>, ViewerError>>,
    document: Resource<Result<Option<String>, ViewerError>>,
    edit_open: RwSignal<bool>,
    editor_body: RwSignal<String>,
    update: Action<(String, String, String, String), Result<String, ViewerError>>,
) -> AnyView {
    let is_json = move || format.get() == ReprFormat::CanonicalJson;
    // Opening the editor prefills the textarea from the currently displayed
    // document (canonical JSON only). Reading a resource in an event handler
    // is untracked — it takes the value already loaded for the pane.
    let toggle = move |_| {
        let opening = !edit_open.get();
        if opening
            && format.get() == ReprFormat::CanonicalJson
            && let Some(Ok(Some(current))) = document.get()
        {
            editor_body.set(current);
        }
        edit_open.set(opening);
    };
    let commit_toaster = thaw::ToasterInjection::expect_context();
    let on_commit = move |_| {
        // If-Match the NEWEST version (entries are newest-first), regardless of
        // which version the dropdown shows. An unresolved history refuses the
        // click loudly — dispatching an empty precondition would surface as a
        // generic invalid-input failure instead of the real reason.
        let Some(current) = versions
            .get()
            .and_then(Result::ok)
            .and_then(|entries| entries.first().map(|entry| entry.version_id.clone()))
        else {
            toast_error(
                commit_toaster,
                "Version history still loading",
                "The commit needs the latest version id as its precondition — try again in a moment.",
            );
            return;
        };
        update.dispatch((ehr_id.get(), uid.get(), current, editor_body.get()));
    };
    view! {
        <div class="mt-3">
            <button id="edit-new-version" type="button" class=BTN_SECONDARY on:click=toggle>
                {move || if edit_open.get() { "Close editor" } else { "Edit as new version" }}
            </button>
            <div class:hidden=move || !edit_open.get()>
                <section class=format!("{CARD_PAD} mt-3")>
                    <h2 class=CARD_TITLE>"Commit new version"</h2>
                    <p class="mb-2 text-xs text-ink-muted">
                        "Commits on top of the latest version."
                    </p>
                    <div class:hidden=move || is_json()>
                        <p class="text-sm text-ink-muted">"Switch to canonical JSON to edit."</p>
                    </div>
                    <div class:hidden=move || !is_json() class="flex flex-col gap-3">
                        <textarea
                            id="edit-body"
                            class=format!("{TEXTAREA} min-h-[16rem]")
                            placeholder="edit the composition document…"
                            prop:value=move || editor_body.get()
                            on:input:target=move |ev| editor_body.set(ev.target().value())
                        >
                            {editor_body.get_untracked()}
                        </textarea>
                        <div class="flex items-center gap-3">
                            <button
                                id="edit-commit"
                                type="button"
                                class=BTN_PRIMARY
                                disabled=Signal::derive(move || update.pending().get())
                                on:click=on_commit
                            >
                                "Commit new version"
                            </button>
                            <Show when=move || update.pending().get()>
                                <span class="text-sm text-ink-muted">"Committing…"</span>
                            </Show>
                        </div>
                        {move || match update.value().get() {
                            Some(Err(error)) => {
                                view! {
                                    <div class=WELL>
                                        <pre class="overflow-auto max-h-[40vh] whitespace-pre-wrap font-mono text-xs text-danger">
                                            {error.to_string()}
                                        </pre>
                                    </div>
                                }
                                    .into_any()
                            }
                            _ => ().into_any(),
                        }}
                    </div>
                </section>
            </div>
        </div>
    }
    .into_any()
}

/// The toolbar: the shared [`FormatSelector`], the version `<select>`
/// (populated from the revision history under `<Suspense>`), and the
/// `version_at_time` picker (a `datetime-local` input + an "At time" button that
/// resolves the version extant at that instant into the shared selection).
fn toolbar_section(
    format: RwSignal<ReprFormat>,
    versions: Resource<Result<Vec<VersionEntry>, ViewerError>>,
    selected_version: RwSignal<String>,
    ehr_id: Signal<String>,
    uid: Signal<String>,
    version_at_time: Action<(String, String, String), Result<String, ViewerError>>,
    at_time_input: RwSignal<String>,
) -> AnyView {
    let offered = vec![
        ReprFormat::CanonicalJson,
        ReprFormat::CanonicalXml,
        ReprFormat::Flat,
        ReprFormat::Structured,
    ];
    let select = view! {
        <Suspense fallback=|| {
            view! { <thaw::Spinner size=thaw::SpinnerSize::Tiny /> }
        }>
            {move || Suspend::new(async move {
                match versions.await {
                    Ok(entries) => version_select(entries, selected_version),
                    Err(_) => {
                        // Resolve inside the Suspense: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! { <span class="text-xs text-danger">"versions unavailable"</span> }
                            .into_any()
                    }
                }
            })}
        </Suspense>
    }
    .into_any();
    // Dispatch the at-time resolution (server-fn does the RFC 3339 completion);
    // skip an empty input so no needless round-trip is made.
    let on_at_time = move |_| {
        let at_time = at_time_input.get();
        if !at_time.trim().is_empty() {
            version_at_time.dispatch((ehr_id.get(), uid.get(), at_time));
        }
    };
    // A 404 (no version at that time) is a neutral note; any other failure
    // renders through the normal inline-error path. Deliberately not an
    // `EmptyState`: this answers the time-travel control beside it, not a data
    // region that came back empty.
    let note = move || match version_at_time.value().get() {
        Some(Err(ref e)) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => view! {
            <p class="mt-2 text-sm text-ink-muted">
                "No version of this composition existed at that time."
            </p>
        }
        .into_any(),
        Some(Err(error)) => crate::components::notice::inline_error(&error),
        _ => ().into_any(),
    };
    view! {
        <section class=format!("{CARD_PAD} mb-3")>
            <div class="flex flex-wrap items-end gap-4">
                <FormatSelector offered=offered selected=format />
                <div class="flex items-center gap-2">
                    <label class=LABEL r#for="version-select">
                        "Version"
                    </label>
                    {select}
                </div>
                <div class="flex items-end gap-2">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="version-at-time">
                            "Time travel (interpreted as UTC)"
                        </label>
                        <input
                            id="version-at-time"
                            type="datetime-local"
                            class=INPUT
                            prop:value=move || at_time_input.get()
                            on:input:target=move |ev| at_time_input.set(ev.target().value())
                        />
                    </div>
                    <button
                        id="version-at-time-go"
                        type="button"
                        class=BTN_SECONDARY
                        disabled=Signal::derive(move || version_at_time.pending().get())
                        on:click=on_at_time
                    >
                        "At time"
                    </button>
                </div>
            </div>
            {note}
        </section>
    }
    .into_any()
}

/// The version `<select>`: a "Latest" option (empty value) plus one option per
/// version, driven by `prop:value` + `on:change`.
fn version_select(entries: Vec<VersionEntry>, selected: RwSignal<String>) -> AnyView {
    let mut options = vec![view! { <option value="">"Latest"</option> }.into_any()];
    options.extend(entries.into_iter().map(|entry| {
        let value = entry.version_id.clone();
        let label = format!("{} — {}", short_version(&entry.version_id), entry.committed);
        view! { <option value=value>{label}</option> }.into_any()
    }));
    view! {
        <select
            id="version-select"
            class=SELECT
            prop:value=move || selected.get()
            on:change=move |ev| selected.set(event_target_value(&ev))
        >
            {options}
        </select>
    }
    .into_any()
}

/// The document pane: the pretty-printed representation for the current
/// `(version, format)` selection, under a `<Transition>` so switching either
/// keeps the prior document visible. A deleted version (`Ok(None)` — the
/// operation's `204`) is a first-class rendered absence; a `406` (declined
/// representation) or any other CDR error renders through the boundary.
///
/// `initial_view` is the deep-linked opening mode (`?view=`), read in the
/// screen's setup — a plain value, so every Suspend re-run mounts the pane the
/// same way on both sides of hydration.
fn document_section(
    document: Resource<Result<Option<String>, ViewerError>>,
    initial_view: PaneView,
) -> AnyView {
    view! {
        <Transition fallback=|| {
            view! {
                <thaw::Skeleton>
                    <thaw::SkeletonItem class="h-64" />
                </thaw::Skeleton>
            }
        }>
            {move || Suspend::new(async move {
                match document.await {
                    Ok(Some(body)) => {
                        let body_sig = RwSignal::new(body);
                        // Resolve inside the Transition: an SSR'd ErrorBoundary fallback
                        // mismatches at hydration in leptos 0.8.
                        view! { <DocumentPane body=body_sig initial_view=initial_view /> }
                            .into_any()
                    }
                    Ok(None) => deleted_pane(),
                    Err(e) => crate::components::notice::inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The document pane for a version the CDR reports as deleted (`204`).
///
/// A rendered absence, not an error: the logical delete commits a
/// deleted-lifecycle version, so the pane says so and points at the history
/// strip, where every earlier version is still readable by its own uid.
fn deleted_pane() -> AnyView {
    view! {
        <div id="composition-version-deleted">
            <EmptyState
                icon=icondata_lu::LuTrash
                message="This version was deleted at the selected instant"
                hint="The delete committed a deleted-lifecycle version; pick an earlier version above to read it."
            />
        </div>
    }
    .into_any()
}

/// The version timeline strip above the audit card: one chip per version,
/// oldest→newest left-to-right, the selected chip accented and the newest
/// tagged "current". Clicking a chip sets the shared selection (the newest →
/// empty string = Latest, matching the dropdown).
fn timeline_section(
    versions: Resource<Result<Vec<VersionEntry>, ViewerError>>,
    selected: RwSignal<String>,
) -> AnyView {
    view! {
        <div class="mt-3">
            <Suspense fallback=|| {
                ().into_any()
            }>
                {move || Suspend::new(async move {
                    match versions.await {
                        Ok(entries) => {
                            let stored = StoredValue::new(entries);
                            // Resolve inside the Suspense: an SSR'd ErrorBoundary fallback
                            // mismatches at hydration in leptos 0.8. A
                            // failed history renders nothing here (the document/toolbar
                            // sections surface the error).
                            view! {
                                {move || {
                                    stored.with_value(|entries| timeline_strip(entries, selected))
                                }}
                            }
                                .into_any()
                        }
                        Err(_) => ().into_any(),
                    }
                })}
            </Suspense>
        </div>
    }
    .into_any()
}

/// Render the timeline chips. `entries` is newest-first (the revision-history
/// order the selector uses), so the strip displays it reversed (oldest→newest).
fn timeline_strip(entries: &[VersionEntry], selected: RwSignal<String>) -> AnyView {
    if entries.is_empty() {
        return ().into_any();
    }
    let newest = entries
        .first()
        .map(|entry| entry.version_id.clone())
        .unwrap_or_default();
    let mut ordered: Vec<VersionEntry> = entries.to_vec();
    ordered.reverse();
    let chips = view! {
        <For each=move || ordered.clone() key=|entry| entry.version_id.clone() let:entry>
            {timeline_chip(&entry, &newest, selected)}
        </For>
    };
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Version timeline"</h2>
            <div class="flex flex-wrap items-center gap-2">{chips}</div>
        </section>
    }
    .into_any()
}

/// One timeline chip: a `rounded-full border` button labelled with the short
/// `vN`, accented while it is the current selection, and suffixed "· current"
/// for the newest version. Clicking selects it — the newest selects Latest
/// (empty string), the others their `OBJECT_VERSION_ID`.
fn timeline_chip(entry: &VersionEntry, newest: &str, selected: RwSignal<String>) -> AnyView {
    let version_id = entry.version_id.clone();
    let is_newest = version_id == newest;
    let target = if is_newest {
        String::new()
    } else {
        version_id.clone()
    };
    let label = short_version(&version_id);
    let newest_owned = newest.to_owned();
    let version_for_class = version_id.clone();
    let class = move || {
        let current = selected.get();
        let is_selected = if is_newest {
            current.is_empty() || current == newest_owned
        } else {
            current == version_for_class
        };
        if is_selected {
            "rounded-full border border-accent bg-accent-subtle px-3 py-1 text-xs font-medium text-accent-ink"
        } else {
            "rounded-full border border-edge px-3 py-1 text-xs font-medium text-ink-muted hover:bg-sunken"
        }
    };
    view! {
        <button type="button" class=class on:click=move |_| selected.set(target.clone())>
            {label}
            {is_newest.then_some(" · current")}
        </button>
    }
    .into_any()
}

/// The audit card for the currently selected version (or the latest when
/// "Latest" is selected), derived reactively from the loaded history.
fn audit_section(
    versions: Resource<Result<Vec<VersionEntry>, ViewerError>>,
    selected: RwSignal<String>,
) -> AnyView {
    view! {
        <div class="mt-3">
            <Suspense fallback=|| {
                ().into_any()
            }>
                {move || Suspend::new(async move {
                    match versions.await {
                        Ok(entries) => {
                            let stored = StoredValue::new(entries);
                            // Resolve inside the Suspense: an SSR'd ErrorBoundary fallback
                            // mismatches at hydration in leptos 0.8. A
                            // failed history renders nothing here (the document/toolbar
                            // sections surface the error).
                            view! {
                                {move || {
                                    let chosen = selected.get();
                                    stored
                                        .with_value(|entries| {
                                            let entry = if chosen.is_empty() {
                                                entries.first()
                                            } else {
                                                entries.iter().find(|e| e.version_id == chosen)
                                            };
                                            audit_card(entry)
                                        })
                                }}
                            }
                                .into_any()
                        }
                        Err(_) => ().into_any(),
                    }
                })}
            </Suspense>
        </div>
    }
    .into_any()
}

/// Render the audit metadata for one version as a card, or the empty state when
/// no matching version is found.
fn audit_card(entry: Option<&VersionEntry>) -> AnyView {
    let Some(entry) = entry else {
        return view! {
            <EmptyState
                icon=icondata_lu::LuShieldCheck
                message="No audit for the selected version"
                hint="Pick another version above — every committed version carries its own audit."
            />
        }
        .into_any();
    };
    let version_id = entry.version_id.clone();
    let committed_at = entry.committed.clone();
    let change_type = entry.change_type.clone();
    let committer = entry.committer.clone();
    view! {
        <section class=CARD_PAD>
            <h2 class=CARD_TITLE>"Audit"</h2>
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {audit_row("version", version_id)} {audit_row("committed", committed_at)}
                {audit_row("change type", change_type)} {audit_row("committer", committer)}
            </div>
        </section>
    }
    .into_any()
}

/// One label/value line in the audit card (empty values shown as an em dash).
fn audit_row(label: &'static str, value: String) -> AnyView {
    let shown = if value.is_empty() {
        "—".to_owned()
    } else {
        value
    };
    view! {
        <div>
            <span class="font-medium text-ink-muted mr-1">{label}":"</span>
            <span class="font-mono break-all text-ink">{shown}</span>
        </div>
    }
    .into_any()
}

/// The short `vN` label from an `OBJECT_VERSION_ID` (the segment after the
/// last `::`), or the whole id when there is no version segment.
fn short_version(version_id: &str) -> String {
    version_id
        .rsplit_once("::")
        .map_or_else(|| version_id.to_owned(), |(_, v)| format!("v{v}"))
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::{parse_versioned_details, parse_versions, short_version};

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

    #[test]
    fn short_version_takes_the_trailing_segment() {
        assert_eq!(short_version("7d44::example.org::3"), "v3");
        assert_eq!(short_version("no-version"), "no-version");
    }
}
