// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/demographics/{kind}/{uid}` screen — one party, read, edited, walked
//! version by version, related, and tagged.
//!
//! Four URL-driven tabs (`?tab=`) over one versioned object:
//!
//! - **Party** (default) — the CURRENT version
//!   (`GET /demographic/{kind}/{uid_based_id}`): its facts, the `edit_form`
//!   that commits a new version (`PUT` with `If-Match`), and the whole document
//!   in the shared [`document_section`].
//! - **History** — the `VERSIONED_PARTY` family
//!   ([`history`](super::history)).
//! - **Relationships** — the party's own `relationships` list
//!   ([`relationship`](super::relationship)).
//! - **Tags** — its `ITEM_TAG`s ([`tags`](super::tags)).
//!
//! One reader per claim: the Party tab is the viewer's ONE reader of the
//! current party document (and the edit form's merge base); the History tab
//! never touches that route — it reads the versioned family for the container +
//! envelope facts and pins a document by an explicit `OBJECT_VERSION_ID`.
//!
//! The edit form replaces exactly `identities` and `details` and re-sends
//! everything else verbatim — never a re-model of the served document. Those
//! two are the party's content: `identities` is RM-mandatory (`PARTY`
//! invariant `Identities_valid`: "not `identities.is_empty`") and `details` is
//! "all other details for this Party"
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`).
//! `name` is deliberately NOT editable: on a `PARTY` it carries the party TYPE
//! (`Type_valid`: "type = name", and `type()` is "Taken from inherited `_name_`
//! attribute"), not a person's name.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
use crate::locale::{Msg, tr, tr_with};
use crate::bff::demographics::PartyKind;
use crate::uid::container_uid_of;

/// The viewer's view of one party version.
///
/// The canonical document verbatim, the version that document IS, and the two
/// attributes the edit form works on — flattened BFF-side so the browser never
/// re-models the RM, with fixed-size ints only.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartyState {
    /// The canonical party JSON exactly as the CDR served it — the base every
    /// edit is applied to, so nothing outside the edited attributes is lost.
    pub body: String,
    /// The served version's `OBJECT_VERSION_ID`, which is both the `If-Match`
    /// value of an update and the path of a delete. Read from the `ETag` the
    /// CDR answered, falling back to the document's own `uid.value`.
    pub version_uid: String,
    /// The version container the routes address ([`container_uid_of`] of
    /// [`Self::version_uid`]).
    pub versioned_object_uid: String,
    /// The document's `_type` (`PERSON`, `ROLE`, …).
    pub rm_type: String,
    /// `LOCATABLE.name.value` — on a `PARTY` this is the party TYPE.
    pub name: String,
    /// `LOCATABLE.archetype_node_id`.
    pub archetype_node_id: String,
    /// `PARTY.identities` pretty-printed as canonical JSON (the edit draft).
    pub identities: String,
    /// How many `PARTY_IDENTITY` entries [`Self::identities`] holds.
    pub identity_count: u32,
    /// `PARTY.details` pretty-printed, empty when the party carries none.
    pub details: String,
    /// The document's inline `PARTY.relationships` — "relationships in which
    /// this Party takes part as source". Part of the party document, so the
    /// Relationships tab projects them from this one read rather than asking
    /// again.
    pub relationships: Vec<super::relationship::InlineRelationship>,
}

/// Read one party (`GET /demographic/{kind}/{uid_based_id}`), flattened into a
/// [`PartyState`].
///
/// `Ok(None)` is the CDR's `204`: the addressed party exists but its current
/// version is logically deleted ("`204` … deleted at time",
/// `operations/person_get.yaml`) — a first-class state, not an error.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment or an empty id; CDR
/// transport errors pass through; a non-2xx CDR answer (the `404` for an
/// unknown id, or for an id held by a DIFFERENT kind, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_party(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The party's `uid_based_id` (either form; the container is used).
    uid: String,
) -> Result<Option<PartyState>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(Msg::PartyKind)))?;
    let uid = container_uid_of(&uid);
    if uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::PartyIdRequired)));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}",
        kind.segment(),
        urlencoding::encode(&uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NO_CONTENT) {
        return Ok(None);
    }
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let served_etag = response.etag_version_uid();
    parse_party_state(&response.body, served_etag.as_deref()).map(Some)
}

/// Create the first version of a party (`POST /demographic/{kind}` —
/// `operations/person_create.yaml`, answering `201`).
///
/// `body` is the operator's canonical-JSON party document, sent verbatim: the
/// viewer never assembles a party from its own model, so nothing it does not
/// render can be dropped. `Prefer: return=representation` asks for the created
/// resource, whose `uid.value` is the new version.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment, a body that is not a
/// JSON object, or a body whose `_type` names a different kind than the route;
/// CDR transport errors pass through; any non-2xx CDR answer (the `400` parse
/// refusal and the `422` validation diagnostic, rendered verbatim, included)
/// normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the created resource carries no `uid`.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_party(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The party document to commit, as canonical JSON text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(Msg::PartyKind)))?;
    let body = checked_party_body(&body, kind)?;
    let url = state
        .cdr
        .rest_v1(&format!("demographic/{}", kind.segment()));
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
    let uid = crate::uid::uid_value_of(&response.body);
    if uid.is_empty() {
        return Err(ViewerError::Internal(tr_served(
            Msg::CdrCreatedPartyButReturnedNo,
        )));
    }
    Ok(uid)
}

/// Commit a new version of a party (`PUT /demographic/{kind}/{uid_based_id}`).
///
/// The path carries the version CONTAINER — the update's `uid_based_id` "can
/// take only a form of an HIER_OBJECT_ID identifier taken from
/// VERSIONED_OBJECT.uid.value" (`operations/person_update.yaml`) — and
/// `If-Match` carries the version the loading read's `ETag` named, since "the
/// existing latest `version_uid` of PERSON resource (i.e. the
/// `preceding_version_uid`) must be
/// specified in the `If-Match` header" (same file), quoted per the overview
/// (`docs/overview/Requests_and_responses.md` §"If-Match and accidental
/// overwrites"). A stale value is the operation's `412`, which reaches the UI
/// as [`ViewerError::Cdr`] with that status and gets its own toast.
///
/// The body sent is `base_body` with exactly `identities` and `details`
/// replaced (`apply_party_edits`); everything else travels back verbatim.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment, a missing version uid,
/// an `identities` draft that is not a non-empty JSON array, or a `details`
/// draft that is not a JSON object; CDR transport errors pass through; any
/// non-2xx CDR answer (the `412` collision and the `400`/`422` diagnostics
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_party(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The version container to update.
    versioned_object_uid: String,
    /// The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    /// The served party document this edit merges into, verbatim.
    base_body: String,
    /// The replacement `identities`, as a JSON array.
    identities: String,
    /// The replacement `details`, as a JSON object; empty removes it.
    details: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(Msg::PartyKind)))?;
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(
            { tr_served(Msg::CurrentVersionUidRequiredUpdateParty) }.clone(),
        ));
    }
    let body = apply_party_edits(&base_body, &identities, &details)?;
    let if_match = format!("\"{current}\"");
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}",
        kind.segment(),
        urlencoding::encode(&container_uid_of(&versioned_object_uid))
    ));
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
    Ok(crate::uid::uid_value_of(&response.body))
}

/// Logically delete a party (`DELETE /demographic/{kind}/{uid_based_id}`).
///
/// The path is the VERSION to supersede, not the container: the delete's
/// `uid_based_id` "MUST be in a form of an OBJECT_VERSION_ID identifier taken
/// from the last (most recent) VERSION.uid.value, representing the
/// `preceding_version_uid` to be deleted" (`operations/person_delete.yaml`).
/// That operation declares no `If-Match` at all — the precondition IS the path
/// — and answers `204`, or `409` when the supplied uid is not the latest
/// version, or `400` when the party is already deleted.
///
/// This is the openEHR LOGICAL delete: it commits a `523|deleted|` version, so
/// every earlier version stays readable by its own uid.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment or a version uid that
/// is not a full `OBJECT_VERSION_ID`; CDR transport errors pass through; any
/// non-2xx CDR answer (the `409` stale-uid and `400` already-deleted branches
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_party(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The version to supersede, as a full `OBJECT_VERSION_ID`.
    version_uid: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, &tr_served(Msg::PartyKind)))?;
    let version_uid = version_uid.trim();
    if !version_uid.contains("::") {
        return Err(ViewerError::Invalid(tr_served(
            Msg::DeletingPartyNeedsLatestVersionS,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/{}/{}",
        kind.segment(),
        urlencoding::encode(version_uid)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
/// Check an operator-authored party body before it is sent: it must be a JSON
/// object, and its `_type` must be the routed kind's RM type.
///
/// The routed kind picks the type the CDR decodes into, so a mismatched `_type`
/// is refused by the CDR's own strict reader as a `400` — this check reports it
/// as the operator's own mistake instead, naming both types.
///
/// # Errors
/// [`ViewerError::Invalid`] when the body is not valid JSON, not an object, or
/// declares a different `_type`.
fn checked_party_body(body: &str, kind: PartyKind) -> Result<String, ViewerError> {
    let doc: Value = serde_json::from_str(body.trim()).map_err(|e| {
        ViewerError::Invalid(tr_with(
            Msg::PartyBodyNotValidJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let object = doc
        .as_object()
        .ok_or_else(|| ViewerError::Invalid(tr(Msg::PartyBodyMustJsonObject)))?;
    let declared = object
        .get("_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if declared != kind.rm_type() {
        return Err(ViewerError::Invalid(format!(
            "this is the {} route, but the body declares _type {declared:?}: create it under \
             /demographics/{} instead, or fix the _type",
            kind.rm_type(),
            declared.to_lowercase()
        )));
    }
    Ok(doc.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a canonical party body into a [`PartyState`], keeping the body
/// verbatim. Defensive throughout — an absent attribute reads as its empty
/// default rather than failing the tab.
///
/// `served_etag` is the identifier the CDR's own `ETag` named
/// ([`CdrResponse::etag_version_uid`](crate::cdr::CdrResponse::etag_version_uid));
/// it wins over the document's `uid.value`, because the header is what the
/// server offers for the conditional round-trip. `None` falls back to the body.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_party_state(body: &str, served_etag: Option<&str>) -> Result<PartyState, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::PartyJson, &[("e", (e).to_string().into())]))
    })?;
    let version_uid =
        served_etag.map_or_else(|| crate::uid::uid_value_of_document(&doc), str::to_owned);
    let identities = doc.get("identities").filter(|value| !value.is_null());
    Ok(PartyState {
        body: body.to_owned(),
        versioned_object_uid: container_uid_of(&version_uid),
        version_uid,
        rm_type: doc
            .get("_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name: super::json_str(&doc, &["name", "value"]),
        archetype_node_id: doc
            .get("archetype_node_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        identities: identities
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_default(),
        identity_count: count_of(identities),
        details: doc
            .get("details")
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_default(),
        relationships: super::relationship::inline_relationships_of(&doc),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The length of a JSON array attribute as the fixed-size int the wire type
/// carries, saturating rather than wrapping.
fn count_of(value: Option<&Value>) -> u32 {
    value
        .and_then(Value::as_array)
        .map_or(0, |items| u32::try_from(items.len()).unwrap_or(u32::MAX))
}

/// Read an `identities` draft: a JSON array with at least one entry.
///
/// `PARTY.identities` is `List<PARTY_IDENTITY>` `1..1` with the invariant
/// `Identities_valid`: "not `identities.is_empty`"
/// (`org.openehr.rm.demographic.party.adoc`), so an empty array or a non-array
/// can never be valid and is refused here, before anything is sent. Everything
/// beyond that shape is the CDR's call, and its diagnostic is rendered
/// verbatim.
///
/// # Errors
/// The operator-facing complaint when the draft is not a non-empty JSON array.
fn parse_identities(draft: &str) -> Result<Value, String> {
    let trimmed = draft.trim();
    let value: Value = serde_json::from_str(trimmed).map_err(|e| {
        tr_with(
            Msg::IdentitiesNotValidJson,
            &[("e", (e).to_string().into())],
        )
    })?;
    match value.as_array() {
        Some(items) if !items.is_empty() => Ok(value),
        Some(_) => Err(tr(Msg::IdentitiesMustHoldLeastOneParty)),
        None => Err(tr(Msg::IdentitiesMustJsonArrayPartyIdentity)),
    }
}

/// Read a `details` draft: `None` for a blank draft (the attribute is removed),
/// `Some(value)` for a JSON object.
///
/// `PARTY.details` is an `ITEM_STRUCTURE` `0..1`
/// (`org.openehr.rm.demographic.party.adoc`), so a non-object — an array, a
/// bare string, a number — can never be valid.
///
/// # Errors
/// The operator-facing complaint when the draft is not parseable JSON or not a
/// JSON object.
fn parse_details(draft: &str) -> Result<Option<Value>, String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| tr_with(Msg::DetailsNotValidJson, &[("e", (e).to_string().into())]))?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err("details must be a JSON object: an ITEM_STRUCTURE such as \
             {\"_type\": \"ITEM_TREE\", …}"
            .to_owned())
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Apply the form's two edits to the loaded party document and return the body
/// to PUT.
///
/// `base` is re-sent verbatim apart from `identities` and `details` — the merge
/// is a key replacement on the parsed object, never a rebuild, so every other
/// attribute (`uid`, `name`, `archetype_node_id`, `contacts`, `relationships`,
/// `languages`, `roles`, `performer`, anything a newer spec release adds)
/// survives unchanged. A blank `details` REMOVES the key, the only way to
/// express "no details" for an optional attribute.
///
/// # Errors
/// [`ViewerError::Invalid`] when `base` is not a JSON object, when either
/// draft fails its own shape check, or when the merged document cannot be
/// re-serialized.
pub(super) fn apply_party_edits(
    base: &str,
    identities: &str,
    details: &str,
) -> Result<String, ViewerError> {
    let mut doc: Value = serde_json::from_str(base).map_err(|e| {
        ViewerError::Invalid(tr_with(
            Msg::LoadedPartyDocumentNotValidJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let identities = parse_identities(identities).map_err(ViewerError::Invalid)?;
    let details = parse_details(details).map_err(ViewerError::Invalid)?;
    let object = doc
        .as_object_mut()
        .ok_or_else(|| ViewerError::Invalid(tr(Msg::LoadedPartyDocumentNotJsonObject)))?;
    drop(object.insert("identities".to_owned(), identities));
    match details {
        Some(details) => drop(object.insert("details".to_owned(), details)),
        None => drop(object.remove("details")),
    }
    serde_json::to_string(&doc).map_err(|e| {
        ViewerError::Invalid(tr_with(
            Msg::EditedPartyCouldNotSerialized,
            &[("e", (e).to_string().into())],
        ))
    })
}
