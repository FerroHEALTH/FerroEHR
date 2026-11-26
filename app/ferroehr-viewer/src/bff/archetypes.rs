// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetypes the CDR keeps beside its templates, and its ADL 1.4 to
//! ADL 2 conversion.
//!
//! The ADL 1.4 archetypes and the ADL 2 archetype list live under the
//! extensions root (`{ext}/definition/archetype/*`); an ADL 2 archetype's
//! source is read where the openEHR API serves every ADL 2 artefact
//! (`definition/template/adl2/{id}`). No openEHR spec governs the archetype
//! routes or the conversion: the CDR's own extension.

use serde::Deserialize;

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// The ids one archetype list holds, `None` when the CDR keeps no archetypes
/// (`404`, the extension is off).
async fn list(route: &str) -> Result<Option<Vec<String>>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(route);
    let response = cdr.get(&url).send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = response.ok()?.body;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::ArchetypeListJson), error))
}

/// The stored ADL 1.4 archetype ids, ascending; `None` when the CDR keeps no
/// archetypes.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Failed`] for a list that is not JSON strings.
pub async fn list_adl14() -> Result<Option<Vec<String>>, ViewerError> {
    list("definition/archetype/adl1.4").await
}

/// The stored ADL 2 archetype ids; `None` when the CDR keeps no archetypes.
///
/// # Errors
/// The errors of [`list_adl14`].
pub async fn list_adl2() -> Result<Option<Vec<String>>, ViewerError> {
    list("definition/archetype/adl2").await
}

/// The path of the stored ADL 1.4 archetype `archetype_id` under the
/// extensions root, the id one encoded segment.
#[must_use]
pub fn adl14_path(archetype_id: &str) -> String {
    format!(
        "definition/archetype/adl1.4/{}",
        urlencoding::encode(archetype_id)
    )
}

/// The stored ADL 1.4 source of `archetype_id`, verbatim; `None` when no
/// archetype is stored under it.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
pub async fn fetch_adl14(archetype_id: &str) -> Result<Option<String>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&adl14_path(archetype_id));
    let response = cdr.get(&url).accept("text/plain").send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(response.ok()?.body))
}

/// Uploads the ADL 1.4 source `source`; the CDR reads the id out of it and
/// replaces an archetype stored under the same id.
///
/// # Errors
/// [`ViewerError::Invalid`] for an empty source; the errors of a CDR call
/// ([`cdr`](crate::bff::cdr)), the CDR's words for a source it refuses
/// (`422`) or a read-only principal (`403`).
pub async fn upload_adl14(source: String) -> Result<String, ViewerError> {
    if source.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            crate::locale::Msg::SelectedFileWasEmpty,
        )));
    }
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("definition/archetype/adl1.4");
    let body = cdr.post(&url).body("text/plain", source).ok().await?.body;
    serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::ArchetypeListJson), error))
}

/// The conversion log of one root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Log {
    /// The notes on the decisions the conversion took.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// One converted archetype root.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Root {
    /// The archetype id of the root.
    pub archetype_id: String,
    /// The ADL 2 source.
    pub adl2: String,
    /// The conversion log.
    #[serde(default)]
    pub log: Log,
    /// What the conversion dropped or changed, by key.
    #[serde(default)]
    pub conversion_details: std::collections::BTreeMap<String, String>,
}

/// One slot fill that joined two roots of a template.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Fill {
    /// The root that holds the slot.
    pub parent_archetype_id: String,
    /// The slot's path in it.
    pub parent_path: String,
    /// The root that filled it.
    pub child_archetype_id: String,
}

/// An ADL 1.4 to ADL 2 conversion.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Conversion {
    /// One root per converted archetype; a template's top root first.
    pub roots: Vec<Root>,
    /// The slot fills that joined a template's roots; empty for an archetype.
    #[serde(default)]
    pub structure: Vec<Fill>,
}

/// Reads one conversion.
async fn convert(path: String) -> Result<Conversion, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&path);
    let body = cdr.get(&url).ok().await?.body;
    serde_json::from_str(&body)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::ConversionJson), error))
}

/// The ADL 2 conversion of the stored ADL 1.4 archetype `archetype_id`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the CDR's words for
/// a source that no longer converts (`422`); [`ViewerError::Failed`] for an
/// answer that does not read.
pub async fn convert_archetype(archetype_id: &str) -> Result<Conversion, ViewerError> {
    convert(format!("{}/adl2", adl14_path(archetype_id))).await
}

/// The ADL 2 conversion of the stored ADL 1.4 operational template
/// `template_id`: one root per archetype root it embeds.
///
/// # Errors
/// The errors of [`convert_archetype`].
pub async fn convert_template(template_id: &str) -> Result<Conversion, ViewerError> {
    convert(format!(
        "definition/template/adl1.4/{}/adl2",
        urlencoding::encode(template_id)
    ))
    .await
}

#[cfg(test)]
mod tests {
    use super::{Conversion, adl14_path};

    /// An id is one encoded segment.
    #[test]
    fn an_archetype_id_is_one_segment() {
        assert_eq!(
            adl14_path("openEHR-EHR-OBSERVATION.a b/c.v1"),
            "definition/archetype/adl1.4/openEHR-EHR-OBSERVATION.a%20b%2Fc.v1"
        );
    }

    /// The served conversion reads whole.
    #[test]
    fn the_conversion_reads() {
        let conversion: Conversion = serde_json::from_str(
            r#"{"roots":[{"archetype_id":"openEHR-EHR-OBSERVATION.x.v1","adl2":"archetype (adl_version=2.4)","log":{"external_at_codes":{},"value_sets":{},"notes":["kept at0000"]},"conversion_details":{"source_model":"openEHR-EHR-OBSERVATION.x.v1"}}],"structure":[]}"#,
        )
        .expect("the conversion reads");
        assert_eq!(conversion.roots[0].log.notes, ["kept at0000"]);
    }
}
