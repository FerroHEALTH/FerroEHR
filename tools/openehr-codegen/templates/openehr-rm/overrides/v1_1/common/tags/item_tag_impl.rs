// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written validating construction + RM class invariants for `ITEM_TAG`.
//!
//! Spec: RM 1.1.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc`:
//! - `Inv_key_valid`: `not key.is_empty and key.is_justified` (no leading or
//!   trailing whitespace).
//! - `Inv_value_valid`: `value /= Void implies not value.is_empty`.
//!
//! Both invariants are stated over `ITEM_TAG`'s OWN fields and are decidable
//! from them alone, so they are enforced at the construction door: the generated
//! struct's fields are `pub(crate)` (the emitter's construction-door scheme,
//! `plan::construction`), and [`ItemTag::new`] is the only way to obtain the
//! type outside `openehr-rm`. The canonical-JSON and canonical-XML readers build
//! through the same door, so a violating payload refuses at PARSE, path-named,
//! in every document position — an `ITEM_TAG` can no longer exist in violation
//! of its own invariants.
//!
//! What stays OUTSIDE the door: whether `target` names an existing versioned
//! object and whether `target_path` resolves inside it. Those read state the
//! instance does not carry, so they remain service-layer checks.

// NOTE: per-generation override — RM 1.1.0's BMM declares ITEM_TAG's fields
// in a different order, the construction door binds them in struct order, so
// this generation's constructor literal cannot be stamped from the template.
use crate::v1_1::common::tags::item_tag::ItemTag;
use openehr_base::v1_2::prelude::{ObjectRef, UidBasedId};
use openehr_base::validate::{InvariantViolation, Validate};

/// Why an [`ItemTag`] could not be constructed — one variant per released
/// invariant, so a caller branches on the failure instead of matching prose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ItemTagError {
    /// `Inv_key_valid` failed: the key was empty, or carried leading/trailing
    /// whitespace (RM `org.openehr.rm.common.item_tag.adoc` §Invariants).
    #[error("Inv_key_valid: key {0:?} must be non-empty and free of leading/trailing whitespace")]
    KeyInvalid(String),
    /// `Inv_value_valid` failed: the value was present and empty (RM
    /// `org.openehr.rm.common.item_tag.adoc` §Invariants).
    #[error("Inv_value_valid: a present value must be non-empty")]
    ValueEmpty,
}

impl ItemTag {
    /// Build an `ITEM_TAG`, checking the two released §Invariants over its own
    /// fields.
    ///
    /// This is THE door: the generated fields are `pub(crate)`, so outside
    /// `openehr-rm` no other construction exists, and the generated codecs route
    /// through here.
    ///
    /// # Errors
    /// [`ItemTagError::KeyInvalid`] when `key` violates `Inv_key_valid`, and
    /// [`ItemTagError::ValueEmpty`] when `value` violates `Inv_value_valid`.
    pub fn new(
        key: String,
        value: Option<String>,
        target: UidBasedId,
        target_path: Option<String>,
        owner_id: ObjectRef,
    ) -> Result<Self, ItemTagError> {
        if !Self::key_valid(&key) {
            return Err(ItemTagError::KeyInvalid(key));
        }
        if !Self::value_valid(value.as_deref()) {
            return Err(ItemTagError::ValueEmpty);
        }
        Ok(Self {
            target,
            target_path,
            key,
            value,
            owner_id,
        })
    }
}

impl ItemTag {
    /// `Inv_key_valid` as a predicate on a candidate key: `not key.is_empty and
    /// key.is_justified`.
    ///
    /// This is the ONE implementation of the invariant. [`Validate`] evaluates
    /// it on a whole instance, and a caller that must judge a key *before* an
    /// `ITEM_TAG` can be constructed — a wire payload whose `target`/`owner_id`
    /// the server has not yet assigned — evaluates the same function rather
    /// than restating the rule.
    ///
    /// NOTE: `is_justified` is defined in no released BASE section (`String`
    /// declares no such function), so the Meaning column of
    /// `org.openehr.rm.common.item_tag.adoc` is the only text that says what it
    /// means: "May not be empty or contain leading or trailing whitespace".
    /// That sentence is what this predicate implements.
    #[must_use]
    pub fn key_valid(key: &str) -> bool {
        !key.is_empty() && key.trim() == key
    }

    /// `Inv_value_valid` as a predicate on a candidate value:
    /// `value /= Void implies not value.is_empty`. The ONE implementation, for
    /// the same reason as [`ItemTag::key_valid`].
    #[must_use]
    pub fn value_valid(value: Option<&str>) -> bool {
        value != Some("")
    }
}

impl Validate for ItemTag {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if !ItemTag::key_valid(&self.key) {
            out.push(InvariantViolation::here(
                "Invariant Inv_key_valid failed on type ITEM_TAG",
            ));
        }
        if !ItemTag::value_valid(self.value.as_deref()) {
            out.push(InvariantViolation::here(
                "Invariant Inv_value_valid failed on type ITEM_TAG",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_2::prelude::{
        HierObjectId, ObjectId, ObjectRef, ObjectRefData, UidBasedId,
    };

    fn tag(key: &str, value: Option<&str>) -> ItemTag {
        ItemTag {
            key: key.to_owned(),
            value: value.map(str::to_owned),
            target: UidBasedId::HierObjectId(
                HierObjectId::new("87284370-2D4B-4e3d-A3F3-F303D2F4F34B".to_owned())
                    .expect("a well-formed identifier"),
            ),
            target_path: None,
            owner_id: ObjectRef::ObjectRef(ObjectRefData {
                namespace: "local".to_owned(),
                r#type: "EHR".to_owned(),
                id: ObjectId::HierObjectId(
                    HierObjectId::new("b5e19c3a-16c8-4d3e-8b1a-1b1c9dd07f11".to_owned())
                        .expect("a well-formed identifier"),
                ),
            }),
        }
    }

    /// The door REFUSES what `Inv_key_valid` forbids, so an `ITEM_TAG` in
    /// violation of it cannot be constructed at all — the structural half of
    /// the `key_must_be_non_empty_and_justified` validation test below.
    #[test]
    fn the_door_refuses_an_invalid_key() {
        let t = tag("ok", None);
        for bad in ["", " padded", "padded ", " both "] {
            assert_eq!(
                ItemTag::new(
                    bad.to_owned(),
                    None,
                    t.target.clone(),
                    None,
                    t.owner_id.clone(),
                ),
                Err(ItemTagError::KeyInvalid(bad.to_owned())),
                "key {bad:?} must be refused at construction"
            );
        }
    }

    /// The door REFUSES what `Inv_value_valid` forbids.
    #[test]
    fn the_door_refuses_a_present_empty_value() {
        let t = tag("ok", None);
        assert_eq!(
            ItemTag::new(
                "k".to_owned(),
                Some(String::new()),
                t.target.clone(),
                None,
                t.owner_id.clone(),
            ),
            Err(ItemTagError::ValueEmpty),
        );
    }

    /// A conforming tag passes the door and reads back exactly what went in.
    #[test]
    fn the_door_admits_a_conforming_tag() {
        let t = tag("ok", None);
        let built = ItemTag::new(
            "severity".to_owned(),
            Some("high".to_owned()),
            t.target.clone(),
            Some("/content[0]".to_owned()),
            t.owner_id.clone(),
        )
        .expect("a conforming ITEM_TAG should construct");
        assert_eq!(built.key(), "severity");
        assert_eq!(built.value(), Some("high"));
        assert_eq!(built.target_path(), Some("/content[0]"));
        assert_eq!(built.target(), &t.target);
        assert_eq!(built.owner_id(), &t.owner_id);
    }

    #[test]
    fn valid_tag() {
        assert!(tag("problem-list", None).invariants().is_empty());
        assert!(tag("severity", Some("high")).invariants().is_empty());
    }

    #[test]
    fn key_must_be_non_empty_and_justified() {
        for bad in ["", " padded", "padded ", " both "] {
            let v = tag(bad, None).invariants();
            assert!(
                v.iter()
                    .any(|m| m.message == "Invariant Inv_key_valid failed on type ITEM_TAG"),
                "{bad:?} should fail, got {v:?}"
            );
        }
    }

    #[test]
    fn set_value_must_be_non_empty() {
        let v = tag("k", Some("")).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Inv_value_valid failed on type ITEM_TAG"),
            "got {v:?}"
        );
    }

    #[test]
    fn the_predicates_and_the_instance_check_agree() {
        // The predicates ARE the invariant bodies, so a caller that can only
        // reach the predicates (a wire payload with no target/owner_id yet)
        // gets exactly the verdict `Validate` would give on the instance.
        for key in ["", " padded", "padded ", " both ", "ok"] {
            assert_eq!(
                ItemTag::key_valid(key),
                !tag(key, None)
                    .invariants()
                    .iter()
                    .any(|m| m.message == "Invariant Inv_key_valid failed on type ITEM_TAG"),
                "key {key:?}"
            );
        }
        for value in [None, Some(""), Some("v")] {
            assert_eq!(
                ItemTag::value_valid(value),
                !tag("k", value)
                    .invariants()
                    .iter()
                    .any(|m| m.message == "Invariant Inv_value_valid failed on type ITEM_TAG"),
                "value {value:?}"
            );
        }
    }
}
