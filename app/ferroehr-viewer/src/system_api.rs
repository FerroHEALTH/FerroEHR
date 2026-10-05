// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR **System API** surface: the CDR's conformance manifest.
//!
//! `OPTIONS {base_path}` with `Accept: application/json` is the spec's own
//! capability-discovery operation — the STABLE ITS-REST 1.1.0 System API
//! (`docs/specs/openehr/ITS-REST/specifications/system.openapi.yaml`, the single
//! `operationId: options`; body schema
//! `docs/specs/openehr/ITS-REST/specifications/schemas/others/Options.yaml`).
//! Its `endpoints` array is the server's live mounted-group set, which is how
//! the viewer discovers optional API groups instead of poking one of their
//! operations.
//!
//! Shared, not screen-local: the manifest also carries the CDR's identity and
//! conformance profile, which more than one screen wants.
//!
//! The manifest is served above CORS and outside authentication (`security:
//! []`), so the fetch carries no CDR credential — but the server fn still
//! guards the viewer session first, because every `#[server]` fn is a publicly
//! reachable endpoint.

use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// The System API conformance manifest — the `Options` schema, field for field.
///
/// Every field is a plain `String`/`Vec<String>` (no `usize`), so the type is
/// safe across the server-fn boundary on the 32-bit WASM target.
///
/// Every member is optional-tolerant: `Options.yaml` declares no `required`
/// list at all, so a conformant peer may omit any of them and the reader fills
/// the gap from [`Default`] rather than failing the whole manifest — which
/// would hide every affordance the `endpoints` list gates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConformanceManifest {
    /// The product name the CDR advertises.
    pub solution: String,
    /// The advertised product version.
    pub solution_version: String,
    /// The advertised vendor.
    pub vendor: String,
    /// The ITS-REST specification version the CDR implements.
    pub restapi_specs_version: String,
    /// The conformance profile the CDR claims.
    pub conformance_profile: String,
    /// The API groups the server actually mounts (`/ehr`, `/query`, …, and
    /// `/admin` only while the admin group is enabled).
    pub endpoints: Vec<String>,
}

impl ConformanceManifest {
    /// Whether the manifest advertises `endpoint` as a mounted API group.
    ///
    /// Compared against the spec's own leading-slash group names
    /// (`Options.yaml` `example`: `/ehr`, `/demographic`, `/definition`,
    /// `/query`, `/admin`), tolerating a missing or extra slash on either side
    /// so a server that writes `admin` still matches.
    #[must_use]
    pub fn advertises(&self, endpoint: &str) -> bool {
        let wanted = endpoint.trim_matches('/');
        self.endpoints
            .iter()
            .any(|advertised| advertised.trim_matches('/') == wanted)
    }
}

/// Fetch the CDR's System API conformance manifest.
///
/// The manifest reports what the server MOUNTS, never what the caller may DO:
/// authorization stays a per-request answer (`401`/`403`) the calling screen
/// surfaces.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnreachable`] on transport failure;
/// [`ViewerError::Cdr`] when the CDR answers non-2xx;
/// [`ViewerError::Internal`] when the body is not the `Options` shape.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_conformance_manifest() -> Result<ConformanceManifest, ViewerError> {
    crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_v1_root();
    let response = state.cdr.options_public(&url, "application/json").await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    serde_json::from_str::<ConformanceManifest>(&body)
        .map_err(|e| ViewerError::Internal(format!("conformance manifest JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::ConformanceManifest;

    fn manifest(endpoints: &[&str]) -> ConformanceManifest {
        ConformanceManifest {
            endpoints: endpoints.iter().map(|e| (*e).to_owned()).collect(),
            ..ConformanceManifest::default()
        }
    }

    #[test]
    fn advertises_matches_the_spec_group_names() {
        let m = manifest(&["/ehr", "/definition", "/query", "/demographic", "/admin"]);
        assert!(m.advertises("/admin"));
        assert!(m.advertises("/query"));
        assert!(!m.advertises("/contribution"));
    }

    #[test]
    fn advertises_tolerates_slash_variants_on_either_side() {
        assert!(manifest(&["admin"]).advertises("/admin"));
        assert!(manifest(&["/admin/"]).advertises("admin"));
        // …but never a partial match: a group is the whole segment set.
        assert!(!manifest(&["/administration"]).advertises("/admin"));
    }

    #[test]
    fn an_admin_less_manifest_advertises_no_admin_group() {
        // The CDR omits `/admin` from the live list while the group is disabled.
        let m = manifest(&["/ehr", "/definition", "/query", "/demographic"]);
        assert!(!m.advertises("/admin"));
    }

    #[test]
    fn the_options_body_deserializes_field_for_field() {
        // The `Options.yaml` example body, verbatim.
        let body = r#"{
            "solution": "openEHRSys",
            "solution_version": "v1.0",
            "vendor": "My-openEHR",
            "restapi_specs_version": "1.1.0",
            "conformance_profile": "STANDARD",
            "endpoints": ["/ehr", "/demographic", "/definition", "/query", "/admin"]
        }"#;
        let parsed: ConformanceManifest =
            serde_json::from_str(body).expect("the spec's own example body parses");
        assert_eq!(parsed.solution, "openEHRSys");
        assert_eq!(parsed.solution_version, "v1.0");
        assert_eq!(parsed.vendor, "My-openEHR");
        assert_eq!(parsed.restapi_specs_version, "1.1.0");
        assert_eq!(parsed.conformance_profile, "STANDARD");
        assert!(parsed.advertises("/admin"));
    }

    #[test]
    fn a_manifest_declaring_only_one_member_still_reads() {
        // `Options.yaml` carries no `required` list, so a conformant peer may
        // send any subset; an absent member must not lose the whole manifest.
        let parsed: ConformanceManifest = serde_json::from_str(r#"{"solution":"otherEHR"}"#)
            .expect("an Options body with one member parses");
        assert_eq!(parsed.solution, "otherEHR");
        assert_eq!(parsed.vendor, String::new());
        assert_eq!(parsed.restapi_specs_version, String::new());
        assert!(parsed.endpoints.is_empty());
        // An empty list is an honest "advertises nothing", not a parse failure.
        assert!(!parsed.advertises("/admin"));
        // The empty document is the same story.
        let empty: ConformanceManifest =
            serde_json::from_str("{}").expect("an empty Options body parses");
        assert_eq!(empty, ConformanceManifest::default());
    }
}
