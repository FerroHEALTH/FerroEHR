// @generated-from-template templates/openehr-rm/common/tags/item_tag_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Validating construction and invariants for `ITEM_TAG` (RM 1.2.0
//! `org.openehr.rm.common.item_tag.adoc`): `Inv_key_valid` and
//! `Inv_value_valid`, enforced at [`ItemTag::new`], which every reader uses.
//! Whether `target` exists stays a service-layer check.

use crate::v1_2::common::tags::item_tag::ItemTag;
use openehr_base::v1_3::prelude::{ObjectRef, UidBasedId};
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
    /// Builds an `ITEM_TAG`, checking its two §Invariants.
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
            key,
            value,
            target,
            target_path,
            owner_id,
        })
    }
}

impl ItemTag {
    /// `Inv_key_valid` on a candidate key: `not key.is_empty and
    /// key.is_justified`.
    ///
    /// NOTE: no BASE section defines `is_justified`; `item_tag.adoc` says "May not
    /// be empty or contain leading or trailing whitespace".
    #[must_use]
    pub fn key_valid(key: &str) -> bool {
        !key.is_empty() && key.trim() == key
    }

    /// `Inv_value_valid` on a candidate value.
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
    use openehr_base::v1_3::prelude::{
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

    /// The door refuses what `Inv_key_valid` forbids.
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
        assert_eq!(
            tag("problem-list", None).invariants(),
            Vec::<InvariantViolation>::new()
        );
        assert_eq!(
            tag("severity", Some("high")).invariants(),
            Vec::<InvariantViolation>::new()
        );
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
        // The predicates give the verdict `Validate` gives on an instance.
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
