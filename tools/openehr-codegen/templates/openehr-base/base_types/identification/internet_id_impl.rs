// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM/BASE class invariant + validating construction for
//! `INTERNET_ID`.
//!
//! Inherited `UID.Value_valid` (`not value.empty`) — BASE
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.uid.adoc`
//! §Invariants — surfaced under the concrete type name.
//! Lexical form: `internet_id = subdomain` (BASE `base_types`
//! `master05-identification_package.adoc` §Syntaxes).

use std::str::FromStr;

use super::internet_id::InternetId;
use super::lexical::{IdComponent, IdError, IdProduction, is_internet_id};
use crate::validate::{InvariantViolation, Validate};

impl InternetId {
    /// Build an `INTERNET_ID` from its string form, validating the
    /// `internet_id = subdomain` production of BASE
    /// `master05-identification_package.adoc` §Syntaxes. The value is stored
    /// verbatim (the case-preserving rule of §"Composite Identifiers and
    /// Case").
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value; [`IdError::Malformed`] when the
    /// value is not a legal `internet_id`.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        if !is_internet_id(&value) {
            return Err(IdError::Malformed {
                component: IdComponent::Value,
                expected: IdProduction::InternetId,
                found: value,
            });
        }
        Ok(Self { value })
    }
}

impl FromStr for InternetId {
    type Err = IdError;

    /// Parse an `INTERNET_ID` — see [`InternetId::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for InternetId {
    type Error = IdError;

    /// Parse an `INTERNET_ID` — see [`InternetId::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Validate for InternetId {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.value.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant Value_valid failed on type INTERNET_ID",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction runs the `internet_id` production; the value is verbatim.
    #[test]
    fn new_validates_the_subdomain_production() {
        for raw in [
            "openEHR.org",
            "uk.nhs.ehr1",
            "a",
            "7",
            "my_system-1.example",
        ] {
            assert_eq!(
                InternetId::new(raw)
                    .expect("a well-formed internet id")
                    .value,
                raw
            );
        }
        assert_eq!(InternetId::new(""), Err(IdError::Empty));
        for raw in ["1234-5678", "-leading", "trailing-", "has space", "a..b"] {
            assert_eq!(
                InternetId::new(raw),
                Err(IdError::Malformed {
                    component: IdComponent::Value,
                    expected: IdProduction::InternetId,
                    found: raw.to_owned(),
                }),
                "for {raw:?}"
            );
        }
        assert_eq!(
            "openehr.org"
                .parse::<InternetId>()
                .expect("a well-formed internet id")
                .value,
            "openehr.org"
        );
    }

    #[test]
    fn value_valid() {
        assert!(
            InternetId {
                value: "org.openehr".to_owned()
            }
            .invariants()
            .is_empty()
        );
        assert_eq!(
            InternetId {
                value: String::new()
            }
            .invariants()[0]
                .message,
            "Invariant Value_valid failed on type INTERNET_ID"
        );
    }
}
