// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written accessor functions + lexical invariant for
//! `OBJECT_VERSION_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.object_version_id.adoc`.
//! Lexical form: `object_id '::' creating_system_id '::' version_tree_id`
//! (exactly three `::`-delimited parts).
//! - `object_id()` — the logical object's UID (part 1).
//! - `creating_system_id()` — the UID of the creating system (part 2), with
//!   `creating_system_id_str()` as its verbatim (case-preserving) sibling.
//! - `version_tree_id()` — the `VERSION_TREE_ID` (part 3).
//! - `is_branch()` — `version_tree_id.is_branch`.
//!
//! The BMM lists no invariant for `OBJECT_VERSION_ID`, but the `::`-delimited
//! well-formedness is required for round-trip (the ITS-REST contract puts this
//! id in `ETag`/`Location`/version paths); we express it as `Value_format_valid`
//! in the same spirit as the ISO-8601 `Value_valid` invariants elsewhere.

use super::lexical::{IdComponent, IdError, IdProduction, is_uid, make_uid};
use super::object_version_id::ObjectVersionId;
use super::uid::Uid;
use super::version_tree_id::VersionTreeId;
use super::version_tree_id_impl::is_valid_version_tree;
use crate::validate::{InvariantViolation, Validate};
use std::str::FromStr;

/// The three `::`-delimited components of an `OBJECT_VERSION_ID` value, or
/// `None` if it does not have exactly three parts.
fn split3(value: &str) -> Option<[&str; 3]> {
    let mut it = value.split("::");
    let (a, b, c, rest) = (it.next(), it.next(), it.next(), it.next());
    match (a, b, c, rest) {
        (Some(a), Some(b), Some(c), None) => Some([a, b, c]),
        _ => None,
    }
}

impl ObjectVersionId {
    /// Unique identifier of the logical object of which this identifies one
    /// version — the first `::`-delimited part (BASE `object_id`). Falls back to
    /// the whole value for a malformed id so the accessor is total.
    #[must_use]
    pub fn object_id(&self) -> Uid {
        let s = split3(&self.value).map_or(self.value.as_str(), |p| p[0]);
        make_uid(s)
    }

    /// Identifier of the system that created this version — the second
    /// `::`-delimited part (BASE `creating_system_id`).
    #[must_use]
    pub fn creating_system_id(&self) -> Uid {
        make_uid(self.creating_system_id_str())
    }

    /// The `creating_system_id` part **verbatim**: the exact bytes of the
    /// second `::`-delimited component, borrowed from `value` (empty for a
    /// malformed id, so the accessor is total like its siblings).
    ///
    /// [`creating_system_id`](Self::creating_system_id) types the same part as a
    /// [`Uid`], which *normalises* a `UUID`-shaped system id to the RFC 4122
    /// lower-case rendering (see the `uid_impl` module note). A caller bound by
    /// the case-**PRESERVING** half of BASE
    /// `master05-identification_package.adoc` §"Composite Identifiers and Case"
    /// ("not change case due to persistence, copying, transfer or other
    /// computation processes") — anything that stores, re-serialises or
    /// round-trips the originating system id — must read it here, not through
    /// the typed accessor.
    #[must_use]
    pub fn creating_system_id_str(&self) -> &str {
        split3(&self.value).map(|p| p[1]).unwrap_or_default()
    }

    /// Tree identifier of this version relative to others in the same version
    /// tree — the third `::`-delimited part (BASE `version_tree_id`).
    #[must_use]
    pub fn version_tree_id(&self) -> VersionTreeId {
        let s = split3(&self.value).map(|p| p[2]).unwrap_or_default();
        VersionTreeId {
            value: s.to_owned(),
        }
    }

    /// `true` if this version identifier represents a branch (BASE `is_branch`).
    #[must_use]
    pub fn is_branch(&self) -> bool {
        self.version_tree_id().is_branch()
    }
}

impl ObjectVersionId {
    /// Build an `OBJECT_VERSION_ID` from its string form, validating the BASE
    /// `master05-identification_package.adoc` §Syntaxes grammar:
    /// `object_version_id = object_id, '::', creating_system_id, '::',
    /// version_tree_id` with `object_id = uid`, `creating_system_id = uid` and
    /// `version_tree_id = trunk_version, [ '.', branch_number, '.',
    /// branch_version ]`.
    ///
    /// The value is stored verbatim — the case-**preserving** half of master05
    /// §"Composite Identifiers and Case".
    ///
    /// # Errors
    /// [`IdError::Empty`] for an empty value; [`IdError::PartCount`] when there
    /// are not exactly three `::`-delimited parts;
    /// [`IdError::EmptyComponent`] when the `object_id` or `creating_system_id`
    /// part is empty; [`IdError::Malformed`] when either of those parts is not
    /// a legal `uid`; [`IdError::VersionTree`] when the third part is not a
    /// well-formed `VERSION_TREE_ID`.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        let parts = split3(&value).ok_or(IdError::PartCount {
            expected: 3,
            found: value.split("::").count(),
        })?;
        check_uid_part(parts[0], IdComponent::ObjectId, "object_id")?;
        check_uid_part(
            parts[1],
            IdComponent::CreatingSystemId,
            "creating_system_id",
        )?;
        if !is_valid_version_tree(parts[2]) {
            return Err(IdError::VersionTree(parts[2].to_owned()));
        }
        Ok(Self { value })
    }
}

impl ObjectVersionId {
    /// Compose an `OBJECT_VERSION_ID` from its three **typed** parts.
    ///
    /// Total, and total by grammar rather than by convention: §Syntaxes defines
    /// `object_version_id = object_id, '::', creating_system_id, '::',
    /// version_tree_id` with `object_id = uid`, `creating_system_id = uid`, and
    /// the third part a `VERSION_TREE_ID` — so three values of those types,
    /// joined by `::`, are a well-formed `object_version_id` with nothing left
    /// to reject. (`extension = ? any string ?` does not apply here: the
    /// three-part form has no extension slot, and no part may itself contain
    /// `::` because [`Uid`] and [`VersionTreeId`] both refuse it.)
    ///
    /// This is the door for a version id the CDR **mints** — where the parts are
    /// already typed and a fallible constructor would force a caller to handle
    /// an outcome the type system has already excluded. Parsing a version id
    /// off the wire is [`ObjectVersionId::new`].
    #[must_use]
    pub fn compose(
        object_id: &Uid,
        creating_system_id: &Uid,
        version_tree_id: &VersionTreeId,
    ) -> Self {
        Self {
            value: format!(
                "{}::{}::{}",
                object_id.value(),
                creating_system_id.value(),
                version_tree_id.value()
            ),
        }
    }
}

/// One `uid`-typed component of an `OBJECT_VERSION_ID`: present, and a legal
/// `uid` (BASE `master05-identification_package.adoc` §Syntaxes,
/// `object_id = uid` / `creating_system_id = uid`).
fn check_uid_part(part: &str, component: IdComponent, name: &'static str) -> Result<(), IdError> {
    if part.is_empty() {
        return Err(IdError::EmptyComponent(name));
    }
    if !is_uid(part) {
        return Err(IdError::Malformed {
            component,
            expected: IdProduction::Uid,
            found: part.to_owned(),
        });
    }
    Ok(())
}

impl FromStr for ObjectVersionId {
    type Err = IdError;

    /// Parse an `OBJECT_VERSION_ID` — see [`ObjectVersionId::new`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for ObjectVersionId {
    type Error = IdError;

    /// Parse an `OBJECT_VERSION_ID` — see [`ObjectVersionId::new`].
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<String> for ObjectVersionId {
    type Error = IdError;

    /// Parse an `OBJECT_VERSION_ID` — see [`ObjectVersionId::new`].
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl Validate for ObjectVersionId {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        // Non-empty is the UID.Value_valid inheritance.
        if self.value.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant Value_valid failed on type OBJECT_VERSION_ID",
            ));
            return;
        }
        if ObjectVersionId::from_str(&self.value).is_err() {
            out.push(InvariantViolation::here(
                "Invariant Value_format_valid failed on type OBJECT_VERSION_ID",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ovid(value: &str) -> ObjectVersionId {
        ObjectVersionId {
            value: value.to_owned(),
        }
    }

    #[test]
    fn accessors_trunk() {
        let o = ovid("1.2.840.113554.3.7.10::openEHR.org::1");
        assert!(matches!(o.object_id(), Uid::IsoOid(_)));
        assert!(matches!(o.creating_system_id(), Uid::InternetId(_)));
        assert_eq!(o.version_tree_id().value, "1");
        assert!(!o.is_branch());
    }

    /// The verbatim accessor preserves case where the typed one normalises it
    /// (BASE master05 §"Composite Identifiers and Case", case-preserving half).
    #[test]
    fn uppercase_uuid_recomposes_to_the_same_identifier() {
        let original = "8849182c-82ad-40A8-a07f-48ead4180515::ferroehr.example.org::1.2.3";
        let o = ObjectVersionId::new(original).expect("a legal OBJECT_VERSION_ID");
        assert_eq!(o.value(), original, "storage is case-preserving");
        let recomposed = ObjectVersionId::compose(
            &o.object_id(),
            &o.creating_system_id(),
            &o.version_tree_id(),
        );
        assert_eq!(
            recomposed.value(),
            "8849182c-82ad-40a8-a07f-48ead4180515::ferroehr.example.org::1.2.3",
            "the typed Uid door renders the UUID part in RFC 4122 lowercase"
        );
        assert!(
            super::super::lexical::composite_ids_equal(recomposed.value(), o.value()),
            "the two spellings identify the same thing (master05 case-insensitive)"
        );
    }

    #[test]
    fn creating_system_id_str_is_verbatim() {
        let o = ovid("1.2.840::openEHR.org::1");
        assert_eq!(o.creating_system_id_str(), "openEHR.org");
        let uuid_system = ovid("1.2.840::87284370-2D4B-4e3d-A3F3-F303D2F4F34B::1");
        assert_eq!(
            uuid_system.creating_system_id_str(),
            "87284370-2D4B-4e3d-A3F3-F303D2F4F34B"
        );
        // The typed accessor normalises the same part to lower case.
        assert_eq!(
            uuid_system.creating_system_id().value(),
            "87284370-2d4b-4e3d-a3f3-f303d2f4f34b"
        );
        // Total on a malformed id, like its siblings.
        assert_eq!(ovid("not-an-ovid").creating_system_id_str(), "");
    }

    #[test]
    fn accessors_branch() {
        let o = ovid("87284370-2D4B-4e3d-A3F3-F303D2F4F34B::openEHR.org::2.1.4");
        assert!(o.is_branch());
        assert_eq!(o.version_tree_id().branch_number(), Some("1"));
    }

    #[test]
    fn from_str_strict() {
        assert!("a::b::1".parse::<ObjectVersionId>().is_ok());
        assert!("a::b::1.2.3".parse::<ObjectVersionId>().is_ok());
        assert_eq!(
            "a::b".parse::<ObjectVersionId>(),
            Err(IdError::PartCount {
                expected: 3,
                found: 2
            })
        );
        assert_eq!(
            "::b::1".parse::<ObjectVersionId>(),
            Err(IdError::EmptyComponent("object_id"))
        );
        assert!(matches!(
            "a::b::1.2".parse::<ObjectVersionId>(),
            Err(IdError::VersionTree(_))
        ));
        assert_eq!("".parse::<ObjectVersionId>(), Err(IdError::Empty));
    }

    /// `object_id = uid` and `creating_system_id = uid` (BASE `master05`
    /// §Syntaxes): a part that matches none of `iso_oid | uuid | internet_id`
    /// is refused, and the error names WHICH part and what was expected.
    #[test]
    fn uid_parts_run_the_master05_grammar() {
        // The three `uid` productions, in both uid positions.
        for raw in [
            "87284370-2D4B-4e3d-A3F3-F303D2F4F34B::uk.nhs.ehr1::2",
            "1.2.840.113554.3.7.10::openEHR.org::1",
            "12345::1.2.840::1.2.3",
            "openehr.org::87284370-2d4b-4e3d-a3f3-f303d2f4f34b::1",
        ] {
            assert!(
                ObjectVersionId::new(raw).is_ok(),
                "{raw:?} is a legal OBJECT_VERSION_ID"
            );
        }

        for (raw, component, found) in [
            ("1234-5678::sys::1", IdComponent::ObjectId, "1234-5678"),
            ("bad id::sys::1", IdComponent::ObjectId, "bad id"),
            ("-lead::sys::1", IdComponent::ObjectId, "-lead"),
            (
                "1-2-3-4-5::sys::1",
                IdComponent::ObjectId,
                // Five hex groups are NOT a UUID: the widths are not 8-4-4-4-12.
                "1-2-3-4-5",
            ),
            (
                "sys::1234-5678::1",
                IdComponent::CreatingSystemId,
                "1234-5678",
            ),
            (
                "sys::trailing-::1",
                IdComponent::CreatingSystemId,
                "trailing-",
            ),
        ] {
            assert_eq!(
                ObjectVersionId::new(raw),
                Err(IdError::Malformed {
                    component,
                    expected: IdProduction::Uid,
                    found: found.to_owned(),
                }),
                "for {raw:?}"
            );
        }

        // An empty middle part keeps its own, more specific error.
        assert_eq!(
            ObjectVersionId::new("a::::1"),
            Err(IdError::EmptyComponent("creating_system_id"))
        );
    }

    /// Construction is case-PRESERVING while identity is case-INSENSITIVE
    /// (master05 §"Composite Identifiers and Case").
    #[test]
    fn construction_preserves_case() {
        let upper = ObjectVersionId::new("87284370-2D4B-4E3D-A3F3-F303D2F4F34B::UK.NHS.EHR1::2")
            .expect("a well-formed version id");
        let lower = ObjectVersionId::new("87284370-2d4b-4e3d-a3f3-f303d2f4f34b::uk.nhs.ehr1::2")
            .expect("a well-formed version id");
        assert_eq!(
            upper.value,
            "87284370-2D4B-4E3D-A3F3-F303D2F4F34B::UK.NHS.EHR1::2"
        );
        assert_ne!(upper.value, lower.value);
        assert!(upper.is_equal(&lower));
        assert_eq!(upper.creating_system_id_str(), "UK.NHS.EHR1");
    }

    #[test]
    fn validate_format() {
        assert!(ovid("a::b::1").invariants().is_empty());
        let bad = ovid("a::b");
        assert!(
            bad.invariants()
                .iter()
                .any(|v| v.message
                    == "Invariant Value_format_valid failed on type OBJECT_VERSION_ID")
        );
        let empty = ovid("");
        assert!(
            empty
                .invariants()
                .iter()
                .any(|v| v.message == "Invariant Value_valid failed on type OBJECT_VERSION_ID")
        );
    }
}
