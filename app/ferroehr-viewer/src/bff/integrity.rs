// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Storage integrity: the sweep and the rebuild of what it finds damaged.
//!
//! The sweep (`{ext}/admin/integrity/verify`) compares every stored version
//! with its decomposed rows; the rebuild (`/rebuild-nodes`) re-derives the
//! rows of each version the sweep finds damaged.
//!
//! Both take the same four optional scope parameters; the viewer asks for
//! the aggregated report, never the stream. No openEHR spec governs these
//! routes: the CDR's own extension.

use serde::Deserialize;

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// What a sweep or a rebuild covers, each part as typed; an empty part
/// narrows nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Only versions of this EHR.
    pub ehr_id: String,
    /// Only versions of this versioned object.
    pub vo_id: String,
    /// Only this version of the object `vo_id` names.
    pub sys_version: String,
    /// Only versions whose validity begins at or after this RFC 3339 instant.
    pub committed_since: String,
}

impl Scope {
    /// The query the two routes take, with the leading `?`; empty when the
    /// scope narrows nothing.
    #[must_use]
    pub fn query(&self) -> String {
        let parts: Vec<String> = [
            ("ehr_id", &self.ehr_id),
            ("vo_id", &self.vo_id),
            ("sys_version", &self.sys_version),
            ("committed_since", &self.committed_since),
        ]
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value.trim())))
        .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    }
}

/// One disagreement the sweep found.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Mismatch {
    /// `clinical` or `demographic`.
    pub domain: String,
    /// The versioned object.
    pub vo_id: String,
    /// Its version ordinal.
    #[serde(default)]
    pub sys_version: Option<u64>,
    /// The RM class of the object or of the rows' root.
    pub kind: String,
    /// What disagrees.
    pub defect: String,
}

/// The sweep's aggregated report.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Verification {
    /// How many versions the sweep read.
    pub versions_checked: u64,
    /// How many disagreements it found, beyond the listed cap too.
    pub mismatch_count: u64,
    /// The disagreements, capped.
    #[serde(default)]
    pub mismatches: Vec<Mismatch>,
    /// Whether the cap cut `mismatches` short.
    #[serde(default)]
    pub truncated: bool,
    /// How long the sweep ran, in milliseconds.
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// What the rebuild did to one damaged version.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rebuilt {
    /// The versioned object.
    pub vo_id: String,
    /// Its version ordinal.
    #[serde(default)]
    pub sys_version: Option<u64>,
    /// The RM class.
    pub kind: String,
    /// The defect that selected it.
    pub defect: String,
    /// `rebuilt` or `refused`.
    pub outcome: String,
    /// Why a refused version was left as it was.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The rebuild's report.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rebuild {
    /// How many versions the underlying sweep read.
    pub versions_checked: u64,
    /// How many it found damaged.
    pub versions_damaged: u64,
    /// How many were rebuilt.
    pub versions_rebuilt: u64,
    /// How many were refused.
    pub versions_refused: u64,
    /// One record per damaged version, capped.
    #[serde(default)]
    pub records: Vec<Rebuilt>,
    /// Whether the cap cut `records` short.
    #[serde(default)]
    pub truncated: bool,
}

/// The sweep: verifies every version in the scope against its stored body.
pub const VERIFY: &str = "verify";

/// The repair: rebuilds the decomposed rows of every damaged version.
pub const REBUILD: &str = "rebuild-nodes";

/// Posts to the integrity route `route` ([`VERIFY`] or [`REBUILD`]) over
/// `scope` and reads its report.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Cdr`]
/// with the CDR's words for a `400` (a malformed scope) and a `405` (the
/// admin group is off); [`ViewerError::Failed`] for a report that does not
/// read.
pub async fn run<T: serde::de::DeserializeOwned>(
    route: &str,
    scope: &Scope,
) -> Result<T, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("admin/integrity/{route}{}", scope.query()));
    let body = cdr.post(&url).ok().await?.body;
    serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::IntegrityReportJson), error))
}

#[cfg(test)]
mod tests {
    use super::{Scope, Verification};

    /// Only the parts typed narrow the scope, each encoded.
    #[test]
    fn the_scope_names_only_what_was_typed() {
        assert_eq!(Scope::default().query(), "");
        let scope = Scope {
            vo_id: "a b".to_owned(),
            sys_version: "2".to_owned(),
            committed_since: " 2026-01-01T00:00:00Z ".to_owned(),
            ..Scope::default()
        };
        assert_eq!(
            scope.query(),
            "?vo_id=a%20b&sys_version=2&committed_since=2026-01-01T00%3A00%3A00Z"
        );
    }

    /// The sweep's documented report reads whole.
    #[test]
    fn the_documented_report_reads() {
        let report: Verification = serde_json::from_str(
            r#"{"versions_checked":128,"versions_with_body":126,"versions_without_body":2,"mismatch_count":1,"mismatches":[{"domain":"clinical","vo_id":"8849182c-82ad-4088-a07f-48ead4180515","sys_version":2,"kind":"COMPOSITION","defect":"content_differs"}],"truncated":false,"elapsed_ms":431}"#,
        )
        .expect("the report reads");
        assert_eq!(report.mismatch_count, 1);
        assert_eq!(report.mismatches[0].sys_version, Some(2));
    }
}
