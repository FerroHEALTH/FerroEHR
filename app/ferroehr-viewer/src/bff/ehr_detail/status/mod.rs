// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Status tabs: the EHR's `EHR_STATUS` resource, read, edited,
//! and walked version by version.
//!
//! Two tabs, one resource family:
//!
//! - **Status** — the CURRENT `EHR_STATUS` (`GET /ehr/{ehr_id}/ehr_status`):
//!   the queryable/modifiable badges, the subject, the whole document in a
//!   [`DocumentPane`], and the [`edit`] form that commits a new version
//!   (`PUT /ehr/{ehr_id}/ehr_status` with `If-Match`).
//! - **Status history** — the `VERSIONED_EHR_STATUS` family ([`history`]): the
//!   container + selected-VERSION envelope facts, the revision-history table, a
//!   `version_at_time` lookup, and any version's document pinned by its
//!   `OBJECT_VERSION_ID` (`GET /ehr/{ehr_id}/ehr_status/{version_uid}`).
//!
//! One reader per claim: the **Status** tab is the viewer's ONE reader of the
//! current status document; the **Status history** tab never touches that
//! endpoint — it reads the versioned family for the commit history and the
//! VERSION envelope, and pins a document by an explicit `version_uid`.
//!
//! No openEHR spec governs the viewer — our own design / product extension. The
//! wire it reads and writes IS spec-bound: the `EHR_STATUS` +
//! `VERSIONED_EHR_STATUS` operations (the ITS-REST EHR API's `EHR_STATUS` +
//! `VERSIONED_EHR_STATUS` families) over the RM `EHR_STATUS`
//! (`docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`).
//! Path segments are percent-encoded server-side; every `#[server]` fn below
//! authenticates the viewer session first, and the CDR credential never reaches
//! client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

use crate::bff::wire::{VersionEntry, VersionedObjectFacts};
use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
use crate::locale::{Msg, tr_with};
pub mod edit;

/// The viewer's view of an EHR's CURRENT `EHR_STATUS`.
///
/// The canonical document verbatim, the version that document IS, and the
/// facts the edit form works on — flattened BFF-side so the browser never
/// re-models the RM and so the type carries no `usize`.
///
/// The attributes are the RM `EHR_STATUS` class's own
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr_status.adoc`):
/// `is_queryable`, `is_modifiable`, the `subject` `PARTY_PROXY`, and the
/// optional `other_details` `ITEM_STRUCTURE`. `uid` is the `OBJECT_VERSION_ID`
/// of the served version — `EHR_STATUS` is `VERSIONABLE` (RM
/// `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`) —
/// and that value is the `If-Match` an update must carry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EhrStatusState {
    /// The canonical `EHR_STATUS` JSON body exactly as the CDR served it — the
    /// base every edit is applied to, so nothing outside the edited fields can
    /// be lost.
    pub body: String,
    /// The served version's `OBJECT_VERSION_ID` — the `If-Match` value of an
    /// update. Read from the `ETag` the CDR answered, falling back to the
    /// document's own `uid.value` when it sent none; empty when neither
    /// carries one.
    pub version_uid: String,
    /// `EHR_STATUS.is_queryable`.
    pub is_queryable: bool,
    /// `EHR_STATUS.is_modifiable`.
    pub is_modifiable: bool,
    /// The subject's external reference id (`subject.external_ref.id.value`),
    /// empty for a bare `PARTY_SELF` subject.
    pub subject: String,
    /// The issuing namespace of that reference
    /// (`subject.external_ref.namespace`), empty when the subject carries no
    /// external reference.
    pub subject_namespace: String,
    /// `EHR_STATUS.other_details` pretty-printed as canonical JSON, empty when
    /// the status carries none (the attribute is optional).
    pub other_details: String,
}

/// The EHR's CURRENT `EHR_STATUS` (`GET /ehr/{ehr_id}/ehr_status`), flattened
/// into an [`EhrStatusState`].
///
/// This is the viewer's ONE reader of the current status document; the history
/// tab reads versions.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_ehr_status(
    /// The EHR whose current status document to read.
    ehr_id: String,
) -> Result<EhrStatusState, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/ehr_status", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    let served_etag = response.etag_version_uid();
    parse_status_state(&response.body, served_etag.as_deref())
}

/// One VERSION of the EHR's `EHR_STATUS`, pretty-printed for the document pane
/// (`GET /ehr/{ehr_id}/ehr_status/{version_uid}` — "retrieves a particular
/// version of the `EHR_STATUS` identified by `version_uid`",
/// `GET ehr/{ehr_id}/ehr_status/{version_uid}` — the ITS-REST EHR API).
///
/// `version_uid` is a full `OBJECT_VERSION_ID`; the history tab only ever passes
/// ids the CDR itself reported.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when no version is given; CDR transport errors pass
/// through; a non-2xx CDR answer (the `404` for an unknown
/// `ehr_id`/`version_uid` included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_ehr_status_version(
    /// The EHR holding the versioned status.
    ehr_id: String,
    /// The status version to read.
    version_uid: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let version_uid = version_uid.trim();
    if version_uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::VersionUidRequiredReadPastEhr,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/ehr_status/{}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(version_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    Ok(crate::bff::pretty_body(
        &body,
        crate::format::ReprFormat::CanonicalJson,
    ))
}

/// Commit a new `EHR_STATUS` version (`PUT /ehr/{ehr_id}/ehr_status`).
///
/// The body sent is `base_body` — the document this screen loaded — with exactly
/// three attributes replaced: `is_queryable`, `is_modifiable`, and
/// `other_details` (removed when the text is blank, since the attribute is
/// optional). Everything else, the `subject` included, travels back verbatim
/// (the merge is `edit::apply_status_edits`), so an edit can never silently
/// drop an attribute the viewer does not render.
///
/// `current_version_uid` is the identifier the seeding read's `ETag` named (its
/// `uid.value` only when the CDR sent no `ETag`) and travels quoted in
/// `If-Match`. The docs text mandates the header's EFFECT — a received
/// condition that evaluates to false "MUST" be answered `412 Precondition Failed`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §If-Match and accidental overwrites) — and is silent on its VALUE, which the
/// released OAS artifacts ground: "the existing latest `version_uid` of the
/// resource (i.e. the `preceding_version_uid`)", in a "format … always an
/// `version_uid` identifier enclosed by double quotes"
/// (`specifications/parameters/header/If-Match.yaml`), refused with the latest
/// `version_uid` in the `ETag` (`specifications/responses/412_EHR_STATUS.yaml`).
/// That `412` reaches the UI as [`ViewerError::Cdr`] and gets its own toast.
///
/// `Prefer: return=representation` asks for the updated resource, whose
/// `uid.value` is the new version; the operation also allows a bare `204`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §Representation-negotiation), so an empty answer is a success with no uid to
/// name.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on a missing version uid or an `other_details`
/// value that is not a JSON object; CDR transport errors pass through; any
/// non-2xx CDR answer (the `412` concurrency failure and the `400`/`422`
/// validation diagnostics, which the UI renders verbatim, included) normalizes
/// via [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn update_ehr_status(
    /// The EHR whose status to update.
    ehr_id: String,
    /// The version this edit is based on, sent as `If-Match`.
    current_version_uid: String,
    /// The served status document this edit merges into, verbatim.
    base_body: String,
    /// The replacement `is_queryable` flag.
    is_queryable: bool,
    /// The replacement `is_modifiable` flag.
    is_modifiable: bool,
    /// The replacement `other_details`, as JSON object text; empty leaves it out.
    other_details: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let current = current_version_uid.trim();
    if current.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::CurrentEhrStatusVersionUidRequired,
        )));
    }
    let body = edit::apply_status_edits(&base_body, is_queryable, is_modifiable, &other_details)?;
    let if_match = format!("\"{current}\"");
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/ehr_status", urlencoding::encode(&ehr_id)));
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

/// The `VERSIONED_EHR_STATUS`'s revision history, newest-first
/// (`GET /ehr/{ehr_id}/versioned_ehr_status/revision_history`).
///
/// The rows are the shared [`VersionEntry`] every History tab renders, parsed by
/// the same `crate::bff::composition::parse_versions`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown `ehr_id`
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the history is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_status_revision_history(
    /// The EHR whose status revision history to read.
    ehr_id: String,
) -> Result<Vec<VersionEntry>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/versioned_ehr_status/revision_history",
        urlencoding::encode(&ehr_id)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    // NOTE: REVISION_HISTORY.items arrives most-recent-LAST on the wire (RM
    // common, generic package — the `items` attribute documentation); the table
    // presents newest-first, so reverse here.
    crate::bff::composition::parse_versions(&response.body).map(|mut entries| {
        entries.reverse();
        entries
    })
}

/// Read the `VERSIONED_EHR_STATUS` container and one of its VERSIONS.
///
/// Two reads, one resource: `GET /ehr/{ehr_id}/versioned_ehr_status` for the
/// container (`GET ehr/{ehr_id}/versioned_ehr_status`) and the
/// VERSION read for the envelope — `…/version/{version_uid}` for an explicitly
/// selected version (`…/versioned_ehr_status/version/{version_uid}`) or
/// `…/version` for the current one when nothing is selected: with no
/// `version_at_time`, that operation "retrieves the _latest_ VERSION"
/// (`…/versioned_ehr_status/version?version_at_time=` — latest when omitted).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (a `404` for an unknown
/// `ehr_id`/version included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when either body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_versioned_status(
    /// The EHR holding the versioned status.
    ehr_id: String,
    /// The status version whose envelope facts to read.
    version_uid: String,
) -> Result<VersionedObjectFacts, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let ehr = urlencoding::encode(&ehr_id);
    let object_url = state
        .cdr
        .rest_v1(&format!("ehr/{ehr}/versioned_ehr_status"));
    let object_response = state
        .cdr
        .get(&session.credential, &object_url, "application/json")
        .await?;
    let object_body = crate::cdr::CdrClient::expect_success(object_response)?.body;

    let version_uid = version_uid.trim();
    let version_url = if version_uid.is_empty() {
        state
            .cdr
            .rest_v1(&format!("ehr/{ehr}/versioned_ehr_status/version"))
    } else {
        state.cdr.rest_v1(&format!(
            "ehr/{ehr}/versioned_ehr_status/version/{}",
            urlencoding::encode(version_uid)
        ))
    };
    let version_response = state
        .cdr
        .get(&session.credential, &version_url, "application/json")
        .await?;
    let version_body = crate::cdr::CdrClient::expect_success(version_response)?.body;
    parse_versioned_status(&object_body, &version_body)
}

/// Resolve the `OBJECT_VERSION_ID` of the `EHR_STATUS` VERSION extant at
/// `at_time` (a browser `datetime-local` value):
/// `GET /ehr/{ehr_id}/versioned_ehr_status/version?version_at_time=…` — "if
/// `version_at_time` is supplied, retrieves the VERSION extant _at specified
/// time_" (the at-time read defaults to the latest version when the parameter is omitted).
/// The `200` body is a VERSION envelope whose `uid.value` is the
/// `OBJECT_VERSION_ID` (RM common — a VERSION's `uid` IS an
/// `OBJECT_VERSION_ID`); that string is returned so the caller can pin it.
///
/// The `datetime-local` → RFC 3339 completion is the composition viewer's shared
/// `crate::format::datetime_local_to_rfc3339`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when `at_time` is empty; CDR transport errors pass
/// through; a non-2xx CDR answer (the `404` for no version at that time
/// included, which the UI renders as an inline note) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_status_version_at_time(
    /// The EHR holding the versioned status.
    ehr_id: String,
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
        "ehr/{}/versioned_ehr_status/version?version_at_time={}",
        urlencoding::encode(&ehr_id),
        urlencoding::encode(&at_time),
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(crate::uid::uid_value_of(&response.body))
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a canonical `EHR_STATUS` body into an [`EhrStatusState`], keeping the
/// body itself verbatim. Defensive throughout — an absent attribute reads as its
/// `false`/empty default rather than failing the tab.
///
/// `served_etag` is the identifier the CDR's own `ETag` named
/// ([`CdrResponse::etag_version_uid`](crate::cdr::CdrResponse::etag_version_uid));
/// it wins over the document's `uid.value`, because the header is what the
/// server offers for the conditional round-trip. `None` falls back to the body.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub(crate) fn parse_status_state(
    body: &str,
    served_etag: Option<&str>,
) -> Result<EhrStatusState, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::EhrStatusJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let other_details = doc
        .get("other_details")
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_default();
    Ok(EhrStatusState {
        body: body.to_owned(),
        version_uid: served_etag
            .map_or_else(|| crate::uid::uid_value_of_document(&doc), str::to_owned),
        is_queryable: doc
            .get("is_queryable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        is_modifiable: doc
            .get("is_modifiable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        subject: json_str(&doc, &["subject", "external_ref", "id", "value"]),
        subject_namespace: json_str(&doc, &["subject", "external_ref", "namespace"]),
        other_details,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a `VERSIONED_EHR_STATUS` body plus a VERSION body into
/// [`VersionedObjectFacts`]. Defensive throughout — an absent attribute reads
/// as empty rather than failing the card.
///
/// # Errors
/// [`ViewerError::Internal`] when either body is not valid JSON.
fn parse_versioned_status(
    object_body: &str,
    version_body: &str,
) -> Result<VersionedObjectFacts, ViewerError> {
    let object: Value = serde_json::from_str(object_body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::VersionedEhrStatusJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let version: Value = serde_json::from_str(version_body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::VersionJson3, &[("e", (e).to_string().into())]))
    })?;
    Ok(VersionedObjectFacts {
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
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Follow a chain of object keys to a string leaf, or an empty string when any
/// hop is absent or not a string.
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
    use super::{parse_status_state, parse_versioned_status};

    /// A canonical `EHR_STATUS` as the wire carries it — the shape of the
    /// example shape of RM `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc` §`EHR_STATUS`.
    const STATUS: &str = r#"{
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "name": {"_type": "DV_TEXT", "value": "EHR status"},
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::1"},
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": {"_type": "GENERIC_ID", "value": "p-42", "scheme": "local"}
            }
        },
        "is_queryable": true,
        "is_modifiable": false,
        "other_details": {"_type": "ITEM_TREE", "items": []}
    }"#;

    #[test]
    fn parses_the_status_flags_subject_version_and_other_details() {
        let state = parse_status_state(STATUS, None).expect("valid EHR_STATUS");
        assert_eq!(state.version_uid, "8849182c::example.org::1");
        assert!(state.is_queryable);
        assert!(!state.is_modifiable);
        assert_eq!(state.subject, "p-42");
        assert_eq!(state.subject_namespace, "demographic");
        // The body is kept verbatim: it is the merge base of every edit.
        assert_eq!(state.body, STATUS);
        // `other_details` reaches the editor pretty-printed.
        assert!(state.other_details.contains("\"ITEM_TREE\""));
        assert!(state.other_details.contains('\n'));
    }

    #[test]
    fn a_party_self_subject_and_absent_other_details_read_as_empty() {
        let body = r#"{
            "_type": "EHR_STATUS",
            "subject": {"_type": "PARTY_SELF"},
            "is_queryable": true,
            "is_modifiable": true
        }"#;
        let state = parse_status_state(body, None).expect("valid EHR_STATUS");
        assert_eq!(state.subject, "");
        assert_eq!(state.subject_namespace, "");
        assert_eq!(state.other_details, "");
        assert_eq!(state.version_uid, "");
        assert!(state.is_queryable && state.is_modifiable);
        // An external_ref with no namespace reads as the bare id.
        let no_namespace = parse_status_state(
            r#"{"_type":"EHR_STATUS","subject":{"_type":"PARTY_SELF",
                "external_ref":{"id":{"value":"p-7"}}}}"#,
            None,
        )
        .expect("valid EHR_STATUS");
        assert_eq!(no_namespace.subject, "p-7");
        assert_eq!(no_namespace.subject_namespace, "");
        // A sparse body never fails the tab; a non-JSON one does.
        assert!(parse_status_state("not json", None).is_err());
    }

    #[test]
    fn the_served_etag_is_the_precondition_and_the_body_uid_only_the_fallback() {
        // The `ETag` is what the CDR offers for the conditional round-trip
        // (ITS-REST overview §ETag and Last-Modified), so an update echoes it
        // back — never a value re-derived from the document.
        let state =
            parse_status_state(STATUS, Some("8849182c::example.org::7")).expect("valid EHR_STATUS");
        assert_eq!(state.version_uid, "8849182c::example.org::7");
        // Without an ETag the document's own uid is all there is.
        let fallback = parse_status_state(STATUS, None).expect("valid EHR_STATUS");
        assert_eq!(fallback.version_uid, "8849182c::example.org::1");
    }

    #[test]
    fn parses_the_versioned_container_and_its_version_envelope() {
        let object = r#"{
            "_type": "VERSIONED_EHR_STATUS",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "8849182c"},
            "owner_id": {
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "EHR",
                "id": {"_type": "HIER_OBJECT_ID", "value": "e1"}
            },
            "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
        }"#;
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::2"},
            "contribution": {"_type": "OBJECT_REF", "id": {"value": "c9"}},
            "lifecycle_state": {"_type": "DV_CODED_TEXT", "value": "complete"},
            "preceding_version_uid": {"value": "8849182c::example.org::1"},
            "signature": "-----BEGIN PGP SIGNATURE-----"
        }"#;
        let details = parse_versioned_status(object, version).expect("valid bodies");
        assert_eq!(details.object_uid, "8849182c");
        assert_eq!(details.owner_id, "e1");
        assert_eq!(details.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(details.version_id, "8849182c::example.org::2");
        assert_eq!(details.lifecycle_state, "complete");
        assert_eq!(details.preceding_version_uid, "8849182c::example.org::1");
        assert_eq!(details.contribution_uid, "c9");
        assert!(details.signed);
    }

    #[test]
    fn a_first_version_envelope_has_no_preceding_version_and_no_signature() {
        // RM common `org.openehr.rm.common.version.adoc` invariant
        // `Preceding_version_uid_validity`: absent exactly for a first version.
        let version = r#"{
            "_type": "ORIGINAL_VERSION",
            "uid": {"value": "8849182c::example.org::1"},
            "lifecycle_state": {"value": "complete"}
        }"#;
        let details = parse_versioned_status("{}", version).expect("valid bodies");
        assert_eq!(details.preceding_version_uid, "");
        assert!(!details.signed);
        assert_eq!(details.object_uid, "");
        assert!(parse_versioned_status("not json", "{}").is_err());
        assert!(parse_versioned_status("{}", "not json").is_err());
    }
}
