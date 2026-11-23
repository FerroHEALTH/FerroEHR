// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Validating construction for `HIER_OBJECT_ID`.
//!
//! BASE 1.3.0 `master05-identification_package.adoc` §Syntaxes:
//! `hier_object_id = uid_based_id`, `uid_based_id = root, [ '::', extension ]`,
//! `root = uid`. The accessors live in
//! [`uid_based_id_impl`](super::uid_based_id_impl).

use std::str::FromStr;

use super::hier_object_id::HierObjectId;
use super::lexical::{IdComponent, IdError, IdProduction, is_uid};
use super::uid::Uid;
use super::uid_based_id_impl::root_str;

impl HierObjectId {
    /// Builds a `HIER_OBJECT_ID` from its string form, stored verbatim (master05
    /// §"Composite Identifiers and Case").
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value, and [`IdError::Malformed`] when
    /// the `root` part is not a legal `uid`. The `extension` is `? any string ?`,
    /// so it is not constrained.
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
    /// A bare `UID` is a `HIER_OBJECT_ID` by grammar (`root = uid`), rendered as
    /// [`Uid::value`].
    fn from(uid: Uid) -> Self {
        Self {
            value: uid.value().into_owned(),
        }
    }
}

impl From<uuid::Uuid> for HierObjectId {
    /// A UUID is a `HIER_OBJECT_ID` by grammar, rendered in the RFC 4122 lower-case
    /// hyphenated form.
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
    use crate::v1_3::base_types::identification::uid::Uid;
    use crate::v1_3::base_types::identification::uid_based_id_impl::extension_str;

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

    /// Construction preserves case; the comparison folds it (master05
    /// §"Composite Identifiers and Case").
    #[test]
    fn construction_is_case_preserving() {
        let upper = HierObjectId::new("UK.NHS.EHR1::A42").expect("a well-formed identifier");
        let lower = HierObjectId::new("uk.nhs.ehr1::a42").expect("a well-formed identifier");
        assert_eq!(upper.value, "UK.NHS.EHR1::A42");
        assert!(upper.is_equal(&lower));
    }
}
