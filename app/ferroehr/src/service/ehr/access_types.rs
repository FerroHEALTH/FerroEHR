// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parsed `ferroehr.access_control.v1` scheme settings the protocol
//! adapter's out-of-band access gate consumes.
//!
//! `EHR_ACCESS` is a mandatory, versioned component of every EHR and the
//! openEHR access-decision authority: "All access decisions to data in the EHR
//! must be made in accordance with the policies and rules in this object" (RM
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class). Its
//! `settings: ACCESS_CONTROL_SETTINGS [0..1]` is abstract and attribute-less,
//! "Currently implementation dependent" (RM
//! `org.openehr.rm.ehr.access_control_settings.adoc`), and `scheme(): String`
//! names the concrete settings instance (`Scheme_valid: not scheme.is_empty`).
//! BASE `architecture_overview/master07-security.adoc` §Access Control describes
//! what a scheme should provide while noting "there is currently no published
//! formal, proven model of access control for shared health information".
//!
//! Everything below the store, version and audit obligation is therefore an
//! extension: no openEHR spec governs the concrete scheme — our own design. The
//! SM likewise defines no `I_EHR_ACCESS` interface, placing authorisation out of
//! band (`openehr_platform/master02-overview.adoc` §General Assumptions), so the
//! settings read (`FerroEhrService::current_ehr_access_settings`) is a
//! native-API extension exposing the current scheme settings for the protocol
//! adapter to enforce after authentication.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `_type` discriminator of this scheme's `ACCESS_CONTROL_SETTINGS`
/// subtype on the wire (canonical JSON). `EHR_ACCESS.scheme()` derives from
/// it.
///
/// No openEHR spec governs the concrete scheme — our own design.
pub const EHR_ACCESS_CONTROL_V1_TYPE: &str = "FERROEHR_ACCESS_CONTROL_V1";

/// The scheme name `EHR_ACCESS.scheme()` reports for settings of this type
/// (`Scheme_valid` — RM `org.openehr.rm.ehr.ehr_access.adoc`).
pub const EHR_ACCESS_CONTROL_V1_SCHEME: &str = "ferroehr.access_control.v1";

/// The default access disposition of an EHR whose `EHR_ACCESS.settings` use
/// this scheme (`master07` "sensible defaults").
///
/// No openEHR spec governs the concrete scheme — our own design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultAccess {
    /// Every authenticated caller may touch the EHR (the default; keeps every
    /// existing EHR working — `master07` "sensible defaults").
    #[default]
    Open,
    /// Only principals matched by the
    /// [`access_list`](EhrAccessSettings::access_list) may touch the EHR.
    Restricted,
}

/// The access kind granted to a matched principal.
///
/// No openEHR spec governs the concrete scheme — our own design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    /// No privacy ceiling — the principal may read every Composition.
    Full,
    /// The principal may read Compositions whose privacy level is strictly
    /// below [`AccessEntry::max_level`].
    RestrictedBelow,
}

/// One entry of the access list: a principal and the access it is granted
/// (`master07` §Access Control — "identified individuals" and "categories").
///
/// No openEHR spec governs the concrete scheme — our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessEntry {
    /// `user:<authenticated id>` (Basic username / OIDC subject) or
    /// `role:<name>` (a category matched against the caller's roles).
    pub principal: String,
    /// Whether the grant is unrestricted or capped by a privacy ceiling.
    pub access: AccessLevel,
    /// The exclusive privacy ceiling for [`AccessLevel::RestrictedBelow`]
    /// (readable iff Composition level `< max_level`); ignored for
    /// [`AccessLevel::Full`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_level: Option<i64>,
}

/// Per-Composition privacy levels.
///
/// The meaning of a level is deliberately deployment-defined ("the definition
/// of the privacy levels is not hard-wired in the openEHR models but rather
/// is defined by standards or agreements within jurisdictions of use" — BASE
/// `architecture_overview/master07-security.adoc` §Access Control).
///
/// No openEHR spec governs the concrete scheme — our own design.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Privacy {
    /// The privacy level of every Composition without an override.
    #[serde(default)]
    pub default_level: i64,
    /// Per-versioned-object privacy-level pins.
    #[serde(default)]
    pub composition_overrides: Vec<CompositionOverride>,
}

impl Privacy {
    /// The effective privacy level of the versioned Composition addressed by
    /// `target_vo_id` (a bare versioned-object uid): its override if pinned,
    /// else [`default_level`](Self::default_level). `target_vo_id` is compared
    /// against each override `uid` on its versioned-object head (the part
    /// before any `::` of an `OBJECT_VERSION_ID` — BASE
    /// `object_version_id.adoc`, `object_id '::' … '::' version_tree_id`).
    #[must_use]
    pub fn level_for(&self, target_vo_id: &str) -> i64 {
        let head = vo_head(target_vo_id);
        self.composition_overrides
            .iter()
            .find(|o| vo_head(&o.uid) == head)
            .map_or(self.default_level, |o| o.level)
    }
}

/// A privacy-level pin for one versioned Composition.
///
/// No openEHR spec governs the concrete scheme — our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionOverride {
    /// The `VERSIONED_COMPOSITION` uid (versioned-object head).
    pub uid: String,
    /// The pinned privacy level.
    pub level: i64,
}

/// The EHRs whose `EHR_ACCESS` carries settings this server understands, with
/// those settings, shared from the service's cache
/// ([`crate::service::FerroEhrService::ehr_access_settings_index`]).
pub type EhrAccessIndex = std::sync::Arc<Vec<(crate::ids::EhrId, EhrAccessSettings)>>;

/// Parsed `EHR_ACCESS.settings` for the `ferroehr.access_control.v1` scheme.
///
/// No openEHR spec governs the concrete scheme — our own design; it realizes the `master07` policy
/// prose (access list + gate-keeper + privacy levels + sensible defaults).
/// Deserialization is tolerant: every field defaults, so an absent or partial
/// object still yields a usable (default-open) value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EhrAccessSettings {
    /// The principal (`user:`/`role:` form) that alone may commit a new
    /// `EHR_ACCESS` version once set (`master07` §Access Control — the
    /// gate-keeper). `None` = no gate-keeper (any authorised caller may change
    /// the settings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_keeper: Option<String>,
    /// The default disposition when no access-list entry matches.
    #[serde(default)]
    pub default_access: DefaultAccess,
    /// The access list, evaluated first-match-wins.
    #[serde(default)]
    pub access_list: Vec<AccessEntry>,
    /// The per-Composition privacy levels.
    #[serde(default)]
    pub privacy: Privacy,
}

impl EhrAccessSettings {
    /// Parse the settings of the `ferroehr.access_control.v1` scheme from an
    /// `EHR_ACCESS` canonical-JSON object. Returns `None` when there are no
    /// settings, they belong to another scheme, or they cannot be parsed as
    /// this scheme — all of which the caller treats as **default-open** (the
    /// gateway clause is dead weight without a scheme it understands — RM
    /// `org.openehr.rm.ehr.ehr_access.adoc`).
    #[must_use]
    pub fn from_ehr_access(access: &Value) -> Option<Self> {
        let settings = access.get("settings")?;
        let is_our_scheme =
            settings.get("_type").and_then(Value::as_str) == Some(EHR_ACCESS_CONTROL_V1_TYPE);
        if !is_our_scheme {
            return None;
        }
        serde_json::from_value(settings.clone()).ok()
    }

    /// The first access-list entry matching the given principal:
    /// `user:<subject>` or any `role:<r>` (`master07`'s "identified
    /// individuals" / "categories").
    #[must_use]
    pub fn match_principal(&self, subject: Option<&str>, roles: &[String]) -> Option<&AccessEntry> {
        self.access_list
            .iter()
            .find(|e| principal_matches(&e.principal, subject, roles))
    }
}

/// Whether a scheme principal string (`user:<id>` / `role:<name>`) matches
/// the authenticated caller identified by `subject` (Basic username / OIDC
/// subject) and `roles`.
///
/// Role comparison is case-insensitive (roles are normalised upper-case at
/// authentication).
#[must_use]
pub fn principal_matches(principal: &str, subject: Option<&str>, roles: &[String]) -> bool {
    if let Some(user) = principal.strip_prefix("user:") {
        return subject == Some(user);
    }
    if let Some(role) = principal.strip_prefix("role:") {
        return roles.iter().any(|r| r.eq_ignore_ascii_case(role));
    }
    false
}

/// The versioned-object head of a uid — the part before the first `::` of an
/// `OBJECT_VERSION_ID`
/// (`object_id '::' creating_system_id '::' version_tree_id` — BASE
/// `object_version_id.adoc`), or the whole string for a bare uid.
fn vo_head(uid: &str) -> &str {
    uid.split("::").next().unwrap_or(uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_full_scheme() {
        let access = json!({
            "_type": "EHR_ACCESS",
            "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
            "name": { "_type": "DV_TEXT", "value": "EHR Access" },
            "settings": {
                "_type": EHR_ACCESS_CONTROL_V1_TYPE,
                "gate_keeper": "user:alice",
                "default_access": "restricted",
                "access_list": [
                    { "principal": "user:bob", "access": "full" },
                    { "principal": "role:nurse", "access": "restricted_below", "max_level": 2 }
                ],
                "privacy": {
                    "default_level": 0,
                    "composition_overrides": [
                        { "uid": "8849182c-82ad-4088-a07f-48ead4180515", "level": 3 }
                    ]
                }
            }
        });
        let s = EhrAccessSettings::from_ehr_access(&access).expect("our scheme");
        assert_eq!(s.gate_keeper.as_deref(), Some("user:alice"));
        assert_eq!(s.default_access, DefaultAccess::Restricted);
        assert_eq!(s.access_list.len(), 2);
        assert_eq!(s.access_list[1].access, AccessLevel::RestrictedBelow);
        assert_eq!(s.access_list[1].max_level, Some(2));
        // Override matches on the versioned-object head, even given an OVID.
        assert_eq!(
            s.privacy
                .level_for("8849182c-82ad-4088-a07f-48ead4180515::sys::2"),
            3
        );
        assert_eq!(s.privacy.level_for("other-vo"), 0);
    }

    #[test]
    fn absent_settings_and_foreign_scheme_are_none() {
        assert!(EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS" })).is_none());
        assert!(
            EhrAccessSettings::from_ehr_access(&json!({
                "_type": "EHR_ACCESS",
                "settings": { "_type": "SOME_OTHER_SCHEME" }
            }))
            .is_none()
        );
    }

    #[test]
    fn principal_matching() {
        let roles = vec!["NURSE".to_owned(), "USER".to_owned()];
        assert!(principal_matches("user:bob", Some("bob"), &[]));
        assert!(!principal_matches("user:bob", Some("carol"), &[]));
        assert!(!principal_matches("user:bob", None, &roles));
        // role match is case-insensitive against the upper-cased roles.
        assert!(principal_matches("role:nurse", Some("bob"), &roles));
        assert!(!principal_matches("role:doctor", Some("bob"), &roles));
    }
}
