// @generated-from-template templates/openehr-base/base_types/identification/party_ref_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `PARTY_REF` checks: the released `Type_validity` invariant (BASE
//! `org.openehr.base.base_types.party_ref.adoc` §Invariants) and the inherited
//! `Namespace_valid` rule.

use super::object_ref_impl::namespace_valid;
use super::party_ref::PartyRef;
use crate::validate::{InvariantViolation, Validate};

/// The `type` values a `PARTY_REF` may point at: the `Type_validity` set
/// (`party_ref.adoc` §Invariants) plus `ANY`.
// NOTE: `object_ref.adoc` §Attributes sanctions `ANY` on the inherited `type`,
// which the invariant's closed set omits; the attribute governs.
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
        assert_eq!(
            party_ref("local", "PERSON").invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    /// `ANY` (the universal supertype) is admitted — the value the CNF positive
    /// commit corpus uses (see `VALID_PARTY_TYPES` doc; CNF wins).
    #[test]
    fn valid_party_ref_any_supertype() {
        assert_eq!(
            party_ref("local", "ANY").invariants(),
            Vec::<InvariantViolation>::new()
        );
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
