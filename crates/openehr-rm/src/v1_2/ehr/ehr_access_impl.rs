// @generated-from-template templates/openehr-rm/ehr/ehr_access_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `EHR_ACCESS.scheme` (`org.openehr.rm.ehr.ehr_access.adoc`).

use crate::v1_2::ehr::ehr_access::EhrAccess;

impl EhrAccess {
    /// The access control scheme: the type name of the `settings` instance
    /// (`scheme`), or `None` when no settings are recorded.
    #[must_use]
    pub fn scheme(&self) -> Option<&str> {
        self.settings
            .as_ref()
            .map(openehr_base::serde_support::OpenSubtype::type_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};

    fn text(value: &str) -> DvText {
        DvText::DvText(DvTextData {
            value: value.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        })
    }

    /// An `EHR_ACCESS` whose settings are an instance of `scheme`, or with no
    /// settings at all.
    #[expect(
        clippy::disallowed_types,
        reason = "an open subtype's members ARE an untyped JSON map — that is what makes the settings extensible (#1694 boundary class)"
    )]
    fn access(scheme: Option<&str>) -> Option<EhrAccess> {
        let settings = match scheme {
            Some(scheme) => Some(
                openehr_base::serde_support::OpenSubtype::new(scheme, serde_json::Map::new())
                    .ok()?,
            ),
            None => None,
        };
        Some(EhrAccess {
            name: text("access"),
            archetype_node_id: "openEHR-EHR-EHR_ACCESS.generic.v1".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            settings,
        })
    }

    /// "Corresponds to the concrete instance of the `settings` attribute" —
    /// the scheme is that instance's own type name, whatever it is.
    #[test]
    fn the_scheme_is_the_settings_concrete_type() {
        let settings = access(Some("OPENEHR_ACCESS_CONTROL_SETTINGS")).expect("a named subtype");
        assert_eq!(settings.scheme(), Some("OPENEHR_ACCESS_CONTROL_SETTINGS"));

        let other = access(Some("SOME_OTHER_ACCESS_CONTROL")).expect("a named subtype");
        assert_eq!(other.scheme(), Some("SOME_OTHER_ACCESS_CONTROL"));
    }

    /// `settings` is `0..1`: with none recorded there is no concrete instance
    /// to name, and `Scheme_valid` has nothing it could hold of.
    #[test]
    fn no_settings_means_no_scheme() {
        let access = access(None).expect("no settings is a valid EHR_ACCESS");
        assert!(access.scheme().is_none());
    }
}
