// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The optional-container convention shared by every crate that builds openEHR
//! model objects.
//!
//! Hand-written; preserved across `openehr-codegen` regeneration (it carries no
//! generated-file marker, so `write_crate` keeps it and `lib.rs` auto-declares
//! `pub mod containers;`).
//!
//! A model attribute whose declared existence is `0..1` and whose type is a
//! container emits as `Option<Vec<T>>`, because absence and present-but-
//! emptiness are two distinct states the models rely on: the
//! `x /= Void implies not x.is_empty` invariant family (e.g.
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
//! §Invariants, `Links_valid`) has nothing to judge unless both are
//! representable.
//!
//! That leaves exactly one decision for every builder that produces such an
//! attribute by COLLECTING parsed or converted members: which of the two states
//! an empty collection means. This module is the single owner of that decision
//! ([`present`]) so it is made once, with its reasoning, rather than re-decided
//! per crate.

/// Wrap a collected member list in the optional-container shape, mapping an
/// empty list to `None`.
///
/// **An empty collected list means the attribute was ABSENT in the source.**
/// Every serialization openEHR defines writes a member list by writing its
/// members, so "no members" and "no attribute" are the same input text and a
/// builder cannot have observed anything else:
///
/// - canonical JSON omits an empty list rather than writing `[]`
///   (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
///   §JSON Format: "The RM attributes (even required ones) that are `Null` or
///   an empty list (array) SHOULD be absent when serialized as JSON");
/// - canonical XML has no representation for it at all — a repeated element
///   with zero occurrences IS absence;
/// - the ODIN/ADL persistence forms attach a member list to a keyword block
///   that is written only when it carries content
///   (`docs/specs/openehr/AM/docs/ADL2/master04-syntax.adoc` §Structure,
///   `docs/specs/openehr/LANG/docs/ODIN/master04-syntax.adoc` §Objects).
///
/// So `Some(vec![])` would assert a state no source syntax produces. The one
/// place present-but-empty legitimately arises is a JSON reader that saw a
/// literal `[]`, and that reader constructs the `Option` directly
/// (`openehr_its::json_codec::runtime::optional_container_field`) rather than
/// going through this function — which is exactly why the two paths are
/// separate.
#[must_use]
pub fn present<T>(members: Vec<T>) -> Option<Vec<T>> {
    (!members.is_empty()).then_some(members)
}

/// [`present`] for an `Option<NonEmptyVec<T>>` field: empty input = absent.
#[must_use]
pub fn present_nonempty<T>(members: Vec<T>) -> Option<NonEmptyVec<T>> {
    NonEmptyVec::new(members).ok()
}

#[cfg(test)]
mod tests {
    use super::present;

    #[test]
    fn an_empty_member_list_is_absence() {
        assert_eq!(present(Vec::<u8>::new()), None);
    }

    #[test]
    fn a_populated_member_list_is_presence() {
        assert_eq!(present(vec![1_u8, 2]), Some(vec![1, 2]));
    }
}

/// A container that is non-empty by construction.
///
/// This is the emission shape of a model attribute whose BMM cardinality has a
/// lower bound of 1 (`CLUSTER.items: List<ITEM> {1..*}`,
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
/// §Attributes).
///
/// A `1..*` bound is a *structural* statement about the model, so it is carried
/// by the type rather than re-checked at every boundary: an empty list is
/// unrepresentable, which is what makes the corresponding validation rule
/// unnecessary rather than merely reliable. The wire is unaffected — a
/// conformant instance always carries at least one member — but a
/// NON-conformant one now fails at the single door into the type
/// ([`NonEmptyVec::new`]), which the canonical-JSON and canonical-XML readers
/// both go through.
///
/// Reads behave exactly like a slice ([`core::ops::Deref`]/[`core::ops::DerefMut`]
/// to `[T]`, plus
/// [`IntoIterator`] in all three forms), so `iter()`, `len()`, `first()`,
/// indexing and `for` loops need no adaptation. Only CONSTRUCTION is
/// restricted, and only length-preserving or length-GROWING mutation is
/// offered ([`NonEmptyVec::push`]) — nothing in the API can empty it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonEmptyVec<T>(Vec<T>);

/// The error [`NonEmptyVec::new`] returns: a container the model declares
/// `1..*` was given no members.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a container with a cardinality lower bound of 1 must have at least one member")]
pub struct EmptyContainer;

impl<T> NonEmptyVec<T> {
    /// Build a non-empty container from `members`.
    ///
    /// # Errors
    /// Returns [`EmptyContainer`] when `members` is empty — the state the
    /// model's `1..*` cardinality forbids.
    pub fn new(members: Vec<T>) -> Result<Self, EmptyContainer> {
        if members.is_empty() {
            return Err(EmptyContainer);
        }
        Ok(Self(members))
    }

    /// Build a non-empty container from a single member, which cannot fail.
    #[must_use]
    pub fn of(member: T) -> Self {
        Self(vec![member])
    }

    /// Append a member. Growing a non-empty container keeps it non-empty, so
    /// this needs no check.
    pub fn push(&mut self, member: T) {
        self.0.push(member);
    }

    /// The members as a plain `Vec`, consuming the container.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    /// The first member. Unlike `[T]::first` this cannot be `None`, because the
    /// container is non-empty by construction.
    #[must_use]
    pub fn head(&self) -> &T {
        // A `NonEmptyVec` is never empty by construction (`new` is the only
        // fallible door and `push` only grows), so index 0 always exists.
        #[expect(
            clippy::indexing_slicing,
            reason = "index 0 is in bounds by the type's construction invariant: `new` rejects an empty Vec and no method can shrink one"
        )]
        &self.0[0]
    }
}

impl<T> TryFrom<Vec<T>> for NonEmptyVec<T> {
    type Error = EmptyContainer;

    fn try_from(members: Vec<T>) -> Result<Self, Self::Error> {
        Self::new(members)
    }
}

impl<T> From<NonEmptyVec<T>> for Vec<T> {
    fn from(container: NonEmptyVec<T>) -> Self {
        container.0
    }
}

impl<T> core::ops::Deref for NonEmptyVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> core::ops::DerefMut for NonEmptyVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

impl<T> AsRef<[T]> for NonEmptyVec<T> {
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T> IntoIterator for NonEmptyVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a NonEmptyVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut NonEmptyVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

/// A `1..*` container writes exactly like the `Vec` it wraps — the bound is a
/// model constraint, not a wire distinction.
impl<T: serde::Serialize> serde::Serialize for NonEmptyVec<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// Reading a `1..*` container goes through [`NonEmptyVec::new`], so a
/// present-but-EMPTY array is refused at PARSE rather than surviving into the
/// model — the structural realization of a BMM cardinality lower bound of 1
/// (e.g. `CLUSTER.items`,
/// `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
/// §Attributes).
impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for NonEmptyVec<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let members = Vec::<T>::deserialize(deserializer)?;
        Self::new(members).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod nonempty_tests {
    use super::{EmptyContainer, NonEmptyVec};

    #[test]
    fn an_empty_member_list_is_refused() {
        assert_eq!(NonEmptyVec::<u8>::new(Vec::new()), Err(EmptyContainer));
    }

    #[test]
    fn a_populated_member_list_is_accepted_and_reads_as_a_slice() {
        let c = NonEmptyVec::new(vec![1_u8, 2]).expect("two members");
        assert_eq!(c.len(), 2);
        assert_eq!(c.head(), &1);
        assert_eq!(c.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn growing_keeps_it_non_empty() {
        let mut c = NonEmptyVec::of(1_u8);
        c.push(2);
        assert_eq!(c.into_vec(), vec![1, 2]);
    }
}
