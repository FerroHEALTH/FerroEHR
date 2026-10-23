// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Commit tab: a staging area that commits several correlated
//! changes as ONE atomic CONTRIBUTION.
//!
//! `POST /ehr/{ehr_id}/contribution` is "the 'native' way of committing"
//! (ITS-REST
//! `specifications/docs/overview/Requests_and_responses.md` §"openehr-version
//! and openehr-audit-details"): one change set, several `VERSION`s, all of them
//! committed or none. The per-resource routes the other tabs use are
//! convenience wrappers over exactly this operation, so a correlated change —
//! a new COMPOSITION *and* the EHR status it belongs with — is one commit here
//! instead of two independent ones.
//!
//! The staging list is **viewer-session state only**: it lives in this tab's
//! component state, so navigating away discards it. The viewer stores nothing
//! of its own (§No viewer-local domain state), and the screen says so.
//!
//! Reads are reused, never re-implemented: the template list, the composition
//! list, an amend's seed and the status seed all come from the tabs that
//! already own them. The one WRITE is [`commit_contribution`]; the body it
//! posts is assembled by the component-free, unit-tested [`staged`] module.
//!
//! No openEHR spec governs the viewer — our own design / product extension.
//! The wire it writes IS spec-bound. The `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches client-visible state.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the carriers here are ssr-only, so #[expect] would be unfulfilled on the \
              hydrate target"
)]

pub mod staged;

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::Value;

use crate::error::ViewerError;
use crate::bff::ehr_detail::commit::staged::StagedChange;

/// What a successful commit reports back.
///
/// The CONTRIBUTION's own uid plus the `OBJECT_VERSION_ID` of every version the
/// commit MINTED — the `201` representation's `versions` are the `OBJECT_REF`s
/// of exactly those (ITS-REST
/// `specifications/responses/201_CONTRIBUTION.yaml`). All strings, so the type
/// is safe across the server-fn boundary on the 32-bit WASM target.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommitOutcome {
    /// `CONTRIBUTION.uid.value` — the id the contributions tab looks up.
    pub contribution_uid: String,
    /// The `OBJECT_VERSION_ID` of each version this commit created.
    pub version_uids: Vec<String>,
}

/// Commit the staged changes as ONE CONTRIBUTION
/// (`POST /ehr/{ehr_id}/contribution`).
///
/// The envelope is assembled by [`staged::contribution_body`] from the staged
/// rows; `committer` falls back to the viewer session's own identity when the
/// operator leaves it blank, because the wire REQUIRES one
/// (`specifications/schemas/common/UpdateAudit.yaml`: `required: [change_type,
/// committer]`) and the session identity is the honest answer to "who committed
/// this". `Prefer: return=representation` is sent so the answer names the
/// CONTRIBUTION and every version it minted.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when nothing is staged or a staged change is
/// structurally unusable; CDR transport errors pass through; a non-2xx CDR
/// answer (its validation diagnostics, which the UI renders verbatim,
/// included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
#[server(input = server_fn::codec::Json, client = crate::session_client::SessionAwareClient)]
pub async fn commit_contribution(
    /// The EHR to commit the change set into.
    ehr_id: String,
    /// The staged changes, in the order they were staged.
    changes: Vec<StagedChange>,
    /// The `AUDIT_DETAILS.committer` name; blank uses the session identity.
    committer: String,
    /// The optional `AUDIT_DETAILS.description`; blank omits the attribute.
    description: String,
) -> Result<CommitOutcome, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let committer = if committer.trim().is_empty() {
        session.identity.clone()
    } else {
        committer.trim().to_owned()
    };
    let body = staged::contribution_body(&changes, &committer, &description)
        .map_err(ViewerError::Invalid)?;
    let url = state.cdr.rest_v1(&format!(
        "ehr/{}/contribution",
        urlencoding::encode(&ehr_id)
    ));
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
    Ok(parse_commit_outcome(&response.body))
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten the committed CONTRIBUTION into a [`CommitOutcome`].
///
/// Defensive throughout — a body the CDR served without the representation
/// (an older server honouring `return=minimal`) reads as an empty outcome
/// rather than failing a commit that actually succeeded.
fn parse_commit_outcome(body: &str) -> CommitOutcome {
    let Ok(doc) = serde_json::from_str::<Value>(body) else {
        return CommitOutcome::default();
    };
    let version_uids = doc
        .get("versions")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .map(|reference| {
                    reference
                        .get("id")
                        .and_then(|id| id.get("value"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .filter(|uid| !uid.is_empty())
                .collect()
        })
        .unwrap_or_default();
    CommitOutcome {
        contribution_uid: crate::uid::uid_value_of_document(&doc),
        version_uids,
    }
}




















#[cfg(all(test, not(target_arch = "wasm32")))]
mod outcome_tests {
    use super::parse_commit_outcome;

    #[test]
    fn the_201_representation_yields_the_contribution_and_every_minted_version() {
        // The body a composed CDR answered to a two-member change set.
        let body = r#"{
            "_type": "CONTRIBUTION",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "01a02ba0-f378-7034-9628-cc373c56ece4"},
            "audit": {"_type": "AUDIT_DETAILS"},
            "versions": [
                {"_type": "OBJECT_REF", "namespace": "local", "type": "EHR_STATUS",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "01a0::ferroehr.local::2"}},
                {"_type": "OBJECT_REF", "namespace": "local", "type": "COMPOSITION",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "01a1::ferroehr.local::1"}}
            ]
        }"#;
        let outcome = parse_commit_outcome(body);
        assert_eq!(
            outcome.contribution_uid,
            "01a02ba0-f378-7034-9628-cc373c56ece4"
        );
        assert_eq!(
            outcome.version_uids,
            vec![
                "01a0::ferroehr.local::2".to_owned(),
                "01a1::ferroehr.local::1".to_owned()
            ]
        );
    }

    #[test]
    fn an_absent_representation_reads_as_an_empty_outcome_never_a_failure() {
        for body in ["", "{}", "not json", r#"{"versions":[{"id":{}}]}"#] {
            let outcome = parse_commit_outcome(body);
            assert!(outcome.contribution_uid.is_empty(), "{body}");
            assert!(outcome.version_uids.is_empty(), "{body}");
        }
    }
}
