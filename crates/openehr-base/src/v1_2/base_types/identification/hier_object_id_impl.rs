// @generated-from-template templates/openehr-base/base_types/identification/hier_object_id_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written validating construction for `HIER_OBJECT_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.hier_object_id.adoc`
//! and `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//! §Syntaxes, which gives the lexical form as
//! `hier_object_id = uid_based_id` and `uid_based_id = root, [ '::', extension ]`
//! with `root = uid` and `extension = ? any string ?`.
//!
//! The accessors (`root`, `extension`, `has_extension`, `is_equal`) live in the
//! shared [`uid_based_id_impl`](super::uid_based_id_impl) macro; this sibling
//! adds the construction door: [`HierObjectId::new`] / `TryFrom<&str>` /
//! `FromStr` run the §Syntaxes grammar, so an identifier built through them is
//! well-formed by construction.
//!
//! Scope of the guarantee: the generated struct's `value` field is `pub(crate)`
//! (the emitter's construction-door scheme), so outside `openehr-base` this
//! constructor and the total [`From<Uid>`] conversion are the ONLY ways to
//! obtain a `HIER_OBJECT_ID` — the type cannot hold a malformed value anywhere
//! in the model, and the canonical-JSON/XML readers build through this door
//! too.

use std::str::FromStr;

use super::hier_object_id::HierObjectId;
use super::lexical::{IdComponent, IdError, IdProduction, is_uid};
use super::uid::Uid;
use super::uid_based_id_impl::root_str;

impl HierObjectId {
    /// Build a `HIER_OBJECT_ID` from its string form, validating the BASE
    /// `master05-identification_package.adoc` §Syntaxes grammar
    /// (`root, [ '::', extension ]`, `root = uid`).
    ///
    /// The value is stored verbatim — the case-**preserving** half of master05
    /// §"Composite Identifiers and Case" ("not change case due to persistence,
    /// copying, transfer or other computation processes").
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value, and [`IdError::Malformed`] when
    /// the `root` part is not a legal `uid` (`iso_oid | uuid | internet_id`).
    /// The `extension` is `? any string ?` in the grammar and is therefore not
    /// constrained — including the empty string, which the grammar admits and
    /// which `has_extension()` reports as absent.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        let root = root_str(&value);
        if !is_uid(root) {
            return Err(IdError::Malformed {
                component: IdComponent::Root,
                expected: IdProduction::Uid,
                found: root.to_owned(),
            });
        }
        Ok(Self { value })
    }
}

impl From<Uid> for HierObjectId {
    /// A bare `UID` **is** a `HIER_OBJECT_ID` — total by grammar, not by
    /// convention: BASE `master05-identification_package.adoc` §Syntaxes gives
    /// `hier_object_id = uid_based_id`, `uid_based_id = root, [ '::',
    /// extension ]` and `root = uid`, so the extension-less form of the
    /// production is exactly a `uid`, which the [`Uid`] type already carries a
    /// grammar-checked value of.
    ///
    /// This is the conversion every derived-identifier accessor uses (e.g. RM
    /// `VERSION.owner_id`, extracted from the version id's `object_id`), so a
    /// derivation can never need a fallible constructor for a value the model
    /// has already validated.
    ///
    /// The rendered value is [`Uid::value`], which normalises a `UUID` to the
    /// RFC 4122 lower-case form. That is a *rendering*, not a re-identification:
    /// master05 §"Composite Identifiers and Case" makes two identifiers
    /// differing only in case the same identifier. The case-PRESERVING half of
    /// the same section binds a *stored* value, which this conversion does not
    /// touch.
    fn from(uid: Uid) -> Self {
        Self {
            value: uid.value().into_owned(),
        }
    }
}

impl From<uuid::Uuid> for HierObjectId {
    /// A UUID **is** a `HIER_OBJECT_ID` — total by grammar: §Syntaxes chains
    /// `hier_object_id = uid_based_id`, `root = uid` and
    /// `uid = iso_oid | uuid | internet_id`, so the extension-less form of a
    /// parsed UUID is a legal `hier_object_id` with nothing left to check.
    ///
    /// The rendering is the RFC 4122 lower-case hyphenated form — exactly the
    /// `uuid` production §Syntaxes gives.
    fn from(value: uuid::Uuid) -> Self {
        Self {
            value: value.to_string(),
        }
    }
}

impl FromStr for HierObjectId {
    type Err = IdError;

    /// Parse a `HIER_OBJECT_ID` — see [`HierObjectId::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for HierObjectId {
    type Error = IdError;

    /// Parse a `HIER_OBJECT_ID` — see [`HierObjectId::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for HierObjectId {
    type Error = IdError;

    /// Parse a `HIER_OBJECT_ID` — see [`HierObjectId::new`].
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::base_types::identification::uid::Uid;
    use crate::v1_2::base_types::identification::uid_based_id_impl::extension_str;

    /// The §Syntaxes forms: `root` alone, and `root '::' extension`, with each
    /// of the three `uid` productions in root position.
    #[test]
    fn accepts_every_root_production() {
        for raw in [
            "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11",
            "1.2.840.113554.3.7.10",
            "12345",
            "openEHR.org",
            "uk.nhs.ehr1::patient-42",
            // The extension is `? any string ?` — anything goes right of `::`.
            "openehr.org:: any string at all ::still the extension",
            "openehr.org::",
        ] {
            let id = HierObjectId::new(raw).expect("a well-formed identifier");
            assert_eq!(id.value, raw, "the value is stored verbatim");
        }
    }

    /// A root that matches none of `iso_oid | uuid | internet_id` is refused,
    /// and the error names the component and the expected production.
    #[test]
    fn refuses_a_malformed_root() {
        for raw in ["1234-5678", "-leading.org", "has space", "a..b"] {
            let err = HierObjectId::new(raw).expect_err("must refuse");
            assert_eq!(
                err,
                IdError::Malformed {
                    component: IdComponent::Root,
                    expected: IdProduction::Uid,
                    found: raw.split("::").next().unwrap_or(raw).to_owned(),
                },
                "for {raw:?}"
            );
        }
        // The refusal applies to the root only — a bad string right of `::` is
        // a legal extension.
        assert!(HierObjectId::new("1234-5678::x").is_err());
        assert!(HierObjectId::new("x::1234-5678").is_ok());
        assert_eq!(HierObjectId::new(""), Err(IdError::Empty));
    }

    /// Construction and the inherited accessors agree.
    #[test]
    fn accessors_see_the_constructed_parts() {
        let id = "1.2.840::extension-part"
            .parse::<HierObjectId>()
            .expect("a well-formed identifier");
        assert!(matches!(id.root(), Uid::IsoOid(_)));
        assert_eq!(id.extension(), "extension-part");
        assert!(id.has_extension());
        assert_eq!(extension_str(&id.value), "extension-part");

        let plain = HierObjectId::try_from("2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11")
            .expect("a well-formed identifier");
        assert!(matches!(plain.root(), Uid::Uuid(_)));
        assert!(!plain.has_extension());
    }

    /// Case is preserved through construction (master05 §"Composite
    /// Identifiers and Case"), while the case-insensitive comparison still
    /// identifies the two as the same thing.
    #[test]
    fn construction_is_case_preserving() {
        let upper = HierObjectId::new("UK.NHS.EHR1::A42").expect("a well-formed identifier");
        let lower = HierObjectId::new("uk.nhs.ehr1::a42").expect("a well-formed identifier");
        assert_eq!(upper.value, "UK.NHS.EHR1::A42");
        assert!(upper.is_equal(&lower));
    }
}
