// @generated-from-template templates/openehr-base/base_types/identification/iso_oid_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The inherited `UID.Value_valid` invariant and validating construction for
//! `ISO_OID` (`iso_oid = number, { '.', number }`, BASE
//! `master05-identification_package.adoc` §Syntaxes).

use std::str::FromStr;

use super::iso_oid::IsoOid;
use super::lexical::{IdComponent, IdError, IdProduction, is_iso_oid};
use crate::validate::{InvariantViolation, Validate};

impl IsoOid {
    /// Builds an `ISO_OID` from its string form.
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
        assert_eq!(
            IsoOid {
                value: "1.2.840".to_owned()
            }
            .invariants(),
            Vec::<InvariantViolation>::new()
        );
        let v = IsoOid {
            value: String::new(),
        }
        .invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].message, "Invariant Value_valid failed on type ISO_OID");
    }
}
