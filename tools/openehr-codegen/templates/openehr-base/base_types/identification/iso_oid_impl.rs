// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM/BASE class invariant + validating construction for
//! `ISO_OID`.
//!
//! Inherited `UID.Value_valid` (`not value.empty`) — BASE
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.uid.adoc`
//! §Invariants — surfaced under the concrete type name.
//! Lexical form: `iso_oid = number, { '.', number }` (BASE `base_types`
//! `master05-identification_package.adoc` §Syntaxes).

use std::str::FromStr;

use super::iso_oid::IsoOid;
use super::lexical::{IdComponent, IdError, IdProduction, is_iso_oid};
use crate::validate::{InvariantViolation, Validate};

impl IsoOid {
    /// Build an `ISO_OID` from its string form, validating the
    /// `iso_oid = number, { '.', number }` production of BASE
    /// `master05-identification_package.adoc` §Syntaxes.
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value; [`IdError::Malformed`] when the
    /// value is not a legal `iso_oid`.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        if !is_iso_oid(&value) {
            return Err(IdError::Malformed {
                component: IdComponent::Value,
                expected: IdProduction::IsoOid,
                found: value,
            });
        }
        Ok(Self { value })
    }
}

impl FromStr for IsoOid {
    type Err = IdError;

    /// Parse an `ISO_OID` — see [`IsoOid::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for IsoOid {
    type Error = IdError;

    /// Parse an `ISO_OID` — see [`IsoOid::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Validate for IsoOid {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.value.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant Value_valid failed on type ISO_OID",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction runs the `iso_oid` production (one or more digit groups).
    #[test]
    fn new_validates_the_number_groups() {
        for raw in ["1.2.840.113554", "12345", "0.0"] {
            assert_eq!(IsoOid::new(raw).expect("a well-formed OID").value, raw);
        }
        assert_eq!(IsoOid::new(""), Err(IdError::Empty));
        for raw in ["1.", ".1", "1.2a", "1-2"] {
            assert_eq!(
                IsoOid::new(raw),
                Err(IdError::Malformed {
                    component: IdComponent::Value,
                    expected: IdProduction::IsoOid,
                    found: raw.to_owned(),
                }),
                "for {raw:?}"
            );
        }
        assert_eq!(
            "1.2.840"
                .parse::<IsoOid>()
                .expect("a well-formed OID")
                .value,
            "1.2.840"
        );
    }

    #[test]
    fn value_valid() {
        assert!(
            IsoOid {
                value: "1.2.840".to_owned()
            }
            .invariants()
            .is_empty()
        );
        let v = IsoOid {
            value: String::new(),
        }
        .invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].message, "Invariant Value_valid failed on type ISO_OID");
    }
}
