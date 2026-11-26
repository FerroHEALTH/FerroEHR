// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! EHR Extracts in and out (`{ext}/message/export/{ehr_id}`,
//! `{ext}/message/import`), and the repository's dump and load on the
//! server's file system (`{ext}/admin/dump`, `/load`).
//!
//! No openEHR spec surfaces these routes over REST: the CDR's own extension
//! of the openEHR service model's Message and Admin components.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde::Deserialize;

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// The path of the export of the EHR `ehr_id` under the extensions root.
#[must_use]
pub fn export_path(ehr_id: &str) -> String {
    format!("message/export/{}", urlencoding::encode(ehr_id))
}

/// The body an import sends for the uploaded `source`, and its media type.
///
/// XML goes as uploaded, and JSON with the one `EXTRACT` of an export's list
/// unwrapped, since an export answers a list and an import takes one.
#[must_use]
pub fn import_body(source: &str) -> (&'static str, String) {
    let trimmed = source.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('<') {
        return ("application/xml", trimmed.to_owned());
    }
    // NOTE: no openEHR spec governs this: our own design; a body that is no JSON
    // list of one goes as uploaded, and the CDR judges it.
    let one = serde_json::from_str::<Vec<serde_json::Value>>(trimmed)
        .ok()
        .filter(|list| list.len() == 1)
        .and_then(|mut list| list.pop())
        .map(|extract| extract.to_string());
    (
        "application/json",
        one.unwrap_or_else(|| trimmed.to_owned()),
    )
}

/// The answer of an import that created an EHR.
#[derive(Debug, Deserialize)]
struct Created {
    /// The EHR's id.
    uid: String,
}

/// Imports the EHR Extract `source` as a new EHR, under `ehr_id` when one is
/// given, else under the id the extract carries.
///
/// # Errors
/// [`ViewerError::Invalid`] for an empty source; the errors of a CDR call
/// ([`cdr`](crate::bff::cdr)), the CDR's words for an extract without an
/// `EHR_STATUS` (`400`), an EHR that exists already (`409`) and an invalid
/// version (`422`); [`ViewerError::Failed`] for an answer naming no EHR.
pub async fn import_extract(source: &str, ehr_id: &str) -> Result<String, ViewerError> {
    if source.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            crate::locale::Msg::SelectedFileWasEmpty,
        )));
    }
    let cdr = crate::bff::cdr()?;
    let url = if ehr_id.trim().is_empty() {
        cdr.extensions_v1("message/import")
    } else {
        cdr.extensions_v1(&format!(
            "message/import?ehr_id={}",
            urlencoding::encode(ehr_id.trim())
        ))
    };
    let (media, body) = import_body(source);
    let answer = cdr.post(&url).body(media, body).ok().await?.body;
    serde_json::from_str::<Created>(&answer)
        .map(|created| created.uid)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::ImportAnswerJson), error))
}

/// What a dump writes: the location on the server and the export settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DumpAsk {
    /// The location on the server's file system.
    pub location: String,
    /// `openehr_canonical_json` or `openehr_canonical_xml`; empty for the
    /// CDR's default.
    pub logical_format: String,
    /// `zip` or `7z`; empty for loose files.
    pub compression: String,
    /// The segment size in kb; empty for the CDR's default.
    pub segment_kb: String,
}

impl DumpAsk {
    /// The dump's request body, each setting left out when empty; a segment
    /// size that is no whole number goes as typed, so the CDR refuses it.
    #[must_use]
    pub fn body(&self) -> String {
        let mut body = serde_json::Map::new();
        body.insert("file_sys_loc".to_owned(), self.location.trim().into());
        if !self.logical_format.is_empty() {
            body.insert(
                "logical_format".to_owned(),
                self.logical_format.clone().into(),
            );
        }
        if !self.compression.is_empty() {
            body.insert(
                "compression_format".to_owned(),
                self.compression.clone().into(),
            );
        }
        let size = self.segment_kb.trim();
        if !size.is_empty() {
            let value = size
                .parse::<u64>()
                .map_or_else(|_| serde_json::Value::from(size), serde_json::Value::from);
            body.insert("segment_split_size".to_owned(), value);
        }
        serde_json::Value::Object(body).to_string()
    }
}

/// One entity a dump or a load did not move.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Failure {
    /// Its kind: `EHR`, or a demographic kind.
    #[serde(default)]
    pub entity_type: String,
    /// Its id.
    #[serde(default)]
    pub entity_id: String,
    /// Why.
    #[serde(default)]
    pub error: String,
}

/// Posts `body` to the admin route `route` and reads its failure report.
async fn repository(route: &str, body: String) -> Result<Vec<Failure>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("admin/{route}"));
    let answer = cdr
        .post(&url)
        .body("application/json", body)
        .ok()
        .await?
        .body;
    serde_json::from_str(&answer)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::RepositoryReportJson), error))
}

/// Writes an archive of the whole repository where `ask` says, on the
/// server: the entities it could not write, none when all went.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the CDR's words for
/// a missing location or a setting outside its lists (`400`), a location it
/// cannot write (`500`) and an admin group that is off (`405`).
pub async fn dump(ask: &DumpAsk) -> Result<Vec<Failure>, ViewerError> {
    repository("dump", ask.body()).await
}

/// Loads the archive at `location` on the server into the repository: the
/// entities it skipped, none when all loaded.
///
/// # Errors
/// The errors of [`dump`], and the CDR's words for a malformed archive
/// (`400`) or a location holding none (`500`).
pub async fn load(location: &str) -> Result<Vec<Failure>, ViewerError> {
    repository(
        "load",
        serde_json::json!({ "file_sys_loc": location.trim() }).to_string(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{DumpAsk, import_body};

    /// An export's list of one goes as its one extract; XML and anything
    /// else go as uploaded.
    #[test]
    fn an_exported_list_imports_as_its_extract() {
        assert_eq!(
            import_body(r#"[{"_type":"EXTRACT"}]"#),
            ("application/json", r#"{"_type":"EXTRACT"}"#.to_owned())
        );
        assert_eq!(
            import_body(r#"{"_type":"EXTRACT"}"#),
            ("application/json", r#"{"_type":"EXTRACT"}"#.to_owned())
        );
        assert_eq!(import_body("<extract/>").0, "application/xml");
    }

    /// A dump names only the settings given.
    #[test]
    fn a_dump_names_the_settings_given() {
        let ask = DumpAsk {
            location: " /var/dumps/a ".to_owned(),
            compression: "zip".to_owned(),
            segment_kb: "2048".to_owned(),
            ..DumpAsk::default()
        };
        assert_eq!(
            ask.body(),
            r#"{"file_sys_loc":"/var/dumps/a","compression_format":"zip","segment_split_size":2048}"#
        );
    }
}
