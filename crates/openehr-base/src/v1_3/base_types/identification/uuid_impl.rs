// @generated-from-template templates/openehr-base/base_types/identification/uuid_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The construction door for `UUID`.
//!
//! BASE 1.3.0 master05 §Syntaxes, the `8-4-4-4-12` form. `FromStr` refuses the
//! braced, URN and simple spellings.

use std::str::FromStr;

use super::lexical::{IdComponent, IdError, IdProduction, is_uuid};
use super::uuid::Uuid;

impl Uuid {
    /// Builds a `UUID` from an already-parsed RFC 4122 value.
    #[must_use]
    pub const fn new(value: uuid::Uuid) -> Self {
        Self { value }
    }
}

impl FromStr for Uuid {
    type Err = IdError;

    /// Parses a `UUID` from its §Syntaxes string form.
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value, [`IdError::Malformed`] for anything
    /// else that is not the hyphenated `uuid` production.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        // `is_uuid` uses the same parser, so this fallback never fires.
        if !is_uuid(s) {
            return Err(IdError::Malformed {
                component: IdComponent::Value,
                expected: IdProduction::Uuid,
                found: s.to_owned(),
            });
        }
        match s.parse::<uuid::Uuid>() {
            Ok(value) => Ok(Self::new(value)),
            Err(_unreachable_by_is_uuid) => Err(IdError::Malformed {
                component: IdComponent::Value,
                expected: IdProduction::Uuid,
                found: s.to_owned(),
            }),
        }
    }
}

impl TryFrom<&str> for Uuid {
    type Error = IdError;

    /// Parse a `UUID` — see [`Uuid::from_str`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s)
    }
}

impl From<uuid::Uuid> for Uuid {
    /// The total construction door, as the standard conversion trait
    /// (API guidelines C-CONV-TRAITS) — see [`Uuid::new`].
    fn from(value: uuid::Uuid) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{IdComponent, IdError, IdProduction, Uuid};

    /// The canonical hyphenated form round-trips through the door, verbatim in
    /// the RFC-4122 lower-case rendering the pinned crate produces.
    #[test]
    fn accepts_the_canonical_form() {
        let id: Uuid = "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"
            .parse()
            .expect("a well-formed UUID");
        assert_eq!(
            id.value().to_string(),
            "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"
        );
        // Mixed case parses (hex digits are case-insensitive) and renders lower.
        let upper: Uuid = "87284370-2D4B-4e3d-A3F3-F303D2F4F34B"
            .parse()
            .expect("a well-formed UUID");
        assert_eq!(
            upper.value().to_string(),
            "87284370-2d4b-4e3d-a3f3-f303d2f4f34b"
        );
    }

    /// The spellings `try_parse` accepts beyond the `uuid` production are refused.
    #[test]
    fn refuses_the_non_syntaxes_spellings() {
        for raw in [
            "{87284370-2D4B-4e3d-A3F3-F303D2F4F34B}",
            "urn:uuid:87284370-2d4b-4e3d-a3f3-f303d2f4f34b",
            "872843702d4b4e3da3f3f303d2f4f34b",
            "1-2-3-4-5",
            "not-a-uuid",
        ] {
            assert_eq!(
                raw.parse::<Uuid>(),
                Err(IdError::Malformed {
                    component: IdComponent::Value,
                    expected: IdProduction::Uuid,
                    found: raw.to_owned(),
                }),
                "for {raw:?}"
            );
        }
        assert_eq!("".parse::<Uuid>(), Err(IdError::Empty));
    }

    /// The total door and the conversion trait agree.
    #[test]
    fn total_door_and_conversion_agree() {
        let raw: uuid::Uuid = "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"
            .parse()
            .expect("a well-formed RFC 4122 UUID");
        assert_eq!(Uuid::new(raw), Uuid::from(raw));
        assert_eq!(*Uuid::new(raw).value(), raw);
    }
}
