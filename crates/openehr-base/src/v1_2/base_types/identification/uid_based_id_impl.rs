// @generated-from-template templates/openehr-base/base_types/identification/uid_based_id_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written accessor functions for the `UID_BASED_ID` family
//! (`HIER_OBJECT_ID`, `OBJECT_VERSION_ID`) and the abstract enum.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.uid_based_id.adoc`.
//! Lexical form: `root '::' extension`.
//! - `root()`  — the part left of the first `::`, or the whole string.
//! - `extension()` — the part right of the first `::`, or the empty string.
//! - `has_extension()` — `not extension.is_empty()`.
//!
//! Invariant `Has_extension_valid` (`extension.is_empty xor has_extension`) is
//! unfalsifiable in this representation: the same page defines
//! `has_extension(): True if not extension.is_empty()`, so computing it from
//! `extension()` makes the invariant a tautology with nothing to check at
//! runtime. It is therefore not surfaced as a runnable `Validate` check.
//! `OBJECT_VERSION_ID` well-formedness is checked by its own sibling
//! (`object_version_id_impl`).

use super::hier_object_id::HierObjectId;
use super::lexical::{composite_ids_equal, make_uid};
use super::object_version_id::ObjectVersionId;
use super::uid::Uid;
use super::uid_based_id::UidBasedId;

/// The `root` substring of a `UID_BASED_ID` value: everything before the first
/// `::`, or the whole value if there is none.
#[must_use]
pub(crate) fn root_str(value: &str) -> &str {
    value.split_once("::").map_or(value, |(r, _)| r)
}

/// The `extension` substring of a `UID_BASED_ID` value: everything after the
/// first `::`, or the empty string.
#[must_use]
pub(crate) fn extension_str(value: &str) -> &str {
    value.split_once("::").map_or("", |(_, e)| e)
}

/// Generate the `UID_BASED_ID` accessor trio (`root`, `extension`,
/// `has_extension`) for a concrete `{ value: String }` type.
macro_rules! uid_based_id_accessors {
    ($ty:ty) => {
        impl $ty {
            /// The conceptual-namespace identifier: the part of `value` left of
            /// the first `::` separator, as a typed [`Uid`] (BASE
            /// `UID_BASED_ID.root`).
            #[must_use]
            pub fn root(&self) -> Uid {
                make_uid(root_str(&self.value))
            }

            /// The optional local identifier within the root namespace: the part
            /// right of the first `::`, or the empty string (BASE
            /// `UID_BASED_ID.extension`).
            #[must_use]
            pub fn extension(&self) -> &str {
                extension_str(&self.value)
            }

            /// `true` if [`extension`](Self::extension) is non-empty (BASE
            /// `UID_BASED_ID.has_extension`).
            #[must_use]
            pub fn has_extension(&self) -> bool {
                !self.extension().is_empty()
            }

            /// Case-**insensitive** identity: `true` iff this identifier and
            /// `other` are equal apart from letter case — the shared
            /// composite-identifier rule
            /// ([`composite_ids_equal`](super::lexical::composite_ids_equal),
            /// BASE `master05-identification_package.adoc` §"Composite
            /// Identifiers and Case").
            ///
            /// The stored `value` is left byte-for-byte intact (the sibling
            /// case-**preserving** rule); only the *comparison* folds case, so a
            /// UUID `object_id` differing only in hex case (`…4E3D…` vs
            /// `…4e3d…`) is recognised as the same version.
            #[must_use]
            pub fn is_equal(&self, other: &Self) -> bool {
                composite_ids_equal(&self.value, &other.value)
            }
        }
    };
}

uid_based_id_accessors!(HierObjectId);
uid_based_id_accessors!(ObjectVersionId);

impl UidBasedId {
    /// The underlying `value` string of whichever concrete variant this is.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::HierObjectId(h) => &h.value,
            Self::ObjectVersionId(o) => &o.value,
        }
    }

    /// The conceptual-namespace identifier (BASE `UID_BASED_ID.root`).
    #[must_use]
    pub fn root(&self) -> Uid {
        make_uid(root_str(self.value()))
    }

    /// The optional local identifier within the root namespace (BASE
    /// `UID_BASED_ID.extension`).
    #[must_use]
    pub fn extension(&self) -> &str {
        extension_str(self.value())
    }

    /// `true` if [`extension`](Self::extension) is non-empty (BASE
    /// `UID_BASED_ID.has_extension`).
    #[must_use]
    pub fn has_extension(&self) -> bool {
        !self.extension().is_empty()
    }

    /// Case-**insensitive** identity across the `UID_BASED_ID` value, regardless
    /// of which concrete variant either side is — the shared
    /// composite-identifier rule
    /// ([`composite_ids_equal`], BASE
    /// `master05-identification_package.adoc` §"Composite Identifiers and Case").
    #[must_use]
    pub fn is_equal(&self, other: &Self) -> bool {
        composite_ids_equal(self.value(), other.value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hier_object_id_root_extension() {
        let h = HierObjectId {
            value: "1.2.840::extension-part".to_owned(),
        };
        assert!(matches!(h.root(), Uid::IsoOid(_)));
        assert_eq!(h.extension(), "extension-part");
        assert!(h.has_extension());
    }

    #[test]
    fn hier_object_id_no_extension() {
        let h = HierObjectId {
            value: "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11".to_owned(),
        };
        assert!(matches!(h.root(), Uid::Uuid(_)));
        assert_eq!(h.extension(), "");
        assert!(!h.has_extension());
    }

    #[test]
    fn object_version_id_root_is_first_of_three() {
        // root is left of the FIRST '::'; extension is the remainder.
        let o = ObjectVersionId {
            value: "87284370-2D4B-4e3d-A3F3-F303D2F4F34B::openEHR.org::2".to_owned(),
        };
        assert_eq!(o.extension(), "openEHR.org::2");
        assert!(o.has_extension());
    }

    #[test]
    fn enum_delegates() {
        let id = UidBasedId::HierObjectId(HierObjectId {
            value: "abc::def".to_owned(),
        });
        assert_eq!(id.value(), "abc::def");
        assert_eq!(id.extension(), "def");
        assert!(id.has_extension());
    }

    /// BASE `master05` §"Composite Identifiers and Case": two identifiers
    /// identical apart from case identify the same thing.
    #[test]
    fn is_equal_is_case_insensitive() {
        // OBJECT_VERSION_ID differing only in UUID hex case → equal.
        let a = ObjectVersionId {
            value: "87284370-2D4B-4E3D-A3F3-F303D2F4F34B::uk.nhs.ehr1::2".to_owned(),
        };
        let b = ObjectVersionId {
            value: "87284370-2d4b-4e3d-a3f3-f303d2f4f34b::UK.NHS.EHR1::2".to_owned(),
        };
        assert!(a.is_equal(&b));
        assert!(b.is_equal(&a));
        // Value stays byte-for-byte intact (case-preserving); only compare folds.
        assert_ne!(a.value, b.value);

        // A genuine difference (version tree id) is still not equal.
        let c = ObjectVersionId {
            value: "87284370-2d4b-4e3d-a3f3-f303d2f4f34b::uk.nhs.ehr1::3".to_owned(),
        };
        assert!(!a.is_equal(&c));

        // HIER_OBJECT_ID and the abstract enum fold case the same way.
        let h1 = HierObjectId {
            value: "2FDBF3F0-1C0A-4A0E-9F2A-3B7F6B1E9C11".to_owned(),
        };
        let h2 = HierObjectId {
            value: "2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11".to_owned(),
        };
        assert!(h1.is_equal(&h2));
        assert!(UidBasedId::HierObjectId(h1).is_equal(&UidBasedId::HierObjectId(h2)));
    }
}
