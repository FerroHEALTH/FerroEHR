// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic CONTRIBUTION the `/parties/contributions/{uid}` page reads.
//!
//! A read-only screen over `GET /demographic/contribution/{contribution_uid}`
//! ("Retrieves a CONTRIBUTION identified by `contribution_uid`",
//! `operations/demographic_contribution_get.yaml`), reached from the
//! contribution a version's envelope names on any demographic History tab.
//!
//! Committing a contribution is deliberately NOT here: every party and
//! relationship write on these screens already commits its own CONTRIBUTION,
//! and a raw change-set authoring form is a different feature.
//!
//! The RM attributes shown are the CONTRIBUTION class's own
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.contribution.adoc`):
//! `uid`, the `audit` `AUDIT_DETAILS`, and `versions` — "the set of references
//! to Versions causing changes to this EHR" — each of which the demographic
//! surface serves as an `OBJECT_REF` whose `type` names the RM class of the
//! object that changed, which is what lets each row link to that object.

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

use crate::error::ViewerError;
#[cfg(not(target_arch = "wasm32"))]
use crate::locale::tr_served;
use crate::locale::{Msg, tr_with};

/// One version a CONTRIBUTION changed, as its `OBJECT_REF` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContributionVersion {
    /// `OBJECT_REF.type` — the RM class of the changed object.
    pub rm_type: String,
    /// `OBJECT_REF.id.value` — the changed version's `OBJECT_VERSION_ID`.
    pub version_uid: String,
}

/// The viewer's view of one demographic CONTRIBUTION.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContributionState {
    /// The canonical CONTRIBUTION JSON, pretty-printed for the pane.
    pub body: String,
    /// `CONTRIBUTION.uid.value`.
    pub uid: String,
    /// `audit.committer.name`.
    pub committer: String,
    /// `audit.time_committed.value`.
    pub time_committed: String,
    /// `audit.change_type.value`.
    pub change_type: String,
    /// `audit.description.value`, empty when the commit carried none.
    pub description: String,
    /// The versions this contribution changed.
    pub versions: Vec<ContributionVersion>,
}

/// Read one demographic CONTRIBUTION.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty uid; CDR transport errors pass
/// through; a non-2xx CDR answer (the `404` for an unknown contribution, or for
/// an EHR-scoped one, included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_demographic_contribution(
    /// The CONTRIBUTION uid to read.
    contribution_uid: String,
) -> Result<ContributionState, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let contribution_uid = contribution_uid.trim();
    if contribution_uid.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::ContributionUidRequired,
        )));
    }
    let url = state.cdr.rest_v1(&format!(
        "demographic/contribution/{}",
        urlencoding::encode(contribution_uid)
    ));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    parse_contribution(&body)
}

#[cfg(not(target_arch = "wasm32"))]
/// Flatten a CONTRIBUTION body into a [`ContributionState`]. Defensive
/// throughout — an absent attribute reads as empty rather than failing the
/// screen.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_contribution(body: &str) -> Result<ContributionState, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(
            Msg::ContributionJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let versions = doc
        .get("versions")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .map(|reference| ContributionVersion {
                    rm_type: reference
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    version_uid: super::json_str(reference, &["id", "value"]),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ContributionState {
        body: crate::bff::pretty_body(
            body,
            crate::format::ReprFormat::CanonicalJson,
        ),
        uid: crate::uid::uid_value_of_document(&doc),
        committer: super::json_str(&doc, &["audit", "committer", "name"]),
        time_committed: super::json_str(&doc, &["audit", "time_committed", "value"]),
        change_type: super::json_str(&doc, &["audit", "change_type", "value"]),
        description: super::json_str(&doc, &["audit", "description", "value"]),
        versions,
    })
}






#[cfg(all(test, not(target_arch = "wasm32")))]
mod wire_tests {
    use super::parse_contribution;

    #[test]
    fn parses_the_audit_facts_and_the_changed_versions() {
        let body = r#"{
            "_type": "CONTRIBUTION",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "c9"},
            "audit": {
                "_type": "AUDIT_DETAILS",
                "system_id": "example.org",
                "committer": {"_type": "PARTY_IDENTIFIED", "name": "Dr Jane Williams"},
                "time_committed": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"},
                "change_type": {"_type": "DV_CODED_TEXT", "value": "creation"},
                "description": {"_type": "DV_TEXT", "value": "PERSON creation"}
            },
            "versions": [
                {"_type": "OBJECT_REF", "namespace": "demographic", "type": "PERSON",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.org::1"}},
                {"_type": "OBJECT_REF", "namespace": "demographic", "type": "PARTY_RELATIONSHIP",
                 "id": {"_type": "OBJECT_VERSION_ID", "value": "7d44aa01::example.org::1"}}
            ]
        }"#;
        let state = parse_contribution(body).expect("a valid CONTRIBUTION");
        assert_eq!(state.uid, "c9");
        assert_eq!(state.committer, "Dr Jane Williams");
        assert_eq!(state.time_committed, "2026-07-12T10:00:00Z");
        assert_eq!(state.change_type, "creation");
        assert_eq!(state.description, "PERSON creation");
        assert_eq!(state.versions.len(), 2);
        assert_eq!(state.versions[0].rm_type, "PERSON");
        assert_eq!(state.versions[0].version_uid, "8849182c::example.org::1");
        assert_eq!(state.versions[1].rm_type, "PARTY_RELATIONSHIP");
        // The pane gets the document pretty-printed.
        assert!(state.body.contains("\n  \"_type\""));
    }

    #[test]
    fn a_sparse_contribution_reads_as_empty_facts_and_a_bad_body_errors() {
        let state = parse_contribution("{}").expect("an object parses");
        assert_eq!(state.uid, "");
        assert_eq!(
            state.versions,
            Vec::<crate::bff::demographics::contribution::ContributionVersion>::new()
        );
        assert!(parse_contribution("not json").is_err());
    }
}
