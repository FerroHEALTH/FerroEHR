// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `access_directives` scheme of `EHR_ACCESS.settings`: the one scheme
//! FerroEHR writes and reads.
//!
//! `EHR_ACCESS` is the openEHR access-decision authority: "All access
//! decisions to data in the EHR must be made in accordance with the policies
//! and rules in this object" (RM `org.openehr.rm.ehr.ehr_access.adoc`
//! §`EHR_ACCESS` Class). Its `settings: ACCESS_CONTROL_SETTINGS [0..1]` is
//! abstract and "Currently implementation dependent" (RM
//! `org.openehr.rm.ehr.access_control_settings.adoc`), and `scheme()` names
//! the concrete subtype. FerroEHR's subtype is the draft class
//! `ACCESS_DIRECTIVE_SETTINGS` ([`AccessDirectiveSettings`], generated from
//! the draft access control BMM), checked against its BMM invariants and the
//! access control terminology before it is stored. No openEHR spec governs
//! the concrete scheme: our own design (#5487).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;

use crate::access_control_draft::access_control::access_directive_settings::AccessDirectiveSettings;
use crate::error::ServiceError;

/// The `_type` of the settings on the wire (canonical JSON).
pub const SETTINGS_TYPE: &str = "ACCESS_DIRECTIVE_SETTINGS";

/// The scheme name `EHR_ACCESS.scheme()` reports for settings of
/// [`SETTINGS_TYPE`] (`Scheme_valid`, RM `org.openehr.rm.ehr.ehr_access.adoc`).
pub const SCHEME: &str = "access_directives";

/// The default definition of the scheme's settings, as its file name and ADL 2
/// text.
///
/// Every boot stores it when no definition of its id is stored
/// ([`crate::service::FerroEhrService::seed_default_definitions`]). FerroEHR
/// reads the settings by their codes, never by this archetype's id.
pub const DEFINITIONS: [(&str, &str); 1] = [(
    "openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls",
    include_str!(
        "../../../assets/definitions/access/openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls"
    ),
)];

/// What an EHR admits where no directive applies: the
/// `ACCESS_DIRECTIVE_SETTINGS.default_access` code of the `access default`
/// group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAccess {
    /// Any caller the other layers admit.
    Open,
    /// Only the callers a permit directive, a gate-keeper or a care
    /// relationship names.
    Restricted,
    /// Only the callers a permit directive names, apart from the subject and
    /// an emergency override.
    ConsentRequired,
}

impl DefaultAccess {
    /// The disposition `coded` names, or `None` for a code outside the
    /// `access default` group.
    fn read(coded: &DvCodedText) -> Option<Self> {
        if coded.defining_code.terminology_id.value
            != crate::party_binding::policy_mark::TERMINOLOGY
        {
            return None;
        }
        match coded.defining_code.code_string.as_str() {
            "open" => Some(Self::Open),
            "restricted" => Some(Self::Restricted),
            "consent_required" => Some(Self::ConsentRequired),
            _ => None,
        }
    }
}

/// The EHRs whose current `EHR_ACCESS` carries settings, with those settings,
/// shared from the service's cache
/// ([`crate::service::FerroEhrService::ehr_access_settings_index`]).
pub type EhrAccessIndex = std::sync::Arc<Vec<(crate::ids::EhrId, EhrAccessSettings)>>;

/// The current `EHR_ACCESS.settings` of an EHR, as the access rules read them.
#[derive(Debug, Clone, PartialEq)]
pub enum EhrAccessSettings {
    /// Settings of the `access_directives` scheme.
    Directives(Box<AccessDirectiveSettings>),
    /// Stored settings that do not read as the scheme: the EHR is held
    /// closed, since a privacy control that cannot be read is never read as
    /// permissive.
    HeldClosed,
}

/// Stored settings that do not read as the `access_directives` scheme.
#[derive(Debug, thiserror::Error)]
#[error("the EHR_ACCESS settings do not read as the {SCHEME} scheme: {0}")]
pub struct UnreadableSettings(String);

impl EhrAccessSettings {
    /// Reads the settings of an `EHR_ACCESS` canonical-JSON object; `None`
    /// when it carries none.
    ///
    /// # Errors
    /// [`UnreadableSettings`] when the settings are not settings of the
    /// `access_directives` scheme that pass [`check_written_settings`]; the
    /// caller holds the EHR closed.
    pub fn from_ehr_access(access: &Value) -> Result<Option<Self>, UnreadableSettings> {
        let Some(settings) = access.get("settings").filter(|s| !s.is_null()) else {
            return Ok(None);
        };
        read_settings(settings)
            .map(|read| Some(Self::Directives(Box::new(read))))
            .map_err(|refusal| UnreadableSettings(refusal.to_string()))
    }

    /// The settings `access` carries, or [`Self::HeldClosed`] for settings
    /// that do not read; `None` when it carries none.
    #[must_use]
    pub fn held_or_read(access: &Value) -> Option<Self> {
        Self::from_ehr_access(access).unwrap_or_else(|error| {
            let cause: &(dyn std::error::Error + 'static) = &error;
            tracing::error!(
                error = cause,
                "EHR_ACCESS settings do not read; the EHR is held closed"
            );
            Some(Self::HeldClosed)
        })
    }

    /// What the EHR admits where no directive applies: the settings'
    /// `default_access`, and [`DefaultAccess::Restricted`] for an EHR held
    /// closed.
    #[must_use]
    pub fn default_access(&self) -> DefaultAccess {
        match self {
            Self::Directives(settings) => {
                DefaultAccess::read(&settings.default_access).unwrap_or(DefaultAccess::Restricted)
            }
            Self::HeldClosed => DefaultAccess::Restricted,
        }
    }

    /// The parties who may commit the next `EHR_ACCESS` version; none for an
    /// EHR held closed.
    #[must_use]
    pub fn gate_keepers(&self) -> &[PartyProxy] {
        match self {
            Self::Directives(settings) => settings.gate_keepers.as_deref().unwrap_or_default(),
            Self::HeldClosed => &[],
        }
    }
}

/// The defect of written settings that are not settings of the scheme.
#[derive(Debug, thiserror::Error)]
enum Defect {
    /// The settings name another scheme, or none.
    #[error(
        "EHR_ACCESS.settings must be an {SETTINGS_TYPE} of the {SCHEME} scheme; _type {0} is no \
         scheme this server keeps"
    )]
    Scheme(String),
    /// The settings do not decode as the class.
    #[error("EHR_ACCESS.settings do not read as an {SETTINGS_TYPE}: {0}")]
    Decode(#[source] serde_json::Error),
    /// The settings break a BMM invariant of the draft access control.
    #[error("EHR_ACCESS.settings break the invariant {0}")]
    Invariant(crate::draft_invariants::Violation),
}

/// Reads written settings as the scheme, checking every invariant.
fn read_settings(settings: &Value) -> Result<AccessDirectiveSettings, Defect> {
    let scheme = settings.get("_type").and_then(Value::as_str);
    if scheme != Some(SETTINGS_TYPE) {
        return Err(Defect::Scheme(
            scheme.map_or_else(|| "absent".to_owned(), |name| format!("{name:?}")),
        ));
    }
    let read = serde_json::from_value::<AccessDirectiveSettings>(settings.clone())
        .map_err(Defect::Decode)?;
    if let Err(violations) = crate::access_control_invariants::check_settings(&read)
        && let Some(first) = violations.0.first()
    {
        return Err(Defect::Invariant(*first));
    }
    Ok(read)
}

/// Checks written `EHR_ACCESS.settings` before they are committed.
///
/// They must be an `ACCESS_DIRECTIVE_SETTINGS` that the strict
/// canonical-JSON reader reads and whose every BMM invariant and terminology
/// group holds.
///
/// Settings of any other scheme are refused, so a body of a scheme this
/// server no longer keeps never governs an EHR. No openEHR spec governs the
/// scheme: our own design.
///
/// # Errors
/// [`ServiceError::bad_request`] (400) naming the first defect: another
/// scheme's `_type`, a member that does not decode, or the first broken
/// invariant as `CLASS.Invariant`.
pub fn check_written_settings(settings: &Value) -> Result<(), ServiceError> {
    read_settings(settings)
        .map(drop)
        .map_err(|defect| ServiceError::bad_request(defect.to_string(), defect))
}

#[cfg(test)]
mod tests;
