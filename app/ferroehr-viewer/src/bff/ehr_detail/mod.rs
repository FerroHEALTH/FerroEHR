// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/ehrs/{ehr_id}` screen — EHR detail: status / status history /
//! directory / compositions / contributions / commit / tags tabs.
//!
//! Seven URL-driven tabs (`?tab=`) over one EHR. Each tab's data is a
//! read co-located with the tab in its own submodule: [`status`],
//! which owns BOTH status tabs, then [`directory`], [`compositions`],
//! [`contributions`], [`commit`] (the atomic multi-change CONTRIBUTION staging
//! area), and the EHR-wide tag browser in [`crate::bff::ehr_tags`]. The
//! resources are created once and their sources are gated on the active tab, so
//! only the visible tab fetches. The tab bodies are always mounted and toggled
//! with `class:hidden`, keeping the server and client view structure identical.
//!
//! This module owns the `EhrDetailPage` shell, the EHR summary header and the
//! shared `tab_bar` strip. The header carries two reads, neither duplicated
//! elsewhere on the screen: the EHR resource itself (`GET /ehr/{ehr_id}`, so an
//! unknown id fails once at the top instead of once per tab) and — through
//! `status_feed` — the current `EHR_STATUS`, which is the ONE read of the
//! subject and the capability flags the Status tab also renders. That one is
//! therefore NOT tab-gated: the header shows on every tab.
//!
//! No openEHR spec governs the viewer — our own design; the wire it reads is
//! spec-bound (ITS-REST EHR + Query APIs). User input NEVER concatenates into
//! AQL — the compositions listing's statement is assembled from compile-time
//! fragments by [`composition_filter`] and every value travels as an AQL
//! `query_parameters` binding; path segments are percent-encoded server-side.
//! Every read guards with
//! [`require_session`](crate::session::require_session) first, and the CDR
//! credential never reaches the page.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

pub mod commit;
pub mod composition_filter;
pub mod compositions;
pub mod contributions;
pub mod directory;
pub mod status;

use serde_json::Value;

use crate::error::ViewerError;
use crate::locale::{Msg, tr_with};

/// The EHR resource's own summary facts, flattened for the detail header.
///
/// The attributes are the RM `EHR` class's own: `system_id` ("the identifier of
/// the logical EHR management system in which this EHR was created"),
/// `time_created`, and the `ehr_status` reference — an `OBJECT_REF` whose `id`
/// addresses the CURRENT `EHR_STATUS` version and whose `type` names the
/// versioned container (invariant `Ehr_status_valid`:
/// `ehr_status.type.is_equal("VERSIONED_EHR_STATUS")`) — all from
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr.adoc`
/// §`EHR Class`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EhrSummary {
    /// `EHR.ehr_id.value` as the CDR reports it.
    pub ehr_id: String,
    /// `EHR.system_id.value` — recorded at creation, immutable thereafter.
    pub system_id: String,
    /// `EHR.time_created.value`.
    pub time_created: String,
    /// `EHR.ehr_status.id.value` — the current `EHR_STATUS` version.
    pub ehr_status_uid: String,
    /// `EHR.ehr_status.type` — the referenced versioned container's RM type.
    pub ehr_status_type: String,
}

/// Read the EHR resource itself (`GET /ehr/{ehr_id}`) for the detail header.
///
/// This is the viewer's ONE reader of the EHR resource's own FACTS: an
/// unknown or mistyped id surfaces here, once, at the top of the screen — the
/// tabs read their own sub-resources. (The status tag panel reads the same
/// endpoint server-side for the one identifier that addresses the status's tag
/// collection — a second window of one endpoint, not a second reader of a
/// claim.)
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; CDR transport
/// errors pass through; a non-2xx CDR answer (the `404` for an unknown
/// `ehr_id` included) normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the body is not valid JSON.
pub async fn fetch_ehr_summary(
    // The EHR whose summary facts to read.
    ehr_id: String,
) -> Result<EhrSummary, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_ehr_summary(&response.body)
}

/// Flatten an `EHR` resource body into an [`EhrSummary`]. Defensive throughout —
/// a missing attribute reads as empty rather than failing, so a header still
/// renders what the CDR did send.
///
/// # Errors
/// [`ViewerError::Internal`] when the body is not valid JSON.
fn parse_ehr_summary(body: &str) -> Result<EhrSummary, ViewerError> {
    let doc: Value = serde_json::from_str(body).map_err(|e| {
        ViewerError::Internal(tr_with(Msg::EhrJson, &[("e", (e).to_string().into())]))
    })?;
    Ok(EhrSummary {
        ehr_id: json_path(&doc, &["ehr_id", "value"]),
        system_id: json_path(&doc, &["system_id", "value"]),
        time_created: json_path(&doc, &["time_created", "value"]),
        ehr_status_uid: json_path(&doc, &["ehr_status", "id", "value"]),
        ehr_status_type: json_path(&doc, &["ehr_status", "type"]),
    })
}

/// Follow a chain of object keys to a string leaf, or an empty string when any
/// hop is absent or not a string.
fn json_path(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::parse_ehr_summary;

    #[test]
    fn parses_the_ehr_resources_summary_attributes() {
        // The RM `EHR` wire shape: `system_id`/`ehr_id` HIER_OBJECT_IDs, a
        // DV_DATE_TIME `time_created`, and the `ehr_status` OBJECT_REF whose id
        // is the CURRENT EHR_STATUS OBJECT_VERSION_ID.
        let body = r#"{
            "_type": "EHR",
            "system_id": {"_type": "HIER_OBJECT_ID", "value": "example.ferroehr.eu"},
            "ehr_id": {"_type": "HIER_OBJECT_ID", "value": "7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d"},
            "ehr_status": {
                "_type": "OBJECT_REF",
                "namespace": "local",
                "type": "VERSIONED_EHR_STATUS",
                "id": {"_type": "OBJECT_VERSION_ID", "value": "8849182c::example.ferroehr.eu::1"}
            },
            "time_created": {"_type": "DV_DATE_TIME", "value": "2026-07-12T10:00:00Z"}
        }"#;
        let summary = parse_ehr_summary(body).expect("valid EHR resource");
        assert_eq!(summary.ehr_id, "7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d");
        assert_eq!(summary.system_id, "example.ferroehr.eu");
        assert_eq!(summary.time_created, "2026-07-12T10:00:00Z");
        assert_eq!(summary.ehr_status_uid, "8849182c::example.ferroehr.eu::1");
        assert_eq!(summary.ehr_status_type, "VERSIONED_EHR_STATUS");
    }

    #[test]
    fn a_sparse_ehr_body_reads_as_empty_facts_and_a_bad_body_errors() {
        let summary = parse_ehr_summary("{}").expect("an empty object parses");
        assert_eq!(summary.ehr_id, "");
        assert_eq!(summary.system_id, "");
        assert_eq!(summary.ehr_status_uid, "");
        assert!(parse_ehr_summary("not json").is_err());
    }
}
