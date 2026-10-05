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

use leptos::prelude::*;
use leptos::{component, server};
use leptos_meta::Title;
use leptos_router::components::{A, Redirect};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::components::data_table::{CELL, CELL_MONO, ROW, table_shell, table_skeleton};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, INPUT, LABEL, SELECT, TEXTAREA};
use crate::components::format_view::document_section;
use crate::components::logical_delete::{LogicalDeleteCopy, logical_delete_section};
use crate::components::notice::inline_error;
use crate::components::notice::{alert_note, deleted_notice, diagnostic_pane, missing_notice};
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::tab_bar::tab_link;
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::pages::demographics::party::fact_row;
use crate::pages::demographics::{DemographicResource, PartyKind, party_href, relationship_href};
use crate::uid::container_uid_of;

/// The noun phrase every relationship write-failure toast is built around.
const RELATIONSHIP_OBJECT: &str = "this relationship";

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
        return Err(ViewerError::Invalid(
            "a relationship id is required".to_owned(),
        ));
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
        return Err(ViewerError::Internal(
            "the CDR created the relationship but returned no uid to open it by".to_owned(),
        ));
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
        return Err(ViewerError::Invalid(
            "the current version uid is required to update this relationship — reload this screen \
             and retry"
                .to_owned(),
        ));
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
        return Err(ViewerError::Invalid(
            "deleting a relationship needs the latest version's full OBJECT_VERSION_ID — reload \
             this screen and retry"
                .to_owned(),
        ));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/party_relationship/{}",
        urlencoding::encode(version_uid)
    ));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

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
        return Err("an archetype node id is required (LOCATABLE.archetype_node_id)".to_owned());
    }
    if name.is_empty() {
        return Err(
            "a relationship type is required — it is stored as the relationship's name \
             (PARTY_RELATIONSHIP invariant Type_validity)"
                .to_owned(),
        );
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

/// One `PARTY_REF` for a relationship end.
///
/// # Errors
/// The operator-facing complaint when the id is blank or a version id, or the
/// kind is outside the five concrete party families.
fn party_ref(uid: &str, kind: &str, end: &str) -> Result<Value, String> {
    let uid = uid.trim();
    if uid.is_empty() {
        return Err(format!("the {end} party's id is required"));
    }
    if uid.contains("::") {
        return Err(format!(
            "the {end} party must be named by its versioned_object_uid, not by one version's \
             OBJECT_VERSION_ID — a relationship refers to the party's version container"
        ));
    }
    let kind = PartyKind::from_segment(kind)
        .ok_or_else(|| format!("the {end} party's kind must be one of the five party kinds"))?;
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

/// A `DV_TEXT` carrying `value`.
fn dv_text(value: &str) -> Value {
    let mut text = serde_json::Map::new();
    drop(text.insert("_type".to_owned(), Value::String("DV_TEXT".to_owned())));
    drop(text.insert("value".to_owned(), Value::String(value.to_owned())));
    Value::Object(text)
}

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
    let value: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("details is not valid JSON: {e}"))?;
    if value.is_object() {
        Ok(Some(value))
    } else {
        Err("details must be a JSON object — an ITEM_STRUCTURE such as \
             {\"_type\": \"ITEM_TREE\", …}"
            .to_owned())
    }
}

#[cfg(feature = "ssr")]
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
            "a relationship type is required (PARTY_RELATIONSHIP invariant Type_validity)"
                .to_owned(),
        ));
    }
    let mut doc: Value = serde_json::from_str(base).map_err(|e| {
        ViewerError::Invalid(format!(
            "the loaded relationship document is not valid JSON ({e}) — reload this screen and \
             retry"
        ))
    })?;
    let details = parse_details_draft(details).map_err(ViewerError::Invalid)?;
    let object = doc.as_object_mut().ok_or_else(|| {
        ViewerError::Invalid(
            "the loaded relationship document is not a JSON object — reload this screen and retry"
                .to_owned(),
        )
    })?;
    drop(object.insert("name".to_owned(), dv_text(name)));
    match details {
        Some(details) => drop(object.insert("details".to_owned(), details)),
        None => drop(object.remove("details")),
    }
    serde_json::to_string(&doc).map_err(|e| {
        ViewerError::Invalid(format!(
            "the edited relationship could not be serialized: {e}"
        ))
    })
}

#[cfg(feature = "ssr")]
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
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("PARTY_RELATIONSHIP JSON: {e}")))?;
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

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
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

/// One relationship end, linked to its party's detail route when the ref names
/// one of the five concrete kinds.
///
/// `PARTY_REF` deliberately admits the abstract supertypes (`PARTY`, `ACTOR`) —
/// BASE `org.openehr.base.base_types.party_ref.adoc` §`Type_validity` — and no
/// per-kind route addresses those, so such an end renders as plain text with
/// its declared type beside it rather than as a link that would `404`.
fn party_ref_link(reference: &PartyRefView, hook: &'static str) -> AnyView {
    let label = if reference.rm_type.is_empty() {
        "party".to_owned()
    } else {
        reference.rm_type.clone()
    };
    let id = reference.id.clone();
    if id.is_empty() {
        return view! {
            <div>
                <span class="font-medium text-ink-muted mr-1">{label}":"</span>
                <span class="font-mono text-ink" data-relationship-end=hook>
                    "—"
                </span>
            </div>
        }
        .into_any();
    }
    match PartyKind::from_rm_type(&reference.rm_type) {
        Some(kind) => {
            let href = party_href(kind, &id);
            view! {
                <div>
                    <span class="font-medium text-ink-muted mr-1">{label}":"</span>
                    <A
                        href=href
                        attr:class="font-mono break-all text-accent hover:underline"
                        attr:data-relationship-end=hook
                    >
                        {id}
                    </A>
                </div>
            }
            .into_any()
        }
        None => view! {
            <div>
                <span class="font-medium text-ink-muted mr-1">{label}":"</span>
                <span class="font-mono break-all text-ink" data-relationship-end=hook>
                    {id}
                </span>
                <span class="ml-2 text-xs text-ink-faint">
                    "(an abstract party type has no per-kind route)"
                </span>
            </div>
        }
        .into_any(),
    }
}

/// `/demographics/relationship` — the relationship index: open one by id, or
/// create one.
///
/// `?find=<uid>` redirects to that relationship's detail route (the browser
/// screen's no-JavaScript lookup pattern, with the same untracked-read
/// soundness argument). `?source=`/`?source_kind=` prefill the create form, so
/// the party detail's "relate this party" affordance is a plain link.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn RelationshipsPage() -> impl IntoView {
    let query = leptos_router::hooks::use_query_map();
    let find = query
        .with_untracked(|q| q.get("find").unwrap_or_default())
        .trim()
        .to_owned();
    if !find.is_empty() {
        return view! {
            <Title text="Relationship" />
            <Redirect path=relationship_href(&find) />
        }
        .into_any();
    }
    let (source_uid, source_kind) = query.with_untracked(|q| {
        (
            q.get("source").unwrap_or_default(),
            q.get("source_kind").unwrap_or_default(),
        )
    });

    let lookup = lookup_card();
    let create = create_card(&source_uid, &source_kind);
    view! {
        <Title text="Relationships" />
        <div class="p-6">
            <PageHeader
                title="Party relationships"
                subtitle="A relationship joins two parties, source to target. It is its own versioned object here — an extension of this CDR, not part of the released openEHR demographic API."
                crumbs=vec![Crumb::new("Demographics", super::browse_href(PartyKind::Person))]
            />
            {lookup}
            {create}
        </div>
    }
    .into_any()
}

/// The by-id lookup card (the browser screen's plain-GET pattern).
fn lookup_card() -> AnyView {
    let lookup_ref = NodeRef::<leptos::html::Input>::new();
    let navigate = leptos_router::hooks::use_navigate();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let id = lookup_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !id.is_empty() {
            navigate(
                &relationship_href(&id),
                leptos_router::NavigateOptions::default(),
            );
        }
    };
    view! {
        <section class=format!("{CARD_PAD} mb-6")>
            <h2 class=CARD_TITLE>"Open a relationship"</h2>
            <form method="GET" action="/demographics/relationship" on:submit=on_submit>
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="relationship-lookup">
                            "PARTY_RELATIONSHIP id"
                        </label>
                        <input
                            id="relationship-lookup"
                            name="find"
                            type="text"
                            class=INPUT
                            placeholder="versioned_object_uid or version_uid"
                            node_ref=lookup_ref
                        />
                    </div>
                    <button id="relationship-find" type="submit" class=BTN_PRIMARY>
                        "Open"
                    </button>
                </div>
            </form>
            <p class="mt-2 text-xs text-ink-muted">
                "A relationship is reached by its own id: the demographic API publishes no way to ask which relationships point at a given party."
            </p>
        </section>
    }
    .into_any()
}

/// The create card: both ends, the relationship type, its archetype id, and an
/// optional `details` document.
///
/// Uncontrolled inputs read at dispatch. The `?source=` prefill is rendered as
/// the input's `value` ATTRIBUTE (its initial value), which is deterministic
/// from the URL and therefore identical on the server pass and at hydration.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the create card's seven fields + validation + action wiring (rules §1)"
)]
fn create_card(source_uid: &str, source_kind: &str) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let archetype_ref = NodeRef::<leptos::html::Input>::new();
    let name_ref = NodeRef::<leptos::html::Input>::new();
    let source_ref = NodeRef::<leptos::html::Input>::new();
    let source_kind_ref = NodeRef::<leptos::html::Select>::new();
    let target_ref = NodeRef::<leptos::html::Input>::new();
    let target_kind_ref = NodeRef::<leptos::html::Select>::new();
    let details_ref = NodeRef::<leptos::html::Textarea>::new();
    let validation = RwSignal::new(Option::<String>::None);

    let create: Action<RelationshipDraft, Result<String, ViewerError>> =
        Action::new(|draft: &RelationshipDraft| {
            let draft = draft.clone();
            async move {
                create_relationship(
                    draft.archetype_node_id,
                    draft.name,
                    draft.source_uid,
                    draft.source_kind,
                    draft.target_uid,
                    draft.target_kind,
                    draft.details,
                )
                .await
            }
        });

    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match create.value().get() {
        Some(Ok(uid)) => {
            toast_success(
                toaster,
                "Relationship created",
                &format!("New PARTY_RELATIONSHIP version {uid}"),
            );
            navigate(
                &relationship_href(&uid),
                leptos_router::NavigateOptions::default(),
            );
        }
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(
                toaster,
                "Create failed",
                "the new relationship",
                &error,
            );
        }
        None => {}
    });

    let on_click = move |_| {
        let text = |node: NodeRef<leptos::html::Input>| {
            node.get_untracked()
                .map(|el| el.value())
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let choice = |node: NodeRef<leptos::html::Select>| {
            node.get_untracked()
                .map(|el| el.value())
                .unwrap_or_default()
        };
        let draft = RelationshipDraft {
            archetype_node_id: text(archetype_ref),
            name: text(name_ref),
            source_uid: text(source_ref),
            source_kind: choice(source_kind_ref),
            target_uid: text(target_ref),
            target_kind: choice(target_kind_ref),
            details: details_ref
                .get_untracked()
                .map(|el| el.value())
                .unwrap_or_default(),
        };
        // The same pure judgement the server function makes, run inline first
        // so a blank field never costs a round trip.
        if let Err(message) = relationship_body(&draft) {
            validation.set(Some(message));
        } else {
            validation.set(None);
            create.dispatch(draft);
        }
    };

    let source_value = source_uid.to_owned();
    let selected_source = source_kind.to_owned();
    view! {
        <section class=CARD_PAD id="relationship-create">
            <h2 class=CARD_TITLE>"Create a relationship"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "Both ends name a party by its versioned_object_uid — a relationship refers to the party's version container, not to one of its versions. The type you give is stored as the relationship's name, which is what openEHR reads it as."
            </p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="relationship-type">
                            "Relationship type"
                        </label>
                        <input
                            id="relationship-type"
                            type="text"
                            class=INPUT
                            placeholder="employment"
                            node_ref=name_ref
                        />
                    </div>
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="relationship-archetype">
                            "Archetype node id"
                        </label>
                        // `LOCATABLE.archetype_node_id` is mandatory and a
                        // relationship is archetyped like any other LOCATABLE,
                        // but no openEHR spec names a relationship archetype —
                        // so the offered value is a placeholder in the
                        // archetype-id syntax for the operator to replace with
                        // their own (our own design).
                        <input
                            id="relationship-archetype"
                            type="text"
                            class=INPUT
                            value="openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1"
                            node_ref=archetype_ref
                        />
                    </div>
                </div>
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="relationship-source">
                            "Source party id"
                        </label>
                        <input
                            id="relationship-source"
                            type="text"
                            class=INPUT
                            value=source_value
                            placeholder="versioned_object_uid"
                            node_ref=source_ref
                        />
                    </div>
                    {kind_select("relationship-source-kind", &selected_source, source_kind_ref)}
                </div>
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="relationship-target">
                            "Target party id"
                        </label>
                        <input
                            id="relationship-target"
                            type="text"
                            class=INPUT
                            placeholder="versioned_object_uid"
                            node_ref=target_ref
                        />
                    </div>
                    {kind_select("relationship-target-kind", "", target_kind_ref)}
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="relationship-details">
                        "details (canonical JSON ITEM_STRUCTURE — optional)"
                    </label>
                    <textarea
                        id="relationship-details"
                        class=format!("{TEXTAREA} min-h-[6rem]")
                        node_ref=details_ref
                    ></textarea>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="relationship-create-submit"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || create.pending().get())
                        on:click=on_click
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuLink width="14" height="14" />
                        "Create relationship"
                    </button>
                    <Show when=move || create.pending().get()>
                        <span class="text-sm text-ink-muted">"Creating…"</span>
                    </Show>
                </div>
                {move || {
                    validation.get().map(|message| alert_note("relationship-validation", message))
                }}
                {move || match create.value().get() {
                    Some(Err(error)) => inline_error(&error),
                    _ => ().into_any(),
                }}
            </div>
        </section>
    }
    .into_any()
}

/// A party-kind `<select>` for one relationship end.
///
/// `selected` marks the initial option through the `selected` ATTRIBUTE rather
/// than `prop:value` on the select: the value is a URL parameter, so the server
/// pass and hydration agree without any client-side state.
fn kind_select(
    id: &'static str,
    selected: &str,
    node_ref: NodeRef<leptos::html::Select>,
) -> AnyView {
    let selected = if PartyKind::from_segment(selected).is_some() {
        selected.to_owned()
    } else {
        PartyKind::Person.segment().to_owned()
    };
    let options = PartyKind::ALL
        .into_iter()
        .map(|kind| {
            let is_selected = kind.segment() == selected;
            view! {
                <option value=kind.segment() selected=is_selected>
                    {kind.rm_type()}
                </option>
            }
            .into_any()
        })
        .collect::<Vec<_>>();
    view! {
        <div class="flex flex-col gap-1">
            <label class=LABEL r#for=id>
                "Kind"
            </label>
            <select id=id class=SELECT node_ref=node_ref>
                {options}
            </select>
        </div>
    }
    .into_any()
}

/// `/demographics/relationship/{uid}` — one relationship: its facts and both
/// linked ends, the edit form, the document, and its version history.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn RelationshipDetailPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let uid = Signal::derive(move || {
        container_uid_of(&params.with(|p| p.get("uid").unwrap_or_default()))
    });
    let query = leptos_router::hooks::use_query_map();
    let selected: Memo<String> = Memo::new(move |_| {
        query
            .with(|q| q.get("tab"))
            .filter(|tab| !tab.is_empty())
            .unwrap_or_else(|| "relationship".to_owned())
    });

    // The screen's one read of the relationship publishes its latest version
    // uid here, because the delete above the tabs addresses that version.
    let latest_version = RwSignal::new(String::new());
    let detail = relationship_section(uid, latest_version);
    let history = super::history::history_section(
        DemographicResource::Relationship,
        uid,
        selected,
        "history",
    );
    let delete = delete_section(uid, latest_version);
    let tabs = tab_bar(uid, selected);
    let heading = Signal::derive(move || {
        let id = uid.get();
        let short: String = id.chars().take(8).collect();
        format!("Relationship {short}…")
    });

    view! {
        <Title text="Relationship" />
        <div class="p-6">
            <PageHeader
                title=heading
                crumbs=vec![Crumb::new("Relationships", "/demographics/relationship")]
                mono=true
            />
            {delete}
            {tabs}
            <div class="mt-4">
                <div class:hidden=move || selected.get() != "relationship">{detail}</div>
                <div class:hidden=move || selected.get() != "history">{history}</div>
            </div>
        </div>
    }
}

/// The relationship detail's two-tab bar.
fn tab_bar(uid: Signal<String>, selected: Memo<String>) -> AnyView {
    let link = move |value: &'static str, label: &'static str| {
        tab_link(
            move || format!("{}?tab={value}", relationship_href(uid.get().as_str())),
            label,
            Signal::derive(move || selected.get() == value),
        )
    };
    view! {
        <div class="flex flex-wrap gap-1 border-b border-edge pb-2">
            {link("relationship", "Relationship")} {link("history", "History")}
        </div>
    }
    .into_any()
}

/// The relationship resource shared by the tab's sections.
type RelationshipResource = Resource<Result<Option<RelationshipState>, ViewerError>>;

/// One dispatched relationship edit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationshipEdit {
    /// The version container the PUT addresses.
    versioned_object_uid: String,
    /// The loaded version's `OBJECT_VERSION_ID` — the `If-Match` value.
    version_uid: String,
    /// The loaded document, verbatim — the merge base.
    base_body: String,
    /// The new relationship type (`name.value`).
    name: String,
    /// The new `details`, as JSON text; blank removes the attribute.
    details: String,
}

/// The **Relationship** tab: the facts + both linked ends, the edit form, and
/// the document.
///
/// ONE resource, ungated by tab — the screen's single reader of the current
/// relationship: the delete above the tabs addresses the latest version, which
/// this read publishes into `latest_version` rather than reading the same claim
/// twice.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the relationship tab's facts + seeding + editor + document (rules §1)"
)]
fn relationship_section(uid: Signal<String>, latest_version: RwSignal<String>) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let save: Action<RelationshipEdit, Result<String, ViewerError>> =
        Action::new(|edit: &RelationshipEdit| {
            let edit = edit.clone();
            async move {
                update_relationship(
                    edit.versioned_object_uid,
                    edit.version_uid,
                    edit.base_body,
                    edit.name,
                    edit.details,
                )
                .await
            }
        });
    // Only a SUCCESSFUL save refetches, so a refused one leaves the operator's
    // input on screen.
    let saved = Memo::new(move |prev: Option<&usize>| {
        let version = save.version().get();
        if save.value().with(|value| matches!(value, Some(Ok(_)))) {
            version
        } else {
            prev.copied().unwrap_or(0)
        }
    });
    let resource: RelationshipResource = Resource::new(
        move || (uid.get(), saved.get()),
        |(id, _)| async move { fetch_relationship(id).await },
    );

    Effect::new(move |_| match save.value().get() {
        Some(Ok(uid)) => {
            let detail = if uid.is_empty() {
                "A new version was committed.".to_owned()
            } else {
                format!("New version {uid}")
            };
            toast_success(toaster, "Relationship updated", &detail);
        }
        Some(Err(error)) => {
            let title = if error.status_code() == Some(http::StatusCode::PRECONDITION_FAILED) {
                "Relationship changed on the server"
            } else {
                "Save failed"
            };
            crate::feedback::toast_write_failure(toaster, title, RELATIONSHIP_OBJECT, &error);
        }
        None => {}
    });

    let name_draft = RwSignal::new(String::new());
    let details_draft = RwSignal::new(String::new());
    let seeded = RwSignal::new(Option::<String>::None);
    let base = RwSignal::new(String::new());
    let version = RwSignal::new(String::new());
    let container = RwSignal::new(String::new());
    let validation = RwSignal::new(Option::<String>::None);

    let facts = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(state)) => {
                        if seeded.get_untracked().as_deref() != Some(state.version_uid.as_str()) {
                            name_draft.set(state.name.clone());
                            details_draft.set(state.details.clone());
                            base.set(state.body.clone());
                            version.set(state.version_uid.clone());
                            container.set(state.versioned_object_uid.clone());
                            validation.set(None);
                            seeded.set(Some(state.version_uid.clone()));
                        }
                        latest_version.set(state.version_uid.clone());
                        facts_card(&state)
                    }
                    Ok(None) => {
                        deleted_notice(
                            "relationship-deleted",
                            "This relationship's current version is deleted. Its earlier versions are still readable — open one from the History tab.",
                        )
                    }
                    Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                        missing_notice(
                            "relationship-not-found",
                            "No relationship with this id. Relationship routes are an extension of this CDR, so a server built without them answers the same 404.",
                        )
                    }
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();

    let editor = edit_card(
        name_draft,
        details_draft,
        base,
        version,
        container,
        validation,
        save,
    );
    // A failed read renders nothing in the pane — the facts section above
    // states it once (the screen never renders an error as nothing).
    let document = document_section(resource, "relationship-document", |state| {
        state.body.as_str()
    });

    view! { <div class="flex flex-col gap-4">{facts} {editor} {document}</div> }.into_any()
}

/// The relationship's facts, with both ends linked to their parties.
fn facts_card(state: &RelationshipState) -> AnyView {
    view! {
        <section class=CARD_PAD id="relationship-facts">
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {fact_row("type", "type", state.name.clone())}
                {fact_row("archetype", "archetype", state.archetype_node_id.clone())}
                {fact_row("version", "version", state.version_uid.clone())}
                {fact_row("time validity", "time-validity", state.time_validity.clone())}
                {party_ref_link(&state.source, "source")} {party_ref_link(&state.target, "target")}
            </div>
        </section>
    }
    .into_any()
}

/// The relationship edit card: its type and `details`.
fn edit_card(
    name_draft: RwSignal<String>,
    details_draft: RwSignal<String>,
    base: RwSignal<String>,
    version: RwSignal<String>,
    container: RwSignal<String>,
    validation: RwSignal<Option<String>>,
    save: Action<RelationshipEdit, Result<String, ViewerError>>,
) -> AnyView {
    let on_save = move |_| {
        let name = name_draft.get();
        let details = details_draft.get();
        if name.trim().is_empty() {
            validation.set(Some(
                "A relationship needs a type — openEHR stores it as the relationship's name."
                    .to_owned(),
            ));
            return;
        }
        if let Err(message) = parse_details_draft(&details) {
            validation.set(Some(message));
            return;
        }
        validation.set(None);
        save.dispatch(RelationshipEdit {
            versioned_object_uid: container.get(),
            version_uid: version.get(),
            base_body: base.get(),
            name,
            details,
        });
    };
    let diagnostic = diagnostic_pane(
        "relationship-diagnostic",
        Signal::derive(move || save.value().get().and_then(Result::err)),
    );
    view! {
        <section class=CARD_PAD id="relationship-edit">
            <h2 class=CARD_TITLE>"Edit relationship"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "Commits a new version on top of the one loaded above (If-Match). The two ends and the time validity travel back exactly as the CDR served them — a relationship between different parties is a different relationship."
            </p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="relationship-edit-type">
                        "Relationship type"
                    </label>
                    <input
                        id="relationship-edit-type"
                        type="text"
                        class=INPUT
                        prop:value=move || name_draft.get()
                        on:input:target=move |ev| name_draft.set(ev.target().value())
                    />
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="relationship-edit-details">
                        "details (canonical JSON ITEM_STRUCTURE — leave blank to remove)"
                    </label>
                    <textarea
                        id="relationship-edit-details"
                        class=format!("{TEXTAREA} min-h-[8rem]")
                        prop:value=move || details_draft.get()
                        on:input:target=move |ev| details_draft.set(ev.target().value())
                    >
                        {details_draft.get_untracked()}
                    </textarea>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="relationship-save"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || save.pending().get())
                        on:click=on_save
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuSave width="14" height="14" />
                        "Save relationship"
                    </button>
                    <Show when=move || save.pending().get()>
                        <span class="text-sm text-ink-muted">"Saving…"</span>
                    </Show>
                </div>
                {move || {
                    validation
                        .get()
                        .map(|message| alert_note("relationship-edit-validation", message))
                }}
                {diagnostic}
            </div>
        </section>
    }
    .into_any()
}

/// The **Delete relationship** affordance, mirroring the party's: a confirmed
/// logical delete that commits a deleted version and leaves the earlier ones
/// readable.
///
/// `version_uid` is the screen's ONE read of the relationship, published by
/// [`relationship_section`] — the delete addresses the version to supersede.
fn delete_section(uid: Signal<String>, version_uid: RwSignal<String>) -> AnyView {
    let delete: Action<String, Result<(), ViewerError>> = Action::new(|version: &String| {
        let version = version.clone();
        async move { delete_relationship(version).await }
    });
    let message = Signal::derive(move || {
        format!(
            "Delete relationship {}? This commits a deleted version: the relationship stops \
             resolving as current, and every earlier version stays readable by its own version \
             uid.",
            uid.get()
        )
    });
    logical_delete_section(
        delete,
        version_uid,
        message,
        "/demographics/relationship".to_owned(),
        LogicalDeleteCopy {
            button_id: "relationship-delete",
            label: "Delete relationship",
            confirm_id: "relationship-delete-confirm",
            success_title: "Relationship deleted",
            object: RELATIONSHIP_OBJECT,
        },
    )
}

/// The party detail's **Relationships** tab: the party's own inline
/// `relationships`, the honest note about the direction the wire cannot answer,
/// and the affordance that creates a relationship FROM this party.
///
/// The list is a projection of the party document the screen ALREADY read
/// ([`PartyState::relationships`](super::party::PartyState::relationships)), not
/// a second request. A failed or absent read renders its own error HERE rather
/// than deferring to the Party tab: on `?tab=relationships` that tab is
/// `class:hidden`, so its message would be in the DOM and invisible. The same
/// read decides whether to offer the create affordance, since relating a party
/// the CDR cannot serve is a write against an unknown source.
pub(super) fn party_relationships_section(
    kind: PartyKind,
    uid: Signal<String>,
    party: Resource<Result<Option<super::party::PartyState>, ViewerError>>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match party.await {
                    Ok(Some(state)) => relationships_view(kind, uid, &state.relationships),
                    Ok(None) => {
                        deleted_notice(
                            "party-relationships-deleted",
                            "This party's current version is deleted, so it has no relationships to show and cannot be related to another party.",
                        )
                    }
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The loaded party's relationships plus the affordance that creates one from
/// it.
fn relationships_view(
    kind: PartyKind,
    uid: Signal<String>,
    items: &[InlineRelationship],
) -> AnyView {
    let table = if items.is_empty() {
        view! {
            <EmptyState
                icon=icondata_lu::LuNetwork
                message="No relationships on this party"
                hint="A party document carries the relationships it is the SOURCE of. Create one below, or add it to the party's own relationships list on the Party tab."
            />
        }
        .into_any()
    } else {
        inline_table(items)
    };
    let create_href = move || {
        format!(
            "/demographics/relationship?source={}&source_kind={}",
            urlencoding::encode(&uid.get()),
            kind.segment()
        )
    };
    view! {
        <div class="flex flex-col gap-4">
            <section class=CARD_PAD id="party-relationships">
                <h2 class=CARD_TITLE>"Relationships (as source)"</h2>
                <p class="mb-3 text-xs text-ink-muted">
                    "These come from the party document itself, which carries the relationships this party is the source of. openEHR models the other direction as a derived attribute the CDR leaves unpopulated, so no request can list the relationships that point AT this party — open those by their own id."
                </p>
                {table}
            </section>
            <section class=CARD_PAD>
                <h2 class=CARD_TITLE>"Relate this party"</h2>
                <p class="mb-3 text-xs text-ink-muted">
                    "Creates a PARTY_RELATIONSHIP resource with this party as its source. That resource is versioned in its own right — an extension of this CDR, not part of the released openEHR demographic API."
                </p>
                <a href=create_href class=BTN_PRIMARY id="party-relate">
                    <leptos_icons::Icon icon=icondata_lu::LuLink width="14" height="14" />
                    "Create a relationship"
                </a>
            </section>
        </div>
    }
    .into_any()
}

/// The party's inline relationships as a table.
///
/// A plain collected `Vec` rather than `<For>`: the list is a derived
/// projection of the party document, replaced wholesale whenever that document
/// reloads, and an inline relationship carries no identity of its own to key on
/// (`LOCATABLE.uid` is `0..1` and is normally absent inline); an index key is
/// forbidden, and a synthetic one would be exactly that.
fn inline_table(items: &[InlineRelationship]) -> AnyView {
    let rows = items
        .iter()
        .map(|item| {
            // Two bindings: the view! macro moves child text before evaluating
            // attribute clones, so one String cannot serve both positions.
            let name = item.name.clone();
            let hook = item.name.clone();
            let archetype = item.archetype_node_id.clone();
            let uid = item.uid.clone();
            let target = party_ref_link(&item.target, "inline-target");
            view! {
                <tr class=ROW>
                    <td class=CELL data-inline-relationship=hook>
                        {name}
                    </td>
                    <td class=CELL>{target}</td>
                    <td class=CELL_MONO>{archetype}</td>
                    <td class=CELL_MONO>{uid}</td>
                </tr>
            }
            .into_any()
        })
        .collect::<Vec<_>>();
    table_shell(&["Type", "Target", "Archetype", "Own uid"], rows.into_any())
}

#[cfg(test)]
mod tests {
    use super::{RelationshipDraft, parse_details_draft, relationship_body};
    use serde_json::Value;

    const SOURCE: &str = "8849182c-82ad-4088-a07f-48ead4180515";
    const TARGET: &str = "b1e6a0c4-6b2e-4f3a-9c1d-2f5a7e8b0c31";

    fn draft(source: &str, target: &str) -> RelationshipDraft {
        RelationshipDraft {
            archetype_node_id: "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1".to_owned(),
            name: "employment".to_owned(),
            source_uid: source.to_owned(),
            source_kind: "person".to_owned(),
            target_uid: target.to_owned(),
            target_kind: "organisation".to_owned(),
            details: String::new(),
        }
    }

    fn body(source: &str, target: &str) -> Result<String, String> {
        relationship_body(&draft(source, target))
    }

    #[test]
    fn a_created_relationship_carries_both_ends_as_container_refs() {
        let doc: Value =
            serde_json::from_str(&body(SOURCE, TARGET).expect("a valid body")).expect("JSON");
        assert_eq!(doc["_type"], "PARTY_RELATIONSHIP");
        // PARTY_RELATIONSHIP invariant Type_validity: type = name.
        assert_eq!(doc["name"]["value"], "employment");
        assert_eq!(doc["source"]["_type"], "PARTY_REF");
        assert_eq!(doc["source"]["type"], "PERSON");
        assert_eq!(doc["source"]["namespace"], "demographic");
        // A ref denotes the party's version CONTAINER (RM demographic
        // master02), so the id is a HIER_OBJECT_ID.
        assert_eq!(doc["source"]["id"]["_type"], "HIER_OBJECT_ID");
        assert_eq!(doc["source"]["id"]["value"], SOURCE);
        assert_eq!(doc["target"]["type"], "ORGANISATION");
        assert_eq!(doc["target"]["id"]["value"], TARGET);
        // details is optional and stays out when blank.
        assert!(doc.get("details").is_none());
    }

    #[test]
    fn a_version_id_endpoint_is_refused_before_the_round_trip() {
        let message = body(&format!("{SOURCE}::example.org::2"), TARGET)
            .expect_err("a version id is not a container id");
        assert!(message.contains("source"), "{message}");
        assert!(message.contains("versioned_object_uid"), "{message}");
        let message = body(SOURCE, &format!("{TARGET}::example.org::1"))
            .expect_err("a version id is not a container id");
        assert!(message.contains("target"), "{message}");
    }

    #[test]
    fn every_mandatory_field_is_required() {
        assert!(body("", TARGET).is_err());
        assert!(body(SOURCE, "  ").is_err());
        let message = relationship_body(&RelationshipDraft {
            archetype_node_id: String::new(),
            ..draft(SOURCE, TARGET)
        })
        .expect_err("archetype_node_id is mandatory on a LOCATABLE");
        assert!(message.contains("archetype node id"), "{message}");
        let message = relationship_body(&RelationshipDraft {
            name: "  ".to_owned(),
            ..draft(SOURCE, TARGET)
        })
        .expect_err("the type IS the name, and name is mandatory");
        assert!(message.contains("relationship type"), "{message}");
        // An endpoint kind outside the five concrete families has no route.
        let message = relationship_body(&RelationshipDraft {
            source_kind: "party".to_owned(),
            ..draft(SOURCE, TARGET)
        })
        .expect_err("PARTY is abstract");
        assert!(message.contains("five party kinds"), "{message}");
    }

    #[test]
    fn a_details_draft_must_be_an_object_or_blank() {
        assert_eq!(parse_details_draft("   "), Ok(None));
        let details = parse_details_draft(r#"{"_type":"ITEM_TREE"}"#)
            .expect("an object is accepted")
            .expect("some value");
        assert_eq!(details["_type"], "ITEM_TREE");
        for bad in ["[]", "\"x\"", "3"] {
            assert!(parse_details_draft(bad).is_err(), "{bad}");
        }
        assert!(parse_details_draft("{").is_err());
        // …and the create body carries an accepted draft through.
        let doc: Value = serde_json::from_str(
            &relationship_body(&RelationshipDraft {
                details: r#"{"_type":"ITEM_TREE","archetype_node_id":"at0002"}"#.to_owned(),
                ..draft(SOURCE, TARGET)
            })
            .expect("a valid body"),
        )
        .expect("JSON");
        assert_eq!(doc["details"]["archetype_node_id"], "at0002");
    }
}

#[cfg(all(test, feature = "ssr"))]
mod wire_tests {
    use super::{apply_relationship_edits, inline_relationships_of, parse_relationship_state};
    use serde_json::Value;

    const RELATIONSHIP: &str = r#"{
        "_type": "PARTY_RELATIONSHIP",
        "name": {"_type": "DV_TEXT", "value": "employment"},
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::example.org::2"},
        "source": {"_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                   "id": {"_type": "HIER_OBJECT_ID", "value": "8849182c"}},
        "target": {"_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                   "id": {"_type": "HIER_OBJECT_ID", "value": "b1e6a0c4"}},
        "time_validity": {"_type": "DV_INTERVAL", "lower": {"_type": "DV_DATE", "value": "2026-01-01"}},
        "details": {"_type": "ITEM_TREE", "archetype_node_id": "at0002", "items": []}
    }"#;

    #[test]
    fn parses_a_relationships_facts_and_both_ends() {
        let state = parse_relationship_state(RELATIONSHIP, None).expect("a valid relationship");
        assert_eq!(state.name, "employment");
        assert_eq!(state.version_uid, "7d44aa01::example.org::2");
        assert_eq!(state.versioned_object_uid, "7d44aa01");
        assert_eq!(state.source.rm_type, "PERSON");
        assert_eq!(state.source.id, "8849182c");
        assert_eq!(state.source.namespace, "demographic");
        assert_eq!(state.target.rm_type, "ORGANISATION");
        assert_eq!(state.target.id, "b1e6a0c4");
        assert!(state.details.contains("ITEM_TREE"));
        assert!(state.time_validity.contains("2026-01-01"));
        assert_eq!(state.body, RELATIONSHIP);
        assert!(parse_relationship_state("not json", None).is_err());
        // The served `ETag` is the precondition an update echoes back, not a
        // value re-derived from the document (ITS-REST overview §ETag and
        // Last-Modified).
        let served = parse_relationship_state(RELATIONSHIP, Some("7d44aa01::example.org::6"))
            .expect("a valid relationship");
        assert_eq!(served.version_uid, "7d44aa01::example.org::6");
        assert_eq!(served.versioned_object_uid, "7d44aa01");
    }

    #[test]
    fn editing_replaces_the_type_and_details_and_keeps_both_ends() {
        let merged = apply_relationship_edits(
            RELATIONSHIP,
            "authority",
            r#"{"_type":"ITEM_TREE","archetype_node_id":"at0009"}"#,
        )
        .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["name"]["value"], "authority");
        assert_eq!(doc["details"]["archetype_node_id"], "at0009");
        // The ends, the time validity and the uid survive verbatim.
        assert_eq!(doc["source"]["id"]["value"], "8849182c");
        assert_eq!(doc["target"]["id"]["value"], "b1e6a0c4");
        assert_eq!(doc["time_validity"]["lower"]["value"], "2026-01-01");
        assert_eq!(doc["uid"]["value"], "7d44aa01::example.org::2");
        // A blank details draft removes the optional attribute.
        let cleared =
            apply_relationship_edits(RELATIONSHIP, "employment", "").expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&cleared).expect("merged JSON");
        assert!(doc.get("details").is_none());
        // A blank type is refused: the type IS the name, which is mandatory.
        assert!(apply_relationship_edits(RELATIONSHIP, "  ", "").is_err());
        assert!(apply_relationship_edits("not json", "employment", "").is_err());
    }

    #[test]
    fn projects_a_partys_inline_relationships() {
        let party: Value = serde_json::from_str(
            r#"{
            "_type": "PERSON",
            "relationships": [{
                "_type": "PARTY_RELATIONSHIP",
                "name": {"_type": "DV_TEXT", "value": "employment"},
                "archetype_node_id": "at0005",
                "source": {"_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                           "id": {"_type": "HIER_OBJECT_ID", "value": "8849182c"}},
                "target": {"_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                           "id": {"_type": "HIER_OBJECT_ID", "value": "b1e6a0c4"}}
            }]
        }"#,
        )
        .expect("a valid party");
        let items = inline_relationships_of(&party);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "employment");
        assert_eq!(items[0].target.rm_type, "ORGANISATION");
        assert_eq!(items[0].target.id, "b1e6a0c4");
        // An inline relationship normally carries no uid of its own.
        assert_eq!(items[0].uid, "");
        // A party with no relationships attribute is an empty list, not a
        // failure — the attribute is 0..1.
        let bare: Value = serde_json::from_str(r#"{"_type":"PERSON"}"#).expect("a valid party");
        assert!(inline_relationships_of(&bare).is_empty());
    }
}
