// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version precondition of a demographic update or delete, parsed once by
//! the protocol adapter from `If-Match` and the delete path.

use std::str::FromStr;

use openehr_base::prelude::ObjectVersionId;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::storage::version_repo::placement::TipRow;
use crate::versioning::change::precondition::tip_uid;
use crate::versioning::object_version_id::{
    TreeId, VersionIdError, entity_tag_value, expected_from_if_match,
};

/// A demographic write's version precondition, judged once against the tip
/// the write locks (`versioning::change::precondition::lock_current`).
///
/// The `If-Match` value arrives as a token: a full `OBJECT_VERSION_ID`, a
/// bare `VERSION_TREE_ID` trunk number, or the RFC 9110 `*` wildcard. A
/// delete also names its preceding version in the path, which stands in
/// when no `If-Match` names one.
#[derive(Debug, Clone)]
pub struct Precondition {
    /// A full `OBJECT_VERSION_ID` the tip must be (ITS-REST overview
    /// `Requests_and_responses.md` §"If-Match and accidental overwrites": a
    /// false condition is `412`).
    full: Option<ObjectVersionId>,
    /// The `VERSION_TREE_ID` the tip must be at.
    tree: Option<TreeId>,
}

impl Precondition {
    /// The precondition of an `If-Match` value, with `path` as the version a
    /// delete names when the header names none.
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] for a malformed `If-Match`, rejected rather
    /// than run as if no precondition was sent.
    pub fn parse(if_match: Option<&str>, path: Option<TreeId>) -> Result<Self, ServiceError> {
        let Some(raw) = if_match else {
            return Ok(Self {
                full: None,
                tree: path,
            });
        };
        // NOTE: a value that is not a full OBJECT_VERSION_ID is legitimately the
        // `*` or trunk form; `expected_from_if_match` below refuses every other shape.
        let full = ObjectVersionId::from_str(&entity_tag_value(raw)).ok();
        let tree = expected_from_if_match(raw).map_err(VersionIdError::into_argument)?;
        Ok(Self {
            full,
            tree: tree.or(path),
        })
    }

    /// The full-identity half: `412` when the `If-Match` names another
    /// version than the locked tip, compared case-insensitively (BASE
    /// `base_types` master05 §"Composite Identifiers and Case").
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] on a mismatch.
    pub(super) fn check_full(&self, vo_id: VoId, tip: &TipRow) -> Result<(), ServiceError> {
        crate::versioning::change::precondition::ensure_preceding(
            &tip_uid(vo_id, tip),
            self.full.as_ref(),
        )
    }

    /// The version-tree half: a stale preceding version is a version
    /// mismatch, which the wire answers with the latest `version_uid` in
    /// `ETag` (`responses/409_PERSON_with_uid_based_id.yaml`).
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the locked tip is at another
    /// version.
    pub(super) fn check_tree(&self, tip: &TipRow) -> Result<(), ServiceError> {
        let current =
            TreeId::from_columns(tip.trunk_version, tip.branch_number, tip.branch_version);
        match self.tree {
            Some(expected) if expected != current => Err(ServiceError::version_conflict(format!(
                "preceding_version_uid names version {expected}, latest is {current}"
            ))),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VO: &str = "8849182c-82ad-4088-a07f-48ead4180515";

    fn vo() -> VoId {
        VoId(VO.parse().expect("uuid"))
    }

    /// The locked tip `{VO}::openEHRSys.example.com::{trunk}`.
    fn tip(trunk: i32) -> TipRow {
        TipRow {
            ehr_id: None,
            kind: "PERSON".to_owned(),
            sys_version: trunk,
            trunk_version: trunk,
            branch_number: 0,
            branch_version: 0,
            creating_system_id: "openEHRSys.example.com".to_owned(),
            lifecycle_state: "532".to_owned(),
            committed_at: jiff::Timestamp::UNIX_EPOCH,
            content_digest: None,
            content_key: trunk,
            open: true,
        }
    }

    fn full(if_match: &str) -> Result<(), ServiceError> {
        Precondition::parse(Some(if_match), None)?.check_full(vo(), &tip(2))
    }

    /// An `If-Match` naming the locked tip in full satisfies the precondition
    /// (ITS-REST overview §"If-Match and accidental overwrites").
    #[test]
    fn matching_full_ovid_passes() {
        let uid = format!("{VO}::openEHRSys.example.com::2");
        assert!(full(&uid).is_ok());
        // The quote tolerance of the library boundary.
        assert!(full(&format!("\"{uid}\"")).is_ok());
    }

    /// `creating_system_id` is a composite identifier, so a case variant names
    /// the SAME version and must NOT raise a spurious `412` (BASE `base_types`
    /// master05 §"Composite Identifiers and Case").
    #[test]
    fn case_variant_creating_system_id_matches() {
        for variant in [
            format!("{VO}::OPENEHRSYS.EXAMPLE.COM::2"),
            format!("{VO}::openehrsys.example.com::2"),
            format!("{}::openEHRSys.example.com::2", VO.to_uppercase()),
        ] {
            assert!(
                full(&variant).is_ok(),
                "BASE master05 §Composite Identifiers and Case: {variant:?} names the \
                 same version as the locked tip"
            );
        }
    }

    /// A stale full `OBJECT_VERSION_ID` — a mismatch in ANY segment — is the
    /// `412` branch.
    #[test]
    fn stale_or_foreign_ovid_is_a_version_mismatch() {
        for stale in [
            // wrong version_tree_id
            format!("{VO}::openEHRSys.example.com::1"),
            // wrong creating_system_id (not a mere case variant)
            format!("{VO}::other.system::2"),
            // wrong object_id
            "00000000-0000-4000-8000-0000000000ff::openEHRSys.example.com::2".to_owned(),
        ] {
            let err = full(&stale).expect_err("stale precondition");
            assert!(
                matches!(err, ServiceError::VersionConflict(_)),
                "{stale:?} must fail the precondition, got {err:?}"
            );
        }
    }

    /// An absent header states no precondition.
    #[test]
    fn absent_header_states_no_precondition() {
        let precondition = Precondition::parse(None, None).expect("no header");
        assert!(precondition.check_full(vo(), &tip(2)).is_ok());
        assert!(precondition.check_tree(&tip(2)).is_ok());
    }

    /// A non-OVID token carries no full identity to compare: the RFC 9110 `*`
    /// wildcard passes, the lenient trunk number is the tree precondition, and
    /// any other shape is rejected as malformed (`400`); nothing is silently
    /// skipped.
    #[test]
    fn non_ovid_tokens_are_the_tree_precondition() {
        let wildcard = Precondition::parse(Some("*"), None).expect("wildcard");
        assert!(wildcard.check_full(vo(), &tip(2)).is_ok());
        assert!(wildcard.check_tree(&tip(2)).is_ok());
        let trunk = Precondition::parse(Some("2"), None).expect("trunk number");
        assert!(trunk.check_full(vo(), &tip(2)).is_ok());
        assert!(trunk.check_tree(&tip(2)).is_ok());
        assert!(matches!(
            trunk.check_tree(&tip(3)),
            Err(ServiceError::VersionConflict(_))
        ));
        assert!(Precondition::parse(Some("garbage"), None).is_err());
    }

    /// A delete's path version stands in when no `If-Match` names one, and an
    /// `If-Match` naming one wins over it.
    #[test]
    fn path_version_stands_in_for_an_absent_if_match() {
        let path = Some(TreeId::trunk(1));
        let from_path = Precondition::parse(None, path).expect("no header");
        assert!(matches!(
            from_path.check_tree(&tip(2)),
            Err(ServiceError::VersionConflict(_))
        ));
        let from_header = Precondition::parse(Some("2"), path).expect("trunk number");
        assert!(from_header.check_tree(&tip(2)).is_ok());
    }
}
