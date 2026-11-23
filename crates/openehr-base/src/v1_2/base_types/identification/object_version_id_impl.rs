// @generated-from-template templates/openehr-base/base_types/identification/object_version_id_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Accessors and the lexical check for `OBJECT_VERSION_ID` (BASE 1.3.0
//! `org.openehr.base.base_types.object_version_id.adoc`):
//! `object_id '::' creating_system_id '::' version_tree_id`.
//!
//! The BMM lists no invariant; `Value_format_valid` is our own name for the
//! three-part form.

use super::hier_object_id::HierObjectId;
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
    /// The logical object's UID, the first part (BASE `object_id`); the whole
    /// value for a malformed id.
    #[must_use]
    pub fn object_id(&self) -> Uid {
        make_uid(self.object_id_str())
    }

    /// The `object_id` part verbatim, case preserved (master05 §"Composite
    /// Identifiers and Case"); the whole value for a malformed id.
    #[must_use]
    pub fn object_id_str(&self) -> &str {
        split3(&self.value).map_or(self.value.as_str(), |p| p[0])
    }

    /// The `HIER_OBJECT_ID` of the versioned object: the `object_id` part,
    /// verbatim.
    #[must_use]
    pub fn versioned_object_id(&self) -> HierObjectId {
        HierObjectId {
            value: self.object_id_str().to_owned(),
        }
    }

    /// Identifier of the system that created this version — the second
    /// `::`-delimited part (BASE `creating_system_id`).
    #[must_use]
    pub fn creating_system_id(&self) -> Uid {
        make_uid(self.creating_system_id_str())
    }

    /// The `creating_system_id` part verbatim, case preserved (master05
    /// §"Composite Identifiers and Case"); empty for a malformed id.
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
    /// Builds an `OBJECT_VERSION_ID` from its string form, stored verbatim
    /// (master05 §Syntaxes `object_version_id`).
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
    /// Composes an `OBJECT_VERSION_ID` from its three typed parts, which the
    /// grammar makes total; parsing is [`ObjectVersionId::new`].
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

/// One `uid` component of an `OBJECT_VERSION_ID`: present and a legal `uid`.
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
    fn the_object_id_is_kept_as_written() {
        let o = ovid("8849182C-82AD-4088-A07F-48EAD4180515::Host.Example::2");
        assert_eq!(o.object_id_str(), "8849182C-82AD-4088-A07F-48EAD4180515");
        assert_eq!(
            o.versioned_object_id().value,
            "8849182C-82AD-4088-A07F-48EAD4180515"
        );
        assert_eq!(o.creating_system_id_str(), "Host.Example");
        assert_eq!(ovid("no-separators").object_id_str(), "no-separators");
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

    /// A part matching no `uid` production is refused, naming the part.
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
        assert_eq!(
            ovid("a::b::1").invariants(),
            Vec::<InvariantViolation>::new()
        );
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
