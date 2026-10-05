// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written construction door for `UUID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.uuid.adoc`
//! and `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
//! §Syntaxes, which gives the lexical form as
//! `uuid = hex-number, '-', hex-number, '-', hex-number, '-', hex-number, '-',
//! hex-number` — the canonical `8-4-4-4-12` spelling (see
//! [`super::lexical::is_uuid`] for why the widths are load-bearing).
//!
//! The generated `value` field carries the pinned [`uuid`] crate's RFC-4122
//! type, so **parsing is validation**: any `uuid::Uuid` that exists is already a
//! well-formed `UUID`. [`Uuid::new`] is therefore total, and its job is to be
//! the ONLY door — with the field emitted `pub(crate)` no consumer outside this
//! crate can install a value that did not come through the grammar, now or after
//! a later refactor.
//!
//! [`Uuid::from_str`] / [`TryFrom<&str>`] are the fallible half: they take the
//! §Syntaxes *string* form and run the grammar, refusing the braced (`{…}`),
//! URN (`urn:uuid:…`) and simple (no-hyphen) spellings that
//! [`uuid::Uuid::try_parse`] would otherwise accept and silently rewrite.

use std::str::FromStr;

use super::lexical::{IdComponent, IdError, IdProduction, is_uuid};
use super::uuid::Uuid;

impl Uuid {
    /// Build a `UUID` from an already-parsed RFC-4122 value.
    ///
    /// Total by construction: the parameter type admits no malformed value, so
    /// there is nothing left to reject (see the module note on why the door
    /// exists anyway).
    #[must_use]
    pub const fn new(value: uuid::Uuid) -> Self {
        Self { value }
    }
}

impl FromStr for Uuid {
    type Err = IdError;

    /// Parse a `UUID` from its BASE `master05-identification_package.adoc`
    /// §Syntaxes string form (the canonical hyphenated `8-4-4-4-12` spelling).
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value, [`IdError::Malformed`] for
    /// anything that is not a `uuid` production — including the braced, URN and
    /// unhyphenated spellings, which are not `uuid` lexical forms.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        // ONE grammar: `is_uuid` delegates to the same pinned parser this
        // arm uses, so a value it accepts always parses — but the fallback is
        // data, never a panic.
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

    /// The spellings [`uuid::Uuid::try_parse`] accepts but the §Syntaxes `uuid`
    /// production does not are refused at the door, so they can never be
    /// silently rewritten into the canonical form.
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
