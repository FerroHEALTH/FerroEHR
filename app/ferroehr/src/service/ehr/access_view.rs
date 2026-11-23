// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a reader of an EHR's `EHR_ACCESS` is served: the settings whole, the
//! settings without the record's confidentiality marking, or nothing.
//!
//! The draft access control decision procedure, Reading and Writing the
//! Settings, item 3: the subject and the controller read the settings, and a
//! representative and a gate-keeper other than the subject read them without
//! the marking. A version served without its marking does not hold the data its
//! `signature` signs, so that view carries no signature and the response says
//! it is redacted (our own design, #6165).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use serde_json::Value;

/// The attribute of `CONFIDENTIALITY_SETTINGS` a redacted view leaves out.
pub const MARKING: &str = "marking";

/// How a caller reads the `EHR_ACCESS` settings of one EHR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsView {
    /// The settings as stored: the subject and the controller.
    Whole,
    /// The settings without `marking`: a representative or a gate-keeper who
    /// is not the subject.
    WithoutMarking,
    /// No settings at all: the `EHR_ACCESS` reads as absent (EHDS Art. 8).
    Hidden,
}

impl SettingsView {
    /// Whether the caller reads the settings in any form.
    #[must_use]
    pub fn reads(self) -> bool {
        self != Self::Hidden
    }

    /// Whether the caller reads the settings without their marking.
    #[must_use]
    pub fn redacts(self) -> bool {
        self == Self::WithoutMarking
    }
}

/// Removes the marking from the `settings` of `object`, an `EHR_ACCESS`.
pub fn redact_object(object: &mut Value) {
    if let Some(settings) = object.get_mut("settings") {
        redact_settings(settings);
    }
}

/// Removes the marking from `settings`, an `ACCESS_CONTROL_SETTINGS`.
pub fn redact_settings(settings: &mut Value) {
    if let Some(members) = settings.as_object_mut() {
        members.remove(MARKING);
    }
}

/// Removes the marking from the `data` of `version`, a `VERSION` of an
/// `EHR_ACCESS`, and its `signature`, which does not sign what is served.
pub fn redact_version(version: &mut Value) {
    if let Some(data) = version.get_mut("data") {
        redact_object(data);
    }
    if let Some(members) = version.as_object_mut() {
        members.remove("signature");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object() -> Value {
        json!({ "_type": "EHR_ACCESS", "settings": {
            "_type": "ACCESS_DIRECTIVE_SETTINGS",
            "marking": { "_type": "CONFIDENTIALITY_MARKING" },
            "gate_keepers": [] } })
    }

    #[test]
    fn a_redacted_object_keeps_every_setting_but_the_marking() {
        let mut served = object();
        redact_object(&mut served);
        assert_eq!(
            served,
            json!({ "_type": "EHR_ACCESS", "settings": {
                "_type": "ACCESS_DIRECTIVE_SETTINGS", "gate_keepers": [] } })
        );
    }

    #[test]
    fn a_redacted_version_carries_no_signature() {
        let mut served = json!({ "_type": "ORIGINAL_VERSION", "signature": "x", "data": object() });
        redact_version(&mut served);
        assert!(served.get("signature").is_none());
        assert!(served["data"]["settings"].get(MARKING).is_none());
        assert!(served["data"]["settings"].get("gate_keepers").is_some());
    }

    #[test]
    fn an_object_without_settings_is_left_as_it_is() {
        let mut served = json!({ "_type": "EHR_ACCESS" });
        redact_object(&mut served);
        assert_eq!(served, json!({ "_type": "EHR_ACCESS" }));
    }

    #[test]
    fn only_the_hidden_view_reads_nothing() {
        assert!(SettingsView::Whole.reads() && !SettingsView::Whole.redacts());
        assert!(SettingsView::WithoutMarking.reads() && SettingsView::WithoutMarking.redacts());
        assert!(!SettingsView::Hidden.reads());
    }
}
