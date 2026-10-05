// @generated-from-template templates/openehr-rm/ehr/ehr_access_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `EHR_ACCESS`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr_access.adoc`
//! §Functions + §Invariants.

use crate::v1_1::ehr::ehr_access::EhrAccess;

impl EhrAccess {
    /// Returns the name of the access control scheme in use.
    ///
    /// Spec: `org.openehr.rm.ehr.ehr_access.adoc` §Functions `scheme` — "The
    /// name of the access control scheme in use; corresponds to the concrete
    /// instance of the `settings` attribute." The scheme is therefore that
    /// instance's own type name, which is exactly what the open-subtype
    /// carrier keeps: `settings` is declared as a subtype of the abstract
    /// `ACCESS_CONTROL_SETTINGS` ("allowing for the use of different access
    /// control schemes", §Attributes), so the concrete type is carried on the
    /// value rather than fixed by the model.
    ///
    /// `None` when no settings are recorded. The class declares `scheme`
    /// `1..1` with §Invariants `Scheme_valid: not scheme.is_empty` while
    /// declaring `settings` `0..1`, so an `EHR_ACCESS` with no settings has no
    /// scheme to name — and the carrier refuses an empty type name, so
    /// whenever settings ARE present `Scheme_valid` holds by construction.
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
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

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
