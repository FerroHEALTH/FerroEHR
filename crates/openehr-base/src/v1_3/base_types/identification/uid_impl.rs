// @generated-from-template templates/openehr-base/base_types/identification/uid_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written accessor for the abstract `UID` class.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.uid.adoc`
//! — `UID` declares exactly one attribute, `value: String` [1..1] ("The value
//! of the id."), and one invariant, `Value_valid: not value.empty`. The
//! generator emits `UID` as the closed subtype enum
//! ([`Uid`]) and puts the `value` field on each concrete
//! subtype, so the inherited attribute has no accessor on the parent; this
//! sibling supplies it.
//!
//! NOTE: `UUID.value` emits as `uuid::Uuid`, so the spec's `String` view is
//! RENDERED (a [`Cow`], owned for `UUID`) in the RFC 4122 lower-case
//! hyphenated form BASE `master05-identification_package.adoc` §Syntaxes
//! gives — a legal but NORMALISED rendering: an upper-case-written `UUID`
//! does not round-trip byte-for-byte, so callers honouring master05
//! §"Composite Identifiers and Case" (verbatim storage) must keep the
//! original string; this accessor reads the value, never re-serialises a
//! stored one.

use std::borrow::Cow;
use std::str::FromStr;

use super::lexical::{IdComponent, IdError, IdProduction, is_uid, make_uid};
use super::uid::Uid;

impl Uid {
    /// Build a `UID` from its string form, validating the
    /// `uid = iso_oid | uuid | internet_id` production of BASE
    /// `master05-identification_package.adoc` §Syntaxes and choosing the
    /// concrete subtype by lexical form (the same dispatch the `UID_BASED_ID`
    /// accessors use — one classification, one grammar).
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value; [`IdError::Malformed`] when the
    /// value matches none of the three `uid` productions.
    pub fn new(value: &str) -> Result<Self, IdError> {
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        if !is_uid(value) {
            return Err(IdError::Malformed {
                component: IdComponent::Value,
                expected: IdProduction::Uid,
                found: value.to_owned(),
            });
        }
        Ok(make_uid(value))
    }
}

impl FromStr for Uid {
    type Err = IdError;

    /// Parse a `UID` — see [`Uid::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for Uid {
    type Error = IdError;

    /// Parse a `UID` — see [`Uid::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Uid {
    /// The `UID.value` string of whichever subtype this is — BASE
    /// `org.openehr.base.base_types.uid.adoc` §Attributes (`value: String`
    /// [1..1], "The value of the id.").
    ///
    /// Borrowed for the two string-backed subtypes (`INTERNET_ID`, `ISO_OID`)
    /// and owned for `UUID`, whose `value` is a typed `uuid::Uuid` rendered in
    /// the RFC 4122 lower-case hyphenated form (see the module note).
    #[must_use]
    pub fn value(&self) -> Cow<'_, str> {
        match self {
            Uid::InternetId(id) => Cow::Borrowed(id.value.as_str()),
            Uid::IsoOid(id) => Cow::Borrowed(id.value.as_str()),
            Uid::Uuid(id) => Cow::Owned(id.value.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_3::base_types::identification::internet_id::InternetId;
    use crate::v1_3::base_types::identification::iso_oid::IsoOid;
    use crate::v1_3::base_types::identification::lexical::make_uid;
    use crate::v1_3::base_types::identification::uuid::Uuid;

    /// Each variant yields its own `value`, and the string-backed ones borrow.
    #[test]
    fn value_per_subtype() {
        let internet = Uid::InternetId(InternetId {
            value: "openehr.org".to_owned(),
        });
        assert_eq!(internet.value(), "openehr.org");
        assert!(matches!(internet.value(), Cow::Borrowed(_)));

        let oid = Uid::IsoOid(IsoOid {
            value: "1.2.840.113554".to_owned(),
        });
        assert_eq!(oid.value(), "1.2.840.113554");
        assert!(matches!(oid.value(), Cow::Borrowed(_)));

        let uuid = Uid::Uuid(Uuid {
            value: "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"
                .parse()
                .expect("the literal is a well-formed RFC 4122 UUID"),
        });
        assert_eq!(uuid.value(), "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11");
        assert!(matches!(uuid.value(), Cow::Owned(_)));
    }

    /// The validating door accepts exactly the three `uid` productions and
    /// classifies each to its subtype; everything else is refused as data.
    #[test]
    fn new_validates_and_classifies() {
        let uid = |raw: &str| Uid::new(raw).expect("a well-formed uid");
        assert!(matches!(
            uid("87284370-2D4B-4e3d-A3F3-F303D2F4F34B"),
            Uid::Uuid(_)
        ));
        assert!(matches!(uid("1.2.840.113554"), Uid::IsoOid(_)));
        assert!(matches!(uid("openEHR.org"), Uid::InternetId(_)));
        assert!(matches!(
            "uk.nhs.ehr1".parse::<Uid>().expect("a well-formed uid"),
            Uid::InternetId(_)
        ));

        assert_eq!(Uid::new(""), Err(IdError::Empty));
        for raw in [
            "1-2-3-4-5",
            "1234-5678",
            "{87284370-2D4B-4e3d-A3F3-F303D2F4F34B}",
            "has space",
        ] {
            assert_eq!(
                Uid::new(raw),
                Err(IdError::Malformed {
                    component: IdComponent::Value,
                    expected: IdProduction::Uid,
                    found: raw.to_owned(),
                }),
                "for {raw:?}"
            );
        }
    }

    /// Postcondition of `UID.Value_valid` (`not value.empty`) on every id the
    /// lexical builder produces from a non-empty string.
    #[test]
    fn value_is_never_empty_for_a_non_empty_source() {
        for raw in [
            "openehr.org",
            "1.2.840.113554.3.7.10",
            "87284370-2D4B-4e3d-A3F3-F303D2F4F34B",
            "12345",
        ] {
            assert!(!make_uid(raw).value().is_empty(), "empty value for {raw:?}");
        }
    }

    /// `value()` round-trips the two string-backed lexical forms verbatim; the
    /// `UUID` form is NORMALISED to lower case (see the module note), which is
    /// why it is not a byte-for-byte round trip.
    #[test]
    fn round_trip_is_verbatim_except_for_the_normalised_uuid_rendering() {
        assert_eq!(make_uid("openEHR.org").value(), "openEHR.org");
        assert_eq!(make_uid("1.2.840.113554").value(), "1.2.840.113554");
        assert_eq!(
            make_uid("87284370-2D4B-4e3d-A3F3-F303D2F4F34B").value(),
            "87284370-2d4b-4e3d-a3f3-f303d2f4f34b"
        );
    }
}
