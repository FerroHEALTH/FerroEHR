// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The groups of the draft access terminology a kinship is coded in: the
//! relationship's `name` and the element of its `details` (draft access
//! control package, "A kinship completes the core").

use std::fmt;

use super::{Group, TERMINOLOGY_ID_ACCESS, Term};

group! {
    /// The demographic relationship of kinship, coded in the `name` or
    /// `details` of a `PARTY_RELATIONSHIP` of its own.
    Kinship in Kinship {
        /// The relationship's source and target, two persons, are kin; the
        /// `kinship` element in its `details` says how. Kinship never
        /// grants access: it can refuse it as a conflict of interest.
        RelatedTo => "related_to",
    }
}

group! {
    /// The element in the `details` of a `related_to` relationship, coded in
    /// the `ELEMENT`'s `name`.
    KinshipElement in KinshipElement {
        /// The kinship, of the value set bound for `kinship`.
        Kinship => "kinship",
    }
}
