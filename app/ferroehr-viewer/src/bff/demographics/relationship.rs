// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `PARTY_RELATIONSHIP` — the relationship index, the relationship detail, and
//! the party detail's Relationships tab.
//!
//! NOTE: the released Demographic API defines no `party_relationship` path at
//! all — these routes are the CDR's own extension realizing SM
//! `I_PARTY_RELATIONSHIP` (`docs/specs/openehr/SM/docs/UML/classes/i_party_relationship.adoc`).
//!
//! A relationship therefore exists in the CDR in TWO shapes, and the viewer
//! shows both because the spec models both:
//!
//! - **Inline, on its source party.** `PARTY.relationships` is
//!   "Relationships in which this Party takes part as source", with the
//!   invariant `Relationships_validity` requiring every entry's `source` to be
//!   that party
//!   (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`).
//!   That list is part of the party document, so the Relationships tab reads it
//!   from the party itself.
//! - **As its own versioned object**, addressed by
//!   `/demographic/party_relationship/{uid_based_id}` — the SM's shape, which
//!   the extension routes serve with the same CRUD + versioned-read envelope as
//!   a party.
//!
//! The two are not synchronized by the CDR, and neither is a view of the other.
//! One consequence is visible on every screen here: **the TARGET side of a
//! relationship is not enumerable.** `PARTY.reverse_relationships` —
//! "References to relationships in which this Party takes part as target" — is
//! a derived `0..1` function in the RM, and the CDR leaves it unpopulated, so
//! no request answers "which relationships point AT this party". A party's tab
//! says so rather than implying the list it shows is complete; the relationship
//! detail links BOTH endpoints, which is the reachable direction.

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

#[cfg(not(target_arch = "wasm32"))]
use crate::bff::demographics::PartyKind;
use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::records::RecordsMsg;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::{tr, tr_with};
#[cfg(not(target_arch = "wasm32"))]
use crate::uid::container_uid_of;

#[cfg(not(target_arch = "wasm32"))]
/// The `PARTY_REF` namespace the viewer writes.
///
/// `OBJECT_REF.namespace` is a free string with only a legality rule — "local",
/// "unknown", or the standard regex — and the class documentation's own
/// examples are "terminology" and "demographic"
/// (`docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.object_ref.adoc`).
/// No openEHR spec fixes which namespace a demographic party ref carries, so
/// "demographic" is the viewer's own choice, matching the RM's example.
const REF_NAMESPACE: &str = "demographic";

/// One end of a relationship, as its `PARTY_REF` carries it.
///
/// `PARTY_REF` inherits `OBJECT_REF`'s mandatory `namespace`, `type` and `id`
/// (`org.openehr.base.base_types.object_ref.adoc`), and its `type` names the
/// referenced party class — which is what lets the viewer link the end to that
/// kind's route.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartyRefView {
    /// `OBJECT_REF.type` — the referenced party class (`PERSON`, `ROLE`, …).
    pub rm_type: String,
    /// `OBJECT_REF.namespace`.
    pub namespace: String,
    /// `OBJECT_REF.id.value` — the referenced party's version container.
    pub id: String,
}

/// The viewer's view of one `PARTY_RELATIONSHIP` version.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RelationshipState {
    /// The canonical relationship JSON exactly as the CDR served it — the base
    /// every edit is applied to.
    pub body: String,
    /// The served version's `OBJECT_VERSION_ID`, which is both the `If-Match`
    /// value of an update and the path of a delete. Read from the `ETag` the
    /// CDR answered, falling back to the document's own `uid.value`.
    pub version_uid: String,
    /// The version container the routes address.
    pub versioned_object_uid: String,
    /// `LOCATABLE.name.value` — on a relationship this IS its type
    /// (`PARTY_RELATIONSHIP` invariant `Type_validity`: "type = name").
    pub name: String,
    /// `LOCATABLE.archetype_node_id`.
    pub archetype_node_id: String,
    /// `PARTY_RELATIONSHIP.source`.
    pub source: PartyRefView,
    /// `PARTY_RELATIONSHIP.target`.
    pub target: PartyRefView,
    /// `PARTY_RELATIONSHIP.details` pretty-printed, empty when absent.
    pub details: String,
    /// `PARTY_RELATIONSHIP.time_validity` as compact JSON, empty when absent —
    /// shown as a fact, never edited by the viewer.
    pub time_validity: String,
}

/// One entry of a party's inline `PARTY.relationships` list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineRelationship {
    /// `LOCATABLE.name.value` — the relationship type.
    pub name: String,
    /// `LOCATABLE.archetype_node_id`.
    pub archetype_node_id: String,
    /// The relationship's own `uid.value`, when the document carries one
    /// (`LOCATABLE.uid` is `0..1`).
    pub uid: String,
    /// The relationship's `source`.
    pub source: PartyRefView,
    /// The relationship's `target`.
    pub target: PartyRefView,
}

/// Read one relationship
/// (`GET /demographic/party_relationship/{uid_based_id}` — extension).
///
/// `Ok(None)` is the `204` a deleted current version answers with, mirroring
/// the party read.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty id; CDR transport errors pass through;
/// a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success) — a
/// `404` from a CDR that does not serve this extension surfaces there;
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_relationship(
    /// The relationship's `uid_based_id` (either form; the container is used).
    uid: String,
) -> Result<Option<RelationshipState>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let uid = container_uid_of(&uid);
    if uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::RelationshipIdRequired,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/party_relationship/{}",
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
    parse_relationship_state(&response.body, served_etag.as_deref()).map(Some)
}

/// Create a relationship (`POST /demographic/party_relationship` — extension).
///
/// The body is assembled from the form's fields, every one of them an RM
/// attribute of `PARTY_RELATIONSHIP`
/// (`org.openehr.rm.demographic.party_relationship.adoc`): the mandatory
/// `source`/`target` `PARTY_REF`s, the `name` that IS the relationship type
/// (invariant `Type_validity`), the LOCATABLE `archetype_node_id`, and the
/// optional `details`. Both refs carry a `HIER_OBJECT_ID`, because a ref
/// "denotes the Version container of a Party" — RM demographic master02
/// §Modelling of Parties and Relationships — and the CDR refuses an
/// `OBJECT_VERSION_ID` there.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when a mandatory field is blank, an endpoint kind
/// is outside the five, or `details` is not a JSON object; CDR transport errors
/// pass through; any non-2xx CDR answer (the `422` ref-invariant diagnostics
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the created resource carries no `uid`.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn create_relationship(
    /// The relationship's `archetype_node_id`.
    archetype_node_id: String,
    /// The relationship type, stored as `name.value`.
    name: String,
    /// The source party's version container id.
    source_uid: String,
    /// The source party's kind, as its route segment.
    source_kind: String,
    /// The target party's version container id.
    target_uid: String,
    /// The target party's kind, as its route segment.
    target_kind: String,
    /// The optional `details`, as a JSON object; empty leaves it out.
    details: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    // Flat string parameters rather than the draft struct: a server function is
    // a URL-encoded public endpoint, and a flat argument list has no nested
    // encoding to get wrong.
    let body = relationship_body(&RelationshipDraft {
        archetype_node_id,
        name,
        source_uid,
        source_kind,
        target_uid,
        target_kind,
        details,
    })
    .map_err(ViewerError::Invalid)?;
    let url = state.cdr.rest_v1("demographic/party_relationship");
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
            RecordsMsg::CdrCreatedRelationshipButReturnedNo,
        )));
    }
    Ok(uid)
}

/// Commit a new version of a relationship
/// (`PUT /demographic/party_relationship/{uid_based_id}` — extension).
///
/// `If-Match` carries the loaded version, exactly as a party update does, so a
/// concurrent change is refused with a `412` rather than overwritten. The body
/// sent is `base_body` with `name` and `details` replaced; `source`, `target`,
/// `time_validity`, `uid` and everything else travel back verbatim — changing
/// which parties a relationship joins would make it a different relationship,
/// so the viewer does not offer it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a missing version uid, a blank name, or a
/// `details` draft that is not a JSON object; CDR transport errors pass
/// through; any non-2xx CDR answer (the `412` collision included) normalizes
/// via [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_relationship(
    /// The version container to update.
    versioned_object_uid: String,
    /// The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    /// The served relationship document this edit merges into, verbatim.
    base_body: String,
    /// The replacement relationship type (`name.value`).
    name: String,
    /// The replacement `details`, as a JSON object; empty removes it.
    details: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::CurrentVersionUidRequiredUpdateRelationship,
        )));
    }
    let body = apply_relationship_edits(&base_body, &name, &details)?;
    let if_match = format!("\"{current}\"");
    let url = state.cdr.rest_v1(&format!(
        "demographic/party_relationship/{}",
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

/// Logically delete a relationship
/// (`DELETE /demographic/party_relationship/{uid_based_id}` — extension).
///
/// The path is the version to supersede, exactly as the party delete's is, and
/// the CDR answers `409` when it is not the latest.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when the id is not a full `OBJECT_VERSION_ID`; CDR
/// transport errors pass through; any non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_relationship(
    /// The version to supersede, as a full `OBJECT_VERSION_ID`.
    version_uid: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let version_uid = version_uid.trim();
    if !version_uid.contains("::") {
        return Err(ViewerError::Invalid(tr_served(
            RecordsMsg::DeletingRelationshipNeedsLatestVersionS,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/party_relationship/{}",
        urlencoding::encode(version_uid)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
/// Assemble a `PARTY_RELATIONSHIP` create body from the form's fields.
///
/// Pure and unit-tested. Refuses what the RM refuses before any round trip: a
/// blank mandatory attribute, an endpoint kind outside the five concrete party
/// families, an endpoint id that is an `OBJECT_VERSION_ID` (a ref denotes the
/// version CONTAINER — RM demographic master02), or a `details` draft that is
/// not a JSON object (`details` is an `ITEM_STRUCTURE`).
///
/// # Errors
/// The operator-facing complaint naming the offending field.
fn relationship_body(draft: &RelationshipDraft) -> Result<String, String> {
    let archetype_node_id = draft.archetype_node_id.trim();
    let name = draft.name.trim();
    if archetype_node_id.is_empty() {
        return Err(tr(RecordsMsg::ArchetypeNodeIdRequiredLocatableArchetype));
    }
    if name.is_empty() {
        return Err(tr(RecordsMsg::RelationshipTypeRequiredStoredRelationshipS));
    }
    let source = party_ref(&draft.source_uid, &draft.source_kind, "source")?;
    let target = party_ref(&draft.target_uid, &draft.target_kind, "target")?;
    let mut body = serde_json::Map::new();
    drop(body.insert(
        "_type".to_owned(),
        Value::String("PARTY_RELATIONSHIP".to_owned()),
    ));
    drop(body.insert("name".to_owned(), dv_text(name)));
    drop(body.insert(
        "archetype_node_id".to_owned(),
        Value::String(archetype_node_id.to_owned()),
    ));
    drop(body.insert("source".to_owned(), source));
    drop(body.insert("target".to_owned(), target));
    if let Some(details) = parse_details_draft(&draft.details)? {
        drop(body.insert("details".to_owned(), details));
    }
    Ok(Value::Object(body).to_string())
}

/// The create form's seven fields, as the operator left them.
///
/// A named struct rather than a positional argument list: the form dispatches
/// the same value the inline check judged, so a field can never reach the CDR
/// in another field's place.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RelationshipDraft {
    /// The relationship's `archetype_node_id`.
    pub archetype_node_id: String,
    /// The relationship type, stored as `name.value`.
    pub name: String,
    /// The source party's version container id.
    pub source_uid: String,
    /// The source party's kind, as its route segment.
    pub source_kind: String,
    /// The target party's version container id.
    pub target_uid: String,
    /// The target party's kind, as its route segment.
    pub target_kind: String,
    /// The optional `details`, as JSON text; blank leaves it out.
    pub details: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// One `PARTY_REF` for a relationship end.
///
/// # Errors
/// The operator-facing complaint when the id is blank or a version id, or the
/// kind is outside the five concrete party families.
fn party_ref(uid: &str, kind: &str, end: &str) -> Result<Value, String> {
    let uid = uid.trim();
    if uid.is_empty() {
        return Err(tr_with(
            RecordsMsg::PartySIdRequired,
            &[("end", (end).to_string().into())],
        ));
    }
    if uid.contains("::") {
        return Err(tr_with(
            RecordsMsg::PartyMustNamedVersionedObjectUid,
            &[("end", (end).to_string().into())],
        ));
    }
    let kind = PartyKind::from_segment(kind).ok_or_else(|| {
        tr_with(
            RecordsMsg::PartySKindMustOneFive,
            &[("end", (end).to_string().into())],
        )
    })?;
    let mut id = serde_json::Map::new();
    drop(id.insert(
        "_type".to_owned(),
        Value::String("HIER_OBJECT_ID".to_owned()),
    ));
    drop(id.insert("value".to_owned(), Value::String(uid.to_owned())));
    let mut reference = serde_json::Map::new();
    drop(reference.insert("_type".to_owned(), Value::String("PARTY_REF".to_owned())));
    drop(reference.insert(
        "namespace".to_owned(),
        Value::String(REF_NAMESPACE.to_owned()),
    ));
    drop(reference.insert("type".to_owned(), Value::String(kind.rm_type().to_owned())));
    drop(reference.insert("id".to_owned(), Value::Object(id)));
    Ok(Value::Object(reference))
}

#[cfg(not(target_arch = "wasm32"))]
/// A `DV_TEXT` carrying `value`.
fn dv_text(value: &str) -> Value {
    let mut text = serde_json::Map::new();
    drop(text.insert("_type".to_owned(), Value::String("DV_TEXT".to_owned())));
    drop(text.insert("value".to_owned(), Value::String(value.to_owned())));
    Value::Object(text)
}

#[cfg(not(target_arch = "wasm32"))]
/// Read a `details` draft: `None` when blank, `Some(object)` otherwise.
///
/// `PARTY_RELATIONSHIP.details` is an `ITEM_STRUCTURE` `0..1`
/// (`org.openehr.rm.demographic.party_relationship.adoc`), so a non-object can
/// never be valid.
///
/// # Errors
/// The operator-facing complaint when the draft is not a JSON object.
fn parse_details_draft(draft: &str) -> Result<Option<Value>, String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|e| {
        tr_with(
            RecordsMsg::DetailsNotValidJson2,
            &[("e", (e).to_string().into())],
        )
    })?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err("details must be a JSON object: an ITEM_STRUCTURE such as \
             {\"_type\": \"ITEM_TREE\", …}"
            .to_owned())
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Apply the edit form's two changes to the loaded relationship document.
///
/// `base` is re-sent verbatim apart from `name` and `details`, so `source`,
/// `target`, `time_validity`, `uid` and anything a newer release adds survive
/// unchanged.
///
/// # Errors
/// [`ViewerError::Invalid`] when `base` is not a JSON object, the name is
/// blank, the `details` draft is not an object, or the merged document cannot be
/// re-serialized.
fn apply_relationship_edits(base: &str, name: &str, details: &str) -> Result<String, ViewerError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ViewerError::Invalid(
            { tr(RecordsMsg::RelationshipTypeRequiredPartyRelationshipInvariant) }.clone(),
        ));
    }
    let mut doc: Value = serde_json::from_str(base).map_err(|e| {
        ViewerError::Invalid(tr_with(
            RecordsMsg::LoadedRelationshipDocumentNotValidJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let details = parse_details_draft(details).map_err(ViewerError::Invalid)?;
    let object = doc.as_object_mut().ok_or_else(|| {
        ViewerError::Invalid({ tr(RecordsMsg::LoadedRelationshipDocumentNotJsonObject) }.clone())
    })?;
    drop(object.insert("name".to_owned(), dv_text(name)));
    match details {
        Some(details) => drop(object.insert("details".to_owned(), details)),
        None => drop(object.remove("details")),
    }
    serde_json::to_string(&doc).map_err(|e| {
        ViewerError::Invalid(tr_with(
            RecordsMsg::EditedRelationshipCouldNotSerialized,
            &[("e", (e).to_string().into())],
        ))
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a canonical `PARTY_RELATIONSHIP` body into a [`RelationshipState`],
/// keeping the body verbatim.
///
/// `served_etag` is the identifier the CDR's own `ETag` named
/// ([`CdrResponse::etag_version_uid`](crate::cdr::CdrResponse::etag_version_uid));
/// it wins over the document's `uid.value`, because the header is what the
/// server offers for the conditional round-trip. `None` falls back to the body.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_relationship_state(
    body: &str,
    served_etag: Option<&str>,
) -> Result<RelationshipState, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            RecordsMsg::PartyRelationshipJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let version_uid =
        served_etag.map_or_else(|| crate::uid::uid_value_of_document(&doc), str::to_owned);
    Ok(RelationshipState {
        body: body.to_owned(),
        versioned_object_uid: container_uid_of(&version_uid),
        version_uid,
        name: super::json_str(&doc, &["name", "value"]),
        archetype_node_id: doc
            .get("archetype_node_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        source: party_ref_view(doc.get("source")),
        target: party_ref_view(doc.get("target")),
        details: doc
            .get("details")
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_default(),
        time_validity: doc
            .get("time_validity")
            .filter(|value| !value.is_null())
            .map(Value::to_string)
            .unwrap_or_default(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a party document's inline `relationships` list — "relationships in
/// which this Party takes part as source"
/// (`org.openehr.rm.demographic.party.adoc`).
///
/// Called from the party flattening, not from a request of its own: the list is
/// part of the party document, and the screen reads that document once.
pub(super) fn inline_relationships_of(doc: &Value) -> Vec<InlineRelationship> {
    let items = doc
        .get("relationships")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .map(|item| InlineRelationship {
            name: super::json_str(item, &["name", "value"]),
            archetype_node_id: item
                .get("archetype_node_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            uid: crate::uid::uid_value_of_document(item),
            source: party_ref_view(item.get("source")),
            target: party_ref_view(item.get("target")),
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten one `PARTY_REF` into its three carried facts.
fn party_ref_view(value: Option<&Value>) -> PartyRefView {
    let Some(value) = value else {
        return PartyRefView::default();
    };
    PartyRefView {
        rm_type: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        namespace: value
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        id: super::json_str(value, &["id", "value"]),
    }
}
