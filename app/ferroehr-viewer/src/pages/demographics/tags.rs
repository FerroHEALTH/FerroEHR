// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic `ITEM_TAG` surface: the space-wide tag index the browser
//! screen lists, and the per-party tag editor on the party detail's Tags tab.
//!
//! Both are released operations. The index is
//! `GET /demographic/tags`, whose list "can be filtered by the given one or
//! more `tag_key`, `tag_value`, `tag_target_path` query parameters" and where
//! "in case no such parameter is provided then all `ITEM_TAG` resources will be
//! retrieved" (`operations/demographic_tags_get.yaml`). The editor is the
//! per-kind trio `{kind}_tags_get` / `{kind}_tags_update` /
//! `{kind}_tags_delete` (`operations/person_tags_*.yaml`).
//!
//! The row type, the wire codec, the merge and the editor panel are the shared
//! kit ([`crate::components::item_tags`]), which carries the tag model's own
//! rules — whole-collection replace, `(key, target_path)` identity, no
//! conditional header — so this module is only the demographic ROUTES.
//!
//! The addressed collection here is the version CONTAINER's: the released
//! operation makes the container and a single VERSION two DISJOINT collections
//! of the same object (`operations/person_tags_get.yaml`), and a container tag
//! survives the party's next version, which is what an operator marking a party
//! means.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;

use crate::components::item_tags::{
    ItemTagRow, TagActions, TagEdit, TagList, TagPanelCopy, tag_panel,
};
use crate::error::ViewerError;
use crate::pages::demographics::PartyKind;

/// The demographic tag index (`GET /demographic/tags`), optionally filtered.
///
/// Each filter is sent only when non-empty
/// ([`tag_filter_query`](crate::components::item_tags::tag_filter_query)),
/// which is exactly the released "omitted parameter constrains nothing"
/// behaviour (`operations/demographic_tags_get.yaml`). The list spans every
/// party kind and both target forms; a no-match answer is `200 []`, never a
/// `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_demographic_tags(
    /// Exact `ITEM_TAG.key` filter; empty constrains nothing.
    key: String,
    /// Exact `ITEM_TAG.value` filter; empty constrains nothing.
    value: String,
    /// Exact `ITEM_TAG.target_path` filter; empty constrains nothing.
    target_path: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let query = crate::components::item_tags::tag_filter_query(&key, &value, &target_path);
    let url = state.cdr.rest_v1(&format!("demographic/tags{query}"));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::components::item_tags::parse_item_tags(&response.body)
}

/// One party's `ITEM_TAG`s
/// (`GET /demographic/{kind}/{uid_based_id}/tags`).
///
/// The addressed id is the version CONTAINER, so these are the
/// `VERSIONED_PARTY`'s own tags (module docs).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment; CDR transport errors
/// pass through; a non-2xx CDR answer (the `404` for an unknown id included)
/// normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_party_tags(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The party's version container id.
    uid: String,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    read_party_tags(&state, &session.credential, kind, &uid).await
}

#[cfg(feature = "ssr")]
/// Read one party's container tag collection.
///
/// The plain function behind [`fetch_party_tags`], so [`set_party_tag`]'s
/// read-modify-write calls THIS rather than another public endpoint (a server
/// fn is thin and the logic it shares lives in an ordinary function).
///
/// # Errors
/// CDR transport errors pass through; a non-2xx answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the list is not valid JSON.
async fn read_party_tags(
    state: &crate::state::AppState,
    credential: &crate::session::Credential,
    kind: PartyKind,
    uid: &str,
) -> Result<Vec<ItemTagRow>, ViewerError> {
    let url = tags_url(state, kind, uid);
    let response = state.cdr.get(credential, &url, "application/json").await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    crate::components::item_tags::parse_item_tags(&response.body)
}

/// Set one tag on a party, keeping every other tag it already carries
/// (`PUT /demographic/{kind}/{uid_based_id}/tags`).
///
/// Read-modify-write, because the operation replaces the whole collection: the
/// current tags are read and
/// [`merged_tag_body`](crate::components::item_tags::merged_tag_body) inserts
/// or replaces the requested `(key, target_path)` entry.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment or a blank key; CDR
/// transport errors pass through; any non-2xx CDR answer (the `422` for a tag
/// that breaks an `ITEM_TAG` invariant included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the current list is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn set_party_tag(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The party's version container id.
    uid: String,
    /// The tag key to set.
    key: String,
    /// The tag value; blank stores a tag with no value.
    value: String,
    /// The tag's `target_path`; blank tags the whole object.
    target_path: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    if key.trim().is_empty() {
        return Err(ViewerError::Invalid(
            "a tag key is required (ITEM_TAG invariant Inv_key_valid: the key is not empty)"
                .to_owned(),
        ));
    }
    let existing = read_party_tags(&state, &session.credential, kind, &uid).await?;
    let body = crate::components::item_tags::merged_tag_body(
        &existing,
        key.trim(),
        value.trim(),
        target_path.trim(),
    );
    let url = tags_url(&state, kind, &uid);
    // NOTE: clients are "strongly encouraged to always include the `Prefer`
    // request header explicitly" — ITS-REST
    // `specifications/docs/overview/Requests_and_responses.md` §Deprecated headers.
    let response = state
        .cdr
        .put(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[("Prefer", "return=minimal")],
            body,
        )
        .await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

/// Delete a party's tags carrying `key`
/// (`DELETE /demographic/{kind}/{uid_based_id}/tags/{key}`).
///
/// The released operation addresses a key, not a `(key, target_path)` pair, so
/// it removes every tag on the target that shares the key
/// (`operations/person_tags_delete.yaml`); an unknown key on an existing target
/// is its `404`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an unknown kind segment or a blank key; CDR
/// transport errors pass through; any non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn delete_party_tag(
    /// The party family, as its route segment ([`PartyKind::segment`]).
    kind: String,
    /// The party's version container id.
    uid: String,
    /// The tag key to remove.
    key: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let kind = PartyKind::from_segment(&kind)
        .ok_or_else(|| super::unknown_segment(&kind, "party kind"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(ViewerError::Invalid("a tag key is required".to_owned()));
    }
    let url = format!(
        "{}/{}",
        tags_url(&state, kind, &uid),
        urlencoding::encode(key)
    );
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    drop(crate::cdr::CdrClient::expect_success(response)?);
    Ok(())
}

#[cfg(feature = "ssr")]
/// The `/demographic/{kind}/{uid_based_id}/tags` URL of a party's container tag
/// collection.
fn tags_url(state: &crate::state::AppState, kind: PartyKind, uid: &str) -> String {
    state.cdr.rest_v1(&format!(
        "demographic/{}/{}/tags",
        kind.segment(),
        urlencoding::encode(&crate::uid::container_uid_of(uid))
    ))
}

/// The party detail's **Tags** tab, drawn by the shared tag kit.
///
/// One resource, created in setup and gated on the tab being active, refetched
/// after every successful write via the actions' version stamps.
pub(super) fn tags_section(
    kind: PartyKind,
    uid: Signal<String>,
    selected: Memo<String>,
) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let set: Action<(String, TagEdit), Result<(), ViewerError>> =
        Action::new(move |(uid, edit): &(String, TagEdit)| {
            let (uid, edit) = (uid.clone(), edit.clone());
            async move {
                set_party_tag(
                    kind.segment().to_owned(),
                    uid,
                    edit.key,
                    edit.value,
                    edit.target_path,
                )
                .await
            }
        });
    let remove: Action<(String, String), Result<(), ViewerError>> =
        Action::new(move |(uid, key): &(String, String)| {
            let (uid, key) = (uid.clone(), key.clone());
            async move { delete_party_tag(kind.segment().to_owned(), uid, key).await }
        });

    let resource: TagList = Resource::new(
        move || {
            (selected.get() == "tags")
                .then(|| (uid.get(), set.version().get(), remove.version().get()))
        },
        move |active| async move {
            match active {
                Some((id, _, _)) => fetch_party_tags(kind.segment().to_owned(), id)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        },
    );

    // Every write toasts both outcomes (the viewer's mutation-feedback rule).
    Effect::new(move |_| match set.value().get() {
        Some(Ok(())) => crate::components::toast::toast_success(
            toaster,
            "Tag saved",
            "The party's tag collection was replaced with the merged list.",
        ),
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(
                toaster,
                "Tag save failed",
                "this party's tags",
                &error,
            );
        }
        None => {}
    });
    Effect::new(move |_| match remove.value().get() {
        Some(Ok(())) => crate::components::toast::toast_success(
            toaster,
            "Tag deleted",
            "Every tag carrying that key was removed.",
        ),
        Some(Err(error)) => crate::components::toast::toast_error(
            toaster,
            "Tag delete failed",
            &crate::feedback::write_failure_copy("this party's tag", &error),
        ),
        None => {}
    });

    let actions = TagActions {
        set: Callback::new(move |edit: TagEdit| {
            set.dispatch((uid.get_untracked(), edit));
        }),
        delete: Callback::new(move |key: String| {
            remove.dispatch((uid.get_untracked(), key));
        }),
        busy: Signal::derive(move || set.pending().get() || remove.pending().get()),
    };
    tag_panel(
        TagPanelCopy {
            title: "Tags",
            note: "These are the VERSIONED_PARTY container's tags. Saving one re-sends the whole \
                   collection, which is how the openEHR tag update works — a tag another client \
                   added in the meantime can be lost, so reload before editing a busy party.",
            empty_message: "No tags on this party",
            empty_hint: "Tags are free key/value markers a client sets on a party; add one below.",
            form_id: "party-tag-set",
        },
        uid,
        resource,
        actions,
    )
}
