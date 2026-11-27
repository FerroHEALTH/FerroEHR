// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admin registers over the CDR's admin extension.
//!
//! The routes are `{ext}/admin/registers/{register}` and `/history`: the EHDS
//! audit category map and the EHDS Art. 3(3) access delay, each read and
//! written whole over its version, version 0 the default no one committed. No openEHR
//! specification governs the registers: FerroEHR's own design.

use crate::bff::access_policy::Shown;
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// A register the CDR keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    /// `audit-categories`: the category map every access is classified
    /// through.
    AuditCategories,
    /// `access-delay`: the delay of the person's own access.
    AccessDelay,
}

impl Register {
    /// Every register, in the order the page shows them.
    pub const ALL: [Self; 2] = [Self::AuditCategories, Self::AccessDelay];

    /// The register's name on the wire and in the viewer's routes.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::AuditCategories => "audit-categories",
            Self::AccessDelay => "access-delay",
        }
    }

    /// The register `key` names.
    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|register| register.key() == key)
    }
}

/// One version of a register.
#[derive(Debug, Clone, PartialEq)]
pub struct RegisterVersion {
    /// Its number, 0 for the default.
    pub version: i64,
    /// Who committed it, `None` for the default.
    pub committer: Option<String>,
    /// When, `None` for the default.
    pub committed_at: Option<String>,
    /// Its document, pretty-printed.
    pub document: String,
    /// Its document's members, read for drawing.
    pub members: Vec<(String, Shown)>,
}

/// The newest version of `register` (`GET admin/registers/{register}`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `403` and the
/// `405` of an admin API that is off included; [`ViewerError::Failed`] when
/// the answer is not a register version.
pub async fn fetch_register(register: Register) -> Result<RegisterVersion, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("admin/registers/{}", register.key()));
    let body = cdr.get(&url).accept("application/json").ok().await?.body;
    parse_version(&body)
}

/// Every version of `register`, newest first, ending with version 0
/// (`GET admin/registers/{register}/history`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not a register history.
pub async fn fetch_register_history(
    register: Register,
) -> Result<Vec<RegisterVersion>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("admin/registers/{}/history", register.key()));
    let body = cdr.get(&url).accept("application/json").ok().await?.body;
    parse_history(&body)
}

/// Replaces `register` with `document` over the version `replaces`
/// (`PUT admin/registers/{register}` with `If-Match`); the version it makes.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400` and
/// `412` normalize via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn put_register(
    register: Register,
    replaces: i64,
    document: String,
) -> Result<RegisterVersion, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("admin/registers/{}", register.key()));
    let condition = format!("\"{replaces}\"");
    let body = cdr
        .put(&url)
        .accept("application/json")
        .body("application/json", document)
        .headers(&[(http::header::IF_MATCH.as_str(), condition.as_str())])
        .ok()
        .await?
        .body;
    parse_version(&body)
}

/// One version as a read answers it.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's register versions, JSON of FerroEHR's own admin \
              extension"
)]
fn version_of(value: &serde_json::Value) -> RegisterVersion {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let document = value.get("document").cloned().unwrap_or_default();
    let members = match Shown::of(&document) {
        Shown::Fields { fields, .. } => fields,
        _ => Vec::new(),
    };
    RegisterVersion {
        version: value
            .get("version")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default(),
        committer: text("committer"),
        committed_at: text("committed_at"),
        document: serde_json::to_string_pretty(&document).unwrap_or_default(),
        members,
    }
}

/// The version a read body answers.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's register versions, JSON of FerroEHR's own admin \
              extension"
)]
pub fn parse_version(body: &str) -> Result<RegisterVersion, ViewerError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::RegisterJson), error))?;
    Ok(version_of(&value))
}

/// The versions a history body lists, newest first.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's register versions, JSON of FerroEHR's own admin \
              extension"
)]
pub fn parse_history(body: &str) -> Result<Vec<RegisterVersion>, ViewerError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::RegisterJson), error))?;
    Ok(value
        .get("versions")
        .and_then(serde_json::Value::as_array)
        .map(|versions| versions.iter().map(version_of).collect())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::{Register, parse_history, parse_version};
    use crate::bff::access_policy::Shown;

    #[test]
    fn a_register_reads_its_version_and_its_members() {
        let version = parse_version(
            r#"{"register":"access-delay","version":2,"committer":"admin",
                "committed_at":"2026-11-12T09:15:00Z",
                "document":{"enabled":true,"categories":["Laboratory-Reports"],"max_delay":"72h"}}"#,
        )
        .expect("a version");
        assert_eq!(2, version.version);
        assert_eq!(Some("admin".to_owned()), version.committer);
        assert_eq!(
            Some(&("enabled".to_owned(), Shown::Flag(true))),
            version.members.first()
        );
    }

    #[test]
    fn the_history_ends_with_the_default() {
        let versions = parse_history(
            r#"{"register":"access-delay","versions":[
                {"version":1,"committer":"admin","committed_at":"2026-11-12T09:15:00Z","document":{}},
                {"version":0,"committer":null,"committed_at":null,"document":{"enabled":false}}]}"#,
        )
        .expect("a history");
        assert_eq!(
            vec![1, 0],
            versions
                .iter()
                .map(|version| version.version)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            None,
            versions
                .last()
                .and_then(|version| version.committer.clone())
        );
    }

    #[test]
    fn a_register_reads_back_from_its_key() {
        for register in Register::ALL {
            assert_eq!(Some(register), Register::parse(register.key()));
        }
        assert_eq!(None, Register::parse("retention"));
    }
}
