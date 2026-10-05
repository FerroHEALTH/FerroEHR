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
use leptos::{component, server};
use leptos_meta::Title;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::components::data_table::table_skeleton;
use crate::components::field::{BTN_PRIMARY, LABEL, TEXTAREA};
use crate::components::format_view::document_section;
use crate::components::logical_delete::{LogicalDeleteCopy, logical_delete_section};
use crate::components::notice::inline_error;
use crate::components::notice::{deleted_notice, diagnostic_pane, missing_notice};
use crate::components::page_header::{Crumb, PageHeader};
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::tab_bar::tab_link;
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::pages::demographics::{PartyKind, browse_href};
use crate::uid::container_uid_of;

/// The noun phrase every party write-failure toast is built around
/// ([`crate::feedback::write_failure_copy`]).
const PARTY_OBJECT: &str = "this party";

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
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    let uid = container_uid_of(&uid);
    if uid.is_empty() {
        return Err(ViewerError::Invalid("a party id is required".to_owned()));
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
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
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
        return Err(ViewerError::Internal(
            "the CDR created the party but returned no uid to open it by".to_owned(),
        ));
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
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(
            "the current version uid is required to update this party — reload this tab and retry"
                .to_owned(),
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
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    let version_uid = version_uid.trim();
    if !version_uid.contains("::") {
        return Err(ViewerError::Invalid(
            "deleting a party needs the latest version's full OBJECT_VERSION_ID — reload this \
             screen and retry"
                .to_owned(),
        ));
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

#[cfg(feature = "ssr")]
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
    let doc: Value = serde_json::from_str(body.trim())
        .map_err(|e| ViewerError::Invalid(format!("the party body is not valid JSON: {e}")))?;
    let object = doc
        .as_object()
        .ok_or_else(|| ViewerError::Invalid("the party body must be a JSON object".to_owned()))?;
    let declared = object
        .get("_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if declared != kind.rm_type() {
        return Err(ViewerError::Invalid(format!(
            "this is the {} route, but the body declares _type {declared:?} — create it under \
             /demographics/{} instead, or fix the _type",
            kind.rm_type(),
            declared.to_lowercase()
        )));
    }
    Ok(doc.to_string())
}

#[cfg(feature = "ssr")]
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
    let doc: Value = serde_json::from_str(body)
        .map_err(|e| ViewerError::Internal(format!("party JSON: {e}")))?;
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

#[cfg(feature = "ssr")]
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
    let value: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("identities is not valid JSON: {e}"))?;
    match value.as_array() {
        Some(items) if !items.is_empty() => Ok(value),
        Some(_) => Err(
            "identities must hold at least one PARTY_IDENTITY — a party with no \
                        identity is invalid (PARTY invariant Identities_valid)"
                .to_owned(),
        ),
        None => Err("identities must be a JSON array of PARTY_IDENTITY objects".to_owned()),
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
        ViewerError::Invalid(format!(
            "the loaded party document is not valid JSON ({e}) — reload this tab and retry"
        ))
    })?;
    let identities = parse_identities(identities).map_err(ViewerError::Invalid)?;
    let details = parse_details(details).map_err(ViewerError::Invalid)?;
    let object = doc.as_object_mut().ok_or_else(|| {
        ViewerError::Invalid(
            "the loaded party document is not a JSON object — reload this tab and retry".to_owned(),
        )
    })?;
    drop(object.insert("identities".to_owned(), identities));
    match details {
        Some(details) => drop(object.insert("details".to_owned(), details)),
        None => drop(object.remove("details")),
    }
    serde_json::to_string(&doc)
        .map_err(|e| ViewerError::Invalid(format!("the edited party could not be serialized: {e}")))
}

/// The party-detail screen: the header, the delete affordance, the tab bar, and
/// four always-mounted, visibility-toggled tab bodies.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn PartyDetailPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let segment = Memo::new(move |_| params.with(|p| p.get("kind").unwrap_or_default()));
    let kind = Memo::new(move |_| segment.with(|s| PartyKind::from_segment(s)));
    let uid = Signal::derive(move || {
        container_uid_of(&params.with(|p| p.get("uid").unwrap_or_default()))
    });
    let query = leptos_router::hooks::use_query_map();
    // Tab state lives in the URL (`?tab=`): shareable, refresh-safe.
    let selected: Memo<String> = Memo::new(move |_| {
        query
            .with(|q| q.get("tab"))
            .filter(|tab| !tab.is_empty())
            .unwrap_or_else(|| "party".to_owned())
    });

    // The kind is read REACTIVELY and the screen is rebuilt when it changes:
    // `leptos_router` answers a navigation to the same `<Route>` by updating the
    // params without re-running this body ("if two IDs are the same, we do not
    // rerender, but only update the params" — `leptos_router` 0.8
    // `src/nested_router.rs`), and a relationship end on the Relationships tab
    // links party → party, so a once-read kind would address the wrong family.
    view! {
        {move || {
            super::browse::kinded_screen(kind, segment, |kind| party_screen(kind, uid, selected))
        }}
    }
}

/// One party's screen: the header, the delete affordance, the tab bar, and the
/// four always-mounted, visibility-toggled tab bodies.
fn party_screen(kind: PartyKind, uid: Signal<String>, selected: Memo<String>) -> AnyView {
    // The screen's ONE read of the party. Its latest version uid goes into
    // `latest_version` for the delete above the tabs, and the resource itself is
    // shared with the Relationships tab, whose list is a projection of the same
    // document — reading it again would be the same claim twice.
    let latest_version = RwSignal::new(String::new());
    let (party, party_resource) = party_section(kind, uid, latest_version);
    let history = super::history::history_section(
        super::DemographicResource::Party(kind),
        uid,
        selected,
        "history",
    );
    let relationships = super::relationship::party_relationships_section(kind, uid, party_resource);
    let party_tags = super::tags::tags_section(kind, uid, selected);
    let delete = delete_section(kind, uid, latest_version);
    let tabs = tab_bar(kind, uid, selected);

    let heading = Signal::derive(move || {
        let id = uid.get();
        let short: String = id.chars().take(8).collect();
        format!("{} {short}…", kind.rm_type())
    });

    view! {
        <Title text="Party" />
        <div class="p-6">
            <PageHeader
                title=heading
                crumbs=vec![
                    Crumb::new("Demographics", browse_href(kind)),
                    Crumb::new(kind.plural(), browse_href(kind)),
                ]
                mono=true
            />
            {delete}
            {tabs}
            <div class="mt-4">
                <div class:hidden=move || selected.get() != "party">{party}</div>
                <div class:hidden=move || selected.get() != "history">{history}</div>
                <div class:hidden=move || {
                    selected.get() != "relationships"
                }>{relationships}</div>
                <div class:hidden=move || selected.get() != "tags">{party_tags}</div>
            </div>
        </div>
    }
    .into_any()
}

/// The URL-driven tab bar: four pill anchors (`?tab=…`). Plain anchors keep the
/// tabs working before hydration (the router intercepts them once WASM loads).
fn tab_bar(kind: PartyKind, uid: Signal<String>, selected: Memo<String>) -> AnyView {
    let link = move |value: &'static str, label: &'static str| {
        tab_link(
            move || {
                format!(
                    "{}?tab={value}",
                    super::party_href(kind, uid.get().as_str())
                )
            },
            label,
            Signal::derive(move || selected.get() == value),
        )
    };
    view! {
        <div class="flex flex-wrap gap-1 border-b border-edge pb-2">
            {link("party", "Party")} {link("history", "History")}
            {link("relationships", "Relationships")} {link("tags", "Tags")}
        </div>
    }
    .into_any()
}

/// The current-party resource shared by the tab's sections.
pub type PartyResource = Resource<Result<Option<PartyState>, ViewerError>>;

/// One dispatched party edit: the target, the `If-Match` version, the verbatim
/// document the edits apply to, and the two edited drafts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PartyEdit {
    /// The party family being written.
    kind: PartyKind,
    /// The version container the PUT addresses.
    versioned_object_uid: String,
    /// The loaded version's `OBJECT_VERSION_ID` — the `If-Match` value.
    version_uid: String,
    /// The loaded document, verbatim — the merge base.
    base_body: String,
    /// The new `identities`, as JSON text.
    identities: String,
    /// The new `details`, as JSON text; blank removes the attribute.
    details: String,
}

/// The edit form's long-lived reactive state, created ONCE in
/// [`party_section`] — ABOVE the `<Transition>`, so it outlives every Suspend
/// re-run — and re-seeded idempotently per loaded version by [`seed`].
#[derive(Clone, Copy)]
struct PartyForm {
    /// The `identities` draft.
    identities: RwSignal<String>,
    /// The `details` draft (blank = remove the attribute).
    details: RwSignal<String>,
    /// The loaded version's `OBJECT_VERSION_ID` — the `If-Match` a save sends.
    version_uid: RwSignal<String>,
    /// The version container a save addresses.
    versioned_object_uid: RwSignal<String>,
    /// The loaded document, verbatim — the merge base a save sends.
    base_body: RwSignal<String>,
    /// The client-side complaint about a draft; `None` while both are
    /// acceptable.
    validation: RwSignal<Option<String>>,
    /// The version this state was last seeded from; [`seed`] is a no-op while
    /// it already equals the loaded version, so a Suspend re-run for the SAME
    /// version never overwrites edits in progress.
    seeded_uid: RwSignal<Option<String>>,
}

impl std::fmt::Debug for PartyForm {
    /// Signal handles carry no readable content outside a reactive owner, so
    /// the `Debug` impl names the type only — and deliberately never a
    /// clinical or demographic value (the PHI caveat in
    /// `.claude/rules/reliability.md`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PartyForm")
    }
}

impl PartyForm {
    /// Create the form's long-lived state, empty until the first [`seed`].
    fn new() -> Self {
        Self {
            identities: RwSignal::new(String::new()),
            details: RwSignal::new(String::new()),
            version_uid: RwSignal::new(String::new()),
            versioned_object_uid: RwSignal::new(String::new()),
            base_body: RwSignal::new(String::new()),
            validation: RwSignal::new(None),
            seeded_uid: RwSignal::new(None),
        }
    }
}

/// Seed [`PartyForm`] from the freshly loaded party, ONCE per loaded version.
///
/// A Suspend re-run for the SAME version is a no-op (the state lives above
/// the Suspend, so re-seeding would overwrite edits in progress); a NEW
/// version resets both drafts, the merge base, the `If-Match` value and the
/// validation note.
fn seed(form: PartyForm, state: &PartyState) {
    if form.seeded_uid.get_untracked().as_deref() == Some(state.version_uid.as_str()) {
        return;
    }
    form.identities.set(state.identities.clone());
    form.details.set(state.details.clone());
    form.version_uid.set(state.version_uid.clone());
    form.versioned_object_uid
        .set(state.versioned_object_uid.clone());
    form.base_body.set(state.body.clone());
    form.validation.set(None);
    form.seeded_uid.set(Some(state.version_uid.clone()));
}

/// The Party tab: the facts card, the edit form, and the document.
///
/// ONE resource, created in setup — the screen's single reader of the current
/// party document, which is why it is NOT gated on the tab: the delete
/// affordance above the tabs needs the latest version uid whichever tab is open.
/// It publishes that uid into `latest_version` for the delete section to read.
///
/// The source carries a stamp that advances ONLY on a successful save, so a
/// refused save (a `412`, a rejected body) leaves the operator's input on screen
/// instead of re-seeding the form from the server.
///
/// Returns the tab's view AND its resource, so the Relationships tab can project
/// the same document instead of fetching it again.
fn party_section(
    kind: PartyKind,
    uid: Signal<String>,
    latest_version: RwSignal<String>,
) -> (AnyView, PartyResource) {
    let toaster = thaw::ToasterInjection::expect_context();

    // Created BEFORE the resource: its stamp is the resource's refetch trigger.
    let save: Action<PartyEdit, Result<String, ViewerError>> = Action::new(|edit: &PartyEdit| {
        let edit = edit.clone();
        async move {
            update_party(
                edit.kind.segment().to_owned(),
                edit.versioned_object_uid,
                edit.version_uid,
                edit.base_body,
                edit.identities,
                edit.details,
            )
            .await
        }
    });
    // `Action::version` increments on failures too; a refetch after a REFUSED
    // save would re-seed the form and discard the edits the operator still
    // needs.
    let saved = Memo::new(move |prev: Option<&usize>| {
        let version = save.version().get();
        if save.value().with(|value| matches!(value, Some(Ok(_)))) {
            version
        } else {
            prev.copied().unwrap_or(0)
        }
    });

    let resource: PartyResource = Resource::new(
        move || (uid.get(), saved.get()),
        move |(id, _)| async move { fetch_party(kind.segment().to_owned(), id).await },
    );

    // Both outcomes toast (the viewer's mutation-feedback rule); a `412` is
    // the mid-air collision and gets its own title.
    Effect::new(move |_| match save.value().get() {
        Some(Ok(uid)) => {
            let detail = if uid.is_empty() {
                "A new version was committed.".to_owned()
            } else {
                format!("New version {uid}")
            };
            toast_success(toaster, "Party updated", &detail);
        }
        Some(Err(error)) => {
            let title = if error.status_code() == Some(http::StatusCode::PRECONDITION_FAILED) {
                "Party changed on the server"
            } else {
                "Save failed"
            };
            crate::feedback::toast_write_failure(toaster, title, PARTY_OBJECT, &error);
        }
        None => {}
    });

    let form = PartyForm::new();
    let facts = facts_section(resource, form, latest_version);
    let editor = edit_form(kind, form, save);
    // A failed read renders nothing in the pane — the facts section above
    // states it once (the screen never renders an error as nothing).
    let document = document_section(resource, "party-document", |state| state.body.as_str());
    (
        view! { <div class="flex flex-col gap-4">{facts} {editor} {document}</div> }.into_any(),
        resource,
    )
}

/// The party's facts — its type, name, archetype, version and counts — plus the
/// idempotent re-seed of the edit form from the freshly loaded version.
///
/// A `<Transition>` (not `<Suspense>`): the resource reloads after every
/// successful save and the previous facts must stay visible instead of flashing
/// the skeleton. The `Result` resolves INSIDE the transition.
fn facts_section(
    resource: PartyResource,
    form: PartyForm,
    latest_version: RwSignal<String>,
) -> AnyView {
    view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(Some(state)) => {
                        seed(form, &state);
                        latest_version.set(state.version_uid.clone());
                        facts_card(&state)
                    }
                    Ok(None) => deleted_card(),
                    Err(e) if e.status_code() == Some(http::StatusCode::NOT_FOUND) => {
                        missing_notice(
                            "party-not-found",
                            "The CDR holds no party of this kind with this id. Check the id and the kind — the same id under a different kind is a different resource.",
                        )
                    }
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any()
}

/// The `204` state: the party exists but its current version is deleted.
fn deleted_card() -> AnyView {
    deleted_notice(
        "party-deleted",
        "This party's current version is deleted. Its earlier versions are still readable — open one from the History tab.",
    )
}

/// Render the loaded party's facts as a card.
fn facts_card(state: &PartyState) -> AnyView {
    let facts = vec![
        fact_row("type", "type", state.rm_type.clone()),
        fact_row("name", "name", state.name.clone()),
        fact_row("archetype", "archetype", state.archetype_node_id.clone()),
        fact_row("version", "version", state.version_uid.clone()),
        fact_row("identities", "identities", state.identity_count.to_string()),
        fact_row(
            "relationships (inline)",
            "relationships",
            state.relationships.len().to_string(),
        ),
    ];
    view! {
        <section class=format!("{CARD_PAD} flex flex-col gap-3") id="party-facts">
            <div class="grid grid-cols-1 sm:grid-cols-2 items-start gap-x-4 gap-y-2 text-sm">
                {facts}
            </div>
        </section>
    }
    .into_any()
}

/// One label/value line of a fact card, carrying the demographic screens'
/// `data-demographic-fact` hook.
pub(super) fn fact_row(label: &'static str, hook: &'static str, value: String) -> AnyView {
    crate::components::facts::fact_row(label, "data-demographic-fact", hook, value)
}

/// The edit card: the two JSON drafts, the save button, and the two inline
/// feedback places (the client-side complaint and the CDR's verbatim
/// diagnostic).
///
/// Always mounted with a constant structure, so the server HTML and the client
/// view match; every value comes from the long-lived [`PartyForm`], so the
/// card survives the facts section's Suspend re-runs.
///
/// Both drafts and the save are DISABLED until [`seed`] has loaded a document
/// into the form: the card is mounted before the read resolves, and an edit
/// typed into it then would be silently replaced by the seed.
#[expect(
    clippy::too_many_lines,
    reason = "one erased section: the edit card's two drafts + validation + action wiring (rules §1)"
)]
fn edit_form(
    kind: PartyKind,
    form: PartyForm,
    save: Action<PartyEdit, Result<String, ViewerError>>,
) -> AnyView {
    let unseeded = Signal::derive(move || form.seeded_uid.with(Option::is_none));
    let on_save = move |_| {
        let identities = form.identities.get();
        let details = form.details.get();
        // Client-side validation first, before any round trip; the server fn
        // re-checks — it is a public endpoint.
        if let Err(message) = draft_complaint(&identities, &details) {
            form.validation.set(Some(message));
        } else {
            form.validation.set(None);
            save.dispatch(PartyEdit {
                kind,
                versioned_object_uid: form.versioned_object_uid.get(),
                version_uid: form.version_uid.get(),
                base_body: form.base_body.get(),
                identities,
                details,
            });
        }
    };
    let validation = move || {
        match form.validation.get() {
        Some(message) => view! {
            <div
                role="alert"
                id="party-validation"
                class="rounded-control border border-danger/40 bg-danger-subtle px-3 py-2 text-sm text-danger"
            >
                {message}
            </div>
        }
        .into_any(),
        None => ().into_any(),
    }
    };
    // The CDR's own diagnostic, kept beside the form it refused: the toast is
    // the notification, this is the detail worth reading line by line.
    let diagnostic = diagnostic_pane(
        "party-diagnostic",
        Signal::derive(move || save.value().get().and_then(Result::err)),
    );
    let hint = format!(
        "Commits a new {} version on top of the one loaded above (If-Match), so a concurrent \
         change is refused rather than overwritten. Every other attribute travels back exactly as \
         the CDR served it.",
        kind.rm_type()
    );
    view! {
        <section class=CARD_PAD id="party-edit">
            <h2 class=CARD_TITLE>"Edit party"</h2>
            <p class="mb-3 text-xs text-ink-muted">{hint}</p>
            <div class="flex flex-col gap-3">
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="party-identities">
                        "identities (canonical JSON array of PARTY_IDENTITY — at least one)"
                    </label>
                    <textarea
                        id="party-identities"
                        class=format!("{TEXTAREA} min-h-[10rem]")
                        disabled=true
                        prop:disabled=move || unseeded.get()
                        prop:value=move || form.identities.get()
                        on:input:target=move |ev| form.identities.set(ev.target().value())
                    >
                        {form.identities.get_untracked()}
                    </textarea>
                </div>
                <div class="flex flex-col gap-1">
                    <label class=LABEL r#for="party-details">
                        "details (canonical JSON ITEM_STRUCTURE — leave blank to remove)"
                    </label>
                    <textarea
                        id="party-details"
                        class=format!("{TEXTAREA} min-h-[8rem]")
                        placeholder="{ \"_type\": \"ITEM_TREE\", \"archetype_node_id\": \"at0001\", … }"
                        disabled=true
                        prop:disabled=move || unseeded.get()
                        prop:value=move || form.details.get()
                        on:input:target=move |ev| form.details.set(ev.target().value())
                    >
                        {form.details.get_untracked()}
                    </textarea>
                </div>
                <div class="flex items-center gap-3">
                    <button
                        id="party-save"
                        type="button"
                        class=BTN_PRIMARY
                        disabled=true
                        prop:disabled=move || unseeded.get() || save.pending().get()
                        on:click=on_save
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuSave width="14" height="14" />
                        "Save party"
                    </button>
                    <Show when=move || save.pending().get()>
                        <span class="text-sm text-ink-muted">"Saving…"</span>
                    </Show>
                </div>
                {validation}
                {diagnostic}
            </div>
        </section>
    }
    .into_any()
}

/// The client-side shape complaint about the two drafts, or `Ok(())` when both
/// are acceptable.
///
/// It is the SAME judgement [`apply_party_edits`] makes server-side, called
/// rather than restated: [`parse_identities`] and [`parse_details`] are pure and
/// compile on both targets, so the two can never disagree.
///
/// # Errors
/// The operator-facing complaint naming the offending draft.
fn draft_complaint(identities: &str, details: &str) -> Result<(), String> {
    drop(parse_identities(identities)?);
    drop(parse_details(details)?);
    Ok(())
}

/// The **Delete party** affordance above the tab bar.
///
/// The click opens the shared confirmation modal
/// ([`ConfirmDialog`](crate::components::confirm_dialog::ConfirmDialog)), whose
/// copy states what a logical delete does: it commits a `523|deleted|` version,
/// so the party stops resolving as current while every earlier version stays
/// readable by its own uid (RM common master06 §Logical Deletion). On success
/// the viewer returns to the kind's browser screen with a toast.
///
/// `version_uid` is the screen's ONE read of the party, published by
/// [`facts_section`]: the delete addresses the version to supersede, not the
/// container ("MUST be in a form of an `OBJECT_VERSION_ID` … representing the
/// `preceding_version_uid` to be deleted", `operations/person_delete.yaml`).
fn delete_section(kind: PartyKind, uid: Signal<String>, version_uid: RwSignal<String>) -> AnyView {
    let delete: Action<String, Result<(), ViewerError>> = Action::new(move |version: &String| {
        let version = version.clone();
        async move { delete_party(kind.segment().to_owned(), version).await }
    });
    let message = Signal::derive(move || {
        format!(
            "Delete {} {}? This commits a deleted version: the party stops resolving as current, \
             and every earlier version stays readable by its own version uid.",
            kind.rm_type(),
            uid.get()
        )
    });
    logical_delete_section(
        delete,
        version_uid,
        message,
        browse_href(kind),
        LogicalDeleteCopy {
            button_id: "party-delete",
            label: "Delete party",
            confirm_id: "party-delete-confirm",
            success_title: "Party deleted",
            object: PARTY_OBJECT,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::draft_complaint;

    #[test]
    fn an_identities_array_with_one_entry_is_accepted() {
        let identities = r#"[{"_type":"PARTY_IDENTITY","name":{"_type":"DV_TEXT","value":"legal identity"},"archetype_node_id":"at0002","details":{"_type":"ITEM_TREE","name":{"_type":"DV_TEXT","value":"tree"},"archetype_node_id":"at0003"}}]"#;
        assert_eq!(draft_complaint(identities, ""), Ok(()));
        assert_eq!(
            draft_complaint(identities, r#"{"_type":"ITEM_TREE"}"#),
            Ok(())
        );
    }

    #[test]
    fn an_empty_or_non_array_identities_draft_is_refused() {
        // PARTY invariant Identities_valid: "not `identities.is_empty`".
        let message = draft_complaint("[]", "").expect_err("an empty list is refused");
        assert!(message.contains("at least one PARTY_IDENTITY"), "{message}");
        for draft in ["{}", "\"x\"", "42", "null"] {
            let message = draft_complaint(draft, "").expect_err("a non-array is refused");
            assert!(message.contains("must be a JSON array"), "{message}");
        }
        let message = draft_complaint("[", "").expect_err("malformed JSON is refused");
        assert!(message.contains("not valid JSON"), "{message}");
    }

    #[test]
    fn a_non_object_details_draft_is_refused_and_a_blank_one_is_not() {
        let identities = r#"[{"_type":"PARTY_IDENTITY"}]"#;
        // details is an ITEM_STRUCTURE (0..1): an object, or absent.
        assert_eq!(draft_complaint(identities, "   "), Ok(()));
        for draft in ["[]", "\"ITEM_TREE\"", "7", "true"] {
            let message = draft_complaint(identities, draft).expect_err("a non-object is refused");
            assert!(message.contains("must be a JSON object"), "{message}");
        }
        let message =
            draft_complaint(identities, "{\"_type\":").expect_err("malformed JSON is refused");
        assert!(message.contains("not valid JSON"), "{message}");
    }
}

#[cfg(all(test, feature = "ssr"))]
mod wire_tests {
    use super::{apply_party_edits, checked_party_body, parse_party_state};
    use crate::pages::demographics::PartyKind;
    use serde_json::Value;

    /// A minimal RM-valid PERSON as the wire serves it — the `uid` is the
    /// enclosing VERSION's `OBJECT_VERSION_ID` (PARTY `Uid_mandatory`).
    const PERSON: &str = r#"{
        "_type": "PERSON",
        "name": {"_type": "DV_TEXT", "value": "PERSON"},
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::2"},
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": {"_type": "DV_TEXT", "value": "legal identity"},
            "archetype_node_id": "at0002",
            "details": {"_type": "ITEM_TREE", "name": {"_type": "DV_TEXT", "value": "tree"}, "archetype_node_id": "at0003"}
        }],
        "contacts": [{"_type": "CONTACT", "name": {"_type": "DV_TEXT", "value": "home"}, "archetype_node_id": "at0004", "addresses": []}],
        "details": {"_type": "ITEM_TREE", "archetype_node_id": "at0001", "items": []}
    }"#;

    #[test]
    fn parses_the_partys_facts_and_keeps_the_body_verbatim() {
        let state = parse_party_state(PERSON, None).expect("a valid PERSON");
        assert_eq!(state.rm_type, "PERSON");
        assert_eq!(state.name, "PERSON");
        assert_eq!(
            state.archetype_node_id,
            "openEHR-DEMOGRAPHIC-PERSON.person.v1"
        );
        assert_eq!(state.version_uid, "8849182c::example.org::2");
        // The routes address the CONTAINER; the served uid is a version id.
        assert_eq!(state.versioned_object_uid, "8849182c");
        assert_eq!(state.identity_count, 1);
        assert!(state.relationships.is_empty());
        // The body is kept VERBATIM — it is the merge base of every edit.
        assert_eq!(state.body, PERSON);
        // Both drafts reach their textareas pretty-printed.
        assert!(state.identities.contains("PARTY_IDENTITY") && state.identities.contains('\n'));
        assert!(state.details.contains("ITEM_TREE"));
    }

    #[test]
    fn the_party_precondition_is_the_served_etag_not_the_body_uid() {
        // The `ETag` is the identifier the CDR offers for the conditional
        // round-trip (ITS-REST overview §ETag and Last-Modified); the update's
        // `If-Match` echoes exactly it, and the container follows from it.
        let state =
            parse_party_state(PERSON, Some("8849182c::example.org::5")).expect("a valid PERSON");
        assert_eq!(state.version_uid, "8849182c::example.org::5");
        assert_eq!(state.versioned_object_uid, "8849182c");
        // Without an ETag the document's own uid is all there is.
        assert_eq!(
            parse_party_state(PERSON, None)
                .expect("a valid PERSON")
                .version_uid,
            "8849182c::example.org::2"
        );
    }

    #[test]
    fn a_sparse_party_reads_as_empty_facts_and_a_bad_body_errors() {
        let state = parse_party_state(r#"{"_type":"ROLE"}"#, None).expect("an object parses");
        assert_eq!(state.rm_type, "ROLE");
        assert_eq!(state.version_uid, "");
        assert_eq!(state.versioned_object_uid, "");
        assert_eq!(state.identities, "");
        assert_eq!(state.identity_count, 0);
        assert!(parse_party_state("not json", None).is_err());
    }

    #[test]
    fn the_two_edited_attributes_change_and_everything_else_survives() {
        let merged = apply_party_edits(
            PERSON,
            r#"[{"_type":"PARTY_IDENTITY","archetype_node_id":"at0009"}]"#,
            r#"{"_type":"ITEM_TREE","archetype_node_id":"at0002","items":[]}"#,
        )
        .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert_eq!(doc["identities"][0]["archetype_node_id"], "at0009");
        assert_eq!(doc["details"]["archetype_node_id"], "at0002");
        // Untouched attributes travel back verbatim — the whole point of
        // merging into the served document instead of rebuilding it.
        assert_eq!(doc["_type"], "PERSON");
        assert_eq!(doc["name"]["value"], "PERSON");
        assert_eq!(doc["uid"]["value"], "8849182c::example.org::2");
        assert_eq!(doc["contacts"][0]["name"]["value"], "home");
        assert_eq!(
            doc["archetype_node_id"],
            "openEHR-DEMOGRAPHIC-PERSON.person.v1"
        );
    }

    #[test]
    fn a_blank_details_draft_removes_the_attribute() {
        let merged = apply_party_edits(PERSON, r#"[{"_type":"PARTY_IDENTITY"}]"#, "")
            .expect("the merge succeeds");
        let doc: Value = serde_json::from_str(&merged).expect("merged JSON");
        assert!(doc.get("details").is_none());
        assert_eq!(doc["contacts"][0]["name"]["value"], "home");
    }

    #[test]
    fn a_refused_draft_or_base_never_reaches_the_wire() {
        assert!(apply_party_edits("not json", "[{}]", "").is_err());
        assert!(apply_party_edits("[]", "[{}]", "").is_err());
        assert!(apply_party_edits(PERSON, "[]", "").is_err());
        assert!(apply_party_edits(PERSON, "[{}]", "[]").is_err());
    }

    #[test]
    fn a_create_body_must_declare_the_routed_kinds_rm_type() {
        let body = checked_party_body(r#"{"_type":"PERSON"}"#, PartyKind::Person)
            .expect("the routed type is accepted");
        assert!(body.contains("PERSON"));
        let message = checked_party_body(r#"{"_type":"ROLE"}"#, PartyKind::Person)
            .expect_err("a different type is refused before the round trip");
        assert!(format!("{message}").contains("ROLE"), "{message}");
        assert!(checked_party_body("[]", PartyKind::Person).is_err());
        assert!(checked_party_body("not json", PartyKind::Person).is_err());
    }
}
