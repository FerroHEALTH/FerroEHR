// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The groups of the draft access terminology for care.
//!
//! A care relationship is coded in the elements of its `details`, who wrote
//! it and how it reaches a caller; a composition is placed in an episode of
//! care by the element of its `context.other_context`.

use std::fmt;

use super::{Group, TERMINOLOGY_ID_ACCESS, Term};

group! {
    /// The elements in the `details` of an `under_care_of` relationship,
    /// coded in each `ELEMENT`'s `name`.
    CareElement in CareRelationshipElement {
        /// The part the target plays in the care of the subject, of the value
        /// set bound for `care_role`.
        CareRole => "care_role",
        /// The healthcare provider in whose care the relationship is.
        Provider => "provider",
        /// The carer the target replaces for the period of the relationship.
        CoversFor => "covers_for",
        /// The episode of care the relationship is scoped to.
        Episode => "episode",
        /// Who wrote the relationship, of the `care basis` group.
        Basis => "basis",
    }
}

group! {
    /// Who wrote a care relationship, the value of its `basis` element.
    CareBasis in CareBasis {
        /// The care delivery environment wrote it; the default when absent.
        Assigned => "assigned",
        /// The professional it names wrote it.
        SelfDeclared => "self_declared",
    }
}

group! {
    /// How a care relationship reaches the caller
    /// (`ACCESS_ACTOR_SCOPE.care_path`).
    CarePath in AccessCarePath {
        /// The relationship's target is the caller's `PERSON` or a `ROLE` it
        /// performs, and its basis is `assigned`.
        Direct => "direct",
        /// The relationship's target is a `GROUP` or an `ORGANISATION` unit
        /// the caller is a member of, one level of nesting at most.
        Team => "team",
        /// No stored relationship: the caller's verified credential attests a
        /// treatment context under an agreement the policy names.
        Attested => "attested",
        /// The relationship's target is the caller's `PERSON` and its basis
        /// is `self_declared`.
        SelfDeclared => "self_declared",
    }
}

group! {
    /// The element of a composition's `context.other_context` that places
    /// it in an episode of care.
    PlacementElement in RecordPlacementElement {
        /// The episode the composition is placed in, a `DV_IDENTIFIER`.
        Episode => "episode",
    }
}
