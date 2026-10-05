// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written BASE class checks for `PARTY_REF`.
//!
//! - `Type_validity` — a RELEASED invariant, declared by BASE base_types
//!   `UML/classes/org.openehr.base.base_types.party_ref.adoc` §Invariants:
//!   `type` is one of the demographic/party types the assertion enumerates.
//! - `Namespace_valid` — the `OBJECT_REF` namespace rule, inherited. BASE
//!   declares the rule but no invariant name for it, so that label is this
//!   workspace's own convention (see `object_ref_impl`).

use super::object_ref_impl::namespace_valid;
use super::party_ref::PartyRef;
use crate::validate::{InvariantViolation, Validate};

/// The party/demographic `type` values a `PARTY_REF` may point at.
///
/// The named subtypes + the abstract supertypes `PARTY`/`ACTOR` are the closed
/// set of the BASE `Type_validity` invariant
/// (`docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.party_ref.adoc`,
/// §Invariants). The class's own Description sanctions **abstract supertypes**
/// "if the referenced object is of a type not known by the current
/// implementation".
///
/// `ANY` is admitted on the strength of the attribute it constrains:
/// `object_ref.adoc` §Attributes `type` — "The type name `ANY` can be used to
/// indicate that any type is accepted (e.g. if the type is unknown)" — which
/// `PARTY_REF` inherits. An *unknown* type string (e.g. a typo) is still
/// rejected.
// NOTE: `party_ref.adoc` §Invariants enumerates a closed set that omits the
// `ANY` its own inherited `type` attribute sanctions — a released-text tension,
// resolved toward the attribute because the invariant constrains that attribute.
const VALID_PARTY_TYPES: &[&str] = &[
    "PERSON",
    "ORGANISATION",
    "GROUP",
    "AGENT",
    "ROLE",
    "PARTY",
    "ACTOR",
    "ANY",
];

impl Validate for PartyRef {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if !VALID_PARTY_TYPES.contains(&self.r#type.as_str()) {
            out.push(InvariantViolation::here(
                "Invariant Type_validity failed on type PARTY_REF",
            ));
        }
        if !namespace_valid(&self.namespace) {
            out.push(InvariantViolation::here(
                "Invariant Namespace_valid failed on type PARTY_REF",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_3::base_types::identification::hier_object_id::HierObjectId;
    use crate::v1_3::base_types::identification::object_id::ObjectId;

    fn party_ref(namespace: &str, ty: &str) -> PartyRef {
        PartyRef {
            namespace: namespace.to_owned(),
            r#type: ty.to_owned(),
            id: ObjectId::HierObjectId(HierObjectId {
                value: "abc".to_owned(),
            }),
        }
    }

    #[test]
    fn valid_party_ref() {
        assert!(party_ref("local", "PERSON").invariants().is_empty());
    }

    /// `ANY` (the universal supertype) is admitted — the value the CNF positive
    /// commit corpus uses (see `VALID_PARTY_TYPES` doc; CNF wins).
    #[test]
    fn valid_party_ref_any_supertype() {
        assert!(party_ref("local", "ANY").invariants().is_empty());
    }

    #[test]
    fn invalid_type() {
        let v = party_ref("local", "SOMEONE").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Type_validity failed on type PARTY_REF"),
            "got {v:?}"
        );
    }

    #[test]
    fn invalid_namespace() {
        let v = party_ref("A*", "AGENT").invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Namespace_valid failed on type PARTY_REF"),
            "got {v:?}"
        );
    }
}
