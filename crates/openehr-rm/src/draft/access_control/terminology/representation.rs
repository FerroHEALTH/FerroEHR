// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The groups of the draft access terminology a representation is coded in:
//! the relationship's `name` and the elements of its `details` (draft access
//! control package, "A representation is").

use std::fmt;

use super::{Group, TERMINOLOGY_ID_ACCESS, Term};

/// HL7 Terminology `v3-RoleCode` by `HL7_` and its code system name: the
/// code system of an unbound representation `basis`.
pub const TERMINOLOGY_ID_ROLE_CODE: &str = "HL7_RoleCode";

group! {
    /// The demographic relationship of representation, coded in the `name`
    /// of a `PARTY_RELATIONSHIP` of its own.
    Representation in Representation {
        /// The relationship's source represents its target, the subject, on
        /// the subject's own record; its `basis` element says why.
        Represents => "represents",
    }
}

group! {
    /// The elements in the `details` of a `represents` relationship, coded
    /// in each `ELEMENT`'s `name`.
    RepresentationElement in RepresentationElement {
        /// Why the source may act for the subject, of the value set bound
        /// for `representation_basis`.
        Basis => "basis",
        /// The healthcare provider for which the representation is recorded.
        Provider => "provider",
        /// The court order, register entry or written authorisation.
        Evidence => "evidence",
    }
}
