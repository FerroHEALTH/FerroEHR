// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's access settings (`EHR_ACCESS`) over the CDR's extension.
//!
//! The settings in force and any version are read at `{ext}/ehr/{id}/ehr_access`
//! and their history at `versioned_ehr_access/revision_history`; an
//! administrator commits new settings on the person's instruction at
//! `{ext}/admin/ehr-access`, naming the version they replace, and repairs
//! settings that do not parse at `/repair`. No openEHR specification governs
//! these routes: FerroEHR's own design over its draft access control package.

use crate::bff::access_policy::Shown;
use crate::bff::wire::VersionEntry;
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// One version of an EHR's access settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessSettings {
    /// The version's uid.
    pub version_uid: String,
    /// The `EHR_ACCESS.settings` pretty-printed, empty for none.
    pub settings: String,
    /// The settings' attributes, read for drawing.
    pub attributes: Vec<(String, Shown)>,
}

/// The settings in force, the version `version_uid` names when it is not
/// empty, or `None` while the EHR holds none (`404`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not JSON.
pub async fn fetch_settings(
    ehr_id: String,
    version_uid: String,
) -> Result<Option<AccessSettings>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let path = if version_uid.is_empty() {
        format!("ehr/{}/ehr_access", urlencoding::encode(&ehr_id))
    } else {
        format!(
            "ehr/{}/ehr_access/{}",
            urlencoding::encode(&ehr_id),
            urlencoding::encode(&version_uid)
        )
    };
    let url = cdr.extensions_v1(&path);
    let response = cdr.get(&url).accept("application/json").send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let response = response.ok()?;
    let served = response.etag_version_uid();
    let mut settings = parse_settings(&response.body)?;
    if let Some(served) = served.filter(|_| settings.version_uid.is_empty()) {
        settings.version_uid = served;
    }
    Ok(Some(settings))
}

/// The versions of an EHR's access settings, newest first.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the history is not valid JSON.
pub async fn fetch_settings_history(ehr_id: String) -> Result<Vec<VersionEntry>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "ehr/{}/versioned_ehr_access/revision_history",
        urlencoding::encode(&ehr_id)
    ));
    crate::bff::wire::revision_history(&cdr, &url).await
}

/// Commits `settings` (an `ACCESS_DIRECTIVE_SETTINGS`, or `null` for none) as
/// the EHR's next access settings over `preceding`, on the person's
/// instruction (`POST admin/ehr-access`); the new version's uid.
///
/// # Errors
/// [`ViewerError::Rejected`] when `settings` is not JSON; the errors of a
/// CDR call ([`cdr`](crate::bff::cdr)), the CDR's `400`, `403`, `412` and
/// `422` included.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer sends the canonical-JSON ACCESS_DIRECTIVE_SETTINGS of the draft access \
              control package inside the admin extension's JSON body"
)]
pub async fn commit_settings(
    ehr_id: String,
    preceding: String,
    settings: String,
) -> Result<String, ViewerError> {
    let settings: serde_json::Value = serde_json::from_str(&settings)
        .map_err(|error| ViewerError::rejected(tr(PolicyMsg::AccessSettingsJson), error))?;
    let body = serde_json::json!({
        "ehr_id": ehr_id, "preceding_version_uid": preceding, "settings": settings,
    });
    post_admin("admin/ehr-access", body.to_string()).await
}

/// Replaces settings that do not parse with `settings`
/// (`POST admin/ehr-access/repair`); the new version's uid.
///
/// # Errors
/// [`ViewerError::Rejected`] when `settings` is not JSON; the errors of a
/// CDR call ([`cdr`](crate::bff::cdr)), the CDR's `400`, `403` and `422`
/// included.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer sends the canonical-JSON ACCESS_DIRECTIVE_SETTINGS of the draft access \
              control package inside the admin extension's JSON body"
)]
pub async fn repair_settings(ehr_id: String, settings: String) -> Result<String, ViewerError> {
    let settings: serde_json::Value = serde_json::from_str(&settings)
        .map_err(|error| ViewerError::rejected(tr(PolicyMsg::AccessSettingsJson), error))?;
    let body = serde_json::json!({ "ehr_id": ehr_id, "settings": settings });
    post_admin("admin/ehr-access/repair", body.to_string()).await
}

/// Posts `body` to the admin route `path`; the `version_uid` it answers.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the admin extension's JSON answer"
)]
async fn post_admin(path: &str, body: String) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(path);
    let answer = cdr
        .post(&url)
        .accept("application/json")
        .body("application/json", body)
        .ok()
        .await?
        .body;
    // NOTE: no specification governs this: our own design; the CDR answered 200,
    // so an answer without a `version_uid` leaves only the toast's version out.
    Ok(serde_json::from_str::<serde_json::Value>(&answer)
        .ok()
        .and_then(|value| {
            value
                .get("version_uid")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default())
}

/// The settings an `EHR_ACCESS` body carries.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON EHR_ACCESS of the draft access control package"
)]
pub fn parse_settings(body: &str) -> Result<AccessSettings, ViewerError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::AccessSettingsJson), error))?;
    let settings = value.get("settings").filter(|settings| !settings.is_null());
    let attributes = match settings.map(Shown::of) {
        Some(Shown::Fields { fields, .. }) => fields,
        _ => Vec::new(),
    };
    Ok(AccessSettings {
        version_uid: crate::uid::uid_value_of_document(&value),
        settings: settings
            .and_then(|settings| serde_json::to_string_pretty(settings).ok())
            .unwrap_or_default(),
        attributes,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_settings;
    use crate::bff::access_policy::Shown;

    #[test]
    fn the_settings_read_their_directives() {
        let settings = parse_settings(
            r#"{"_type":"EHR_ACCESS","uid":{"value":"s1::localhost::2"},
                "settings":{"_type":"ACCESS_DIRECTIVE_SETTINGS",
                  "directives":[{"_type":"ACCESS_DIRECTIVE","name":{"_type":"DV_TEXT","value":"No research"},
                    "emergency_override":true}]}}"#,
        )
        .expect("settings");
        assert_eq!("s1::localhost::2", settings.version_uid);
        let directives = settings
            .attributes
            .iter()
            .find(|(key, _)| key == "directives")
            .map(|(_, value)| value.clone());
        assert!(matches!(directives, Some(Shown::List(items)) if items.len() == 1));
    }

    #[test]
    fn settings_of_none_read_as_empty() {
        let settings = parse_settings(
            r#"{"_type":"EHR_ACCESS","uid":{"value":"s1::localhost::1"},"settings":null}"#,
        )
        .expect("settings");
        assert_eq!("", settings.settings);
        assert_eq!(0, settings.attributes.len());
    }
}
