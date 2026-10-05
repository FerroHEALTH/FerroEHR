// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Version lifecycle: the `version_lifecycle_state` codes and the state-machine
//! that governs transitions between them.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Version Lifecycle.
//! `ORIGINAL_VERSION.lifecycle_state` is coded from the openEHR
//! `version lifecycle state` group; "the possible values are `532|complete|`,
//! `553|incomplete|`, `523|deleted|`, `800|inactive|` and `801|abandoned|`"
//! (master06 §Version Lifecycle). "One basic rule … is that any transition
//! requires the commit of a _new version_, even if the content is otherwise
//! unchanged" — realized by the storage layer (a new `version` row per
//! commit). This module owns the **legality** of each transition: reading
//! the current state from the preceding version and rejecting a transition the
//! spec does not sanction.

use crate::service::error::{ServiceError, Violation};
use openehr_term::bundle::openehr;

/// The `version_lifecycle_state` openEHR terminology group id.
const VERSION_LIFECYCLE_STATE: &str = "version_lifecycle_state";

/// `version_lifecycle_state` group codes (master06 §Version Lifecycle).
pub(crate) mod state {
    /// `532|complete|` — a fully authored version.
    pub(crate) const COMPLETE: &str = "532";
    /// `553|incomplete|` — a partial/unreviewed version committed with relaxed
    /// validation (master06 §Incomplete Content).
    pub(crate) const INCOMPLETE: &str = "553";
    /// `523|deleted|` — a logically deleted version (master06 §Logical Deletion).
    pub(crate) const DELETED: &str = "523";
    /// `800|inactive|` — content marked no longer valid for use (master06
    /// §Abandoned and Inactive States).
    pub(crate) const INACTIVE: &str = "800";
    /// `801|abandoned|` — incomplete content that lost relevance before
    /// completion (master06 §Abandoned and Inactive States).
    pub(crate) const ABANDONED: &str = "801";
}

/// Resolve an inbound `lifecycle_state` token — a numeric group code (`"553"`)
/// or a rubric (`"incomplete"`) — to its canonical numeric
/// `version_lifecycle_state` code. `None` when the token is not a member of the
/// group (`ORIGINAL_VERSION.Lifecycle_state_valid`; master06 §Version
/// Lifecycle: the five values `532/553/523/800/801`).
pub(crate) fn lifecycle_state_code(token: &str) -> Option<String> {
    let t = openehr();
    if t.is_valid_version_lifecycle_state(token) {
        return Some(token.to_owned());
    }
    t.concepts_in_group(VERSION_LIFECYCLE_STATE)
        .iter()
        .find(|c| c.rubric.eq_ignore_ascii_case(token))
        .map(|c| c.id.clone())
}

/// The rubric (English display text) for a `version_lifecycle_state` code;
/// falls back to the code itself if unknown. Note the SPECPR-51 quirk: `532` is
/// `complete` in this group (`completed` only in `instruction_states`) — the
/// group-scoped lookup resolves it correctly.
pub(crate) fn lifecycle_rubric(code: &str) -> String {
    openehr()
        .rubric(VERSION_LIFECYCLE_STATE, code, "en")
        .unwrap_or(code)
        .to_owned()
}

/// Resolve a client-supplied `version_lifecycle_state` token into its canonical
/// numeric code, defaulting to `532|complete|` when absent (master06 §Version
/// Lifecycle). An out-of-group token is a `422`
/// (`ORIGINAL_VERSION.Lifecycle_state_valid`), naming the terminology group.
pub(crate) fn resolve_lifecycle(token: Option<String>) -> Result<String, ServiceError> {
    match token {
        Some(token) => lifecycle_state_code(&token).ok_or_else(|| {
            ServiceError::content_invalid(
                Violation::new(format!(
                    "{token:?} is not a code in the openEHR version_lifecycle_state group"
                ))
                .with_path("lifecycle_state")
                .with_invariant("ORIGINAL_VERSION.Lifecycle_state_valid"),
            )
        }),
        None => Ok(state::COMPLETE.to_owned()),
    }
}

/// Refuse `523|deleted|` on a commit that carries `data`.
///
/// master06 §Logical Deletion states deletion as one indivisible procedure,
/// "create a new Version in the normal way; delete its `_data_` …; set the
/// `_lifecycle_state_` value to the code for `deleted`; commit in the normal
/// way", so a data-carrying version in the `deleted` state is not producible by
/// the spec's own procedure: it would serve `204` for the resource while its
/// content stayed stored and AQL-queryable. `ORIGINAL_VERSION.data` is typed
/// `0..1` precisely so a deleted version can carry none.
///
/// This mirrors the CONTRIBUTION `classify` rule coupling change type `523` to
/// data-absence, on the lifecycle axis, so the CONTRIBUTION member
/// (`UPDATE_VERSION.lifecycle_state`) and the direct route's `openehr-version`
/// header reach the same refusal. The code is the ITS-REST overview 422 row
/// (`Requests_and_responses.md` §HTTP status codes).
///
/// # Errors
/// [`ServiceError::Unprocessable`] when `lifecycle` is `523|deleted|`.
pub(crate) fn reject_deleted_with_data(lifecycle: &str) -> Result<(), ServiceError> {
    if lifecycle != state::DELETED {
        return Ok(());
    }
    Err(ServiceError::content_invalid(
        Violation::new(
            "523|deleted| is invalid on a version that carries data — logical deletion \
             deletes the version's data and sets the deleted state in one act",
        )
        .with_path("lifecycle_state")
        .with_invariant("RM common master06 §Logical Deletion"),
    ))
}

/// Whether a `version_lifecycle_state` transition `from -> to` is sanctioned by
/// the master06 §Version Lifecycle state machine.
///
/// The authority is the FORMAL machine the section designates — "The following
/// diagram shows the formal state machine of the
/// `ORIGINAL_VERSION._lifecycle_state_` attribute"
/// (`RM/docs/UML/diagrams/RM-version_lifecycle.svg`); the §Abandoned and
/// Inactive States prose table covers only that pair's transitions. The
/// diagram's exact edge set:
///
/// | from | allowed to (edge) |
/// |---|---|
/// | (new)        | `incomplete` (`create_draft`), `complete` (`create_final`) |
/// | `complete`   | `complete` (update), `incomplete` (update), `inactive` (deactivate), `deleted` (delete) |
/// | `incomplete` | `incomplete` (update), `complete` (complete), `abandoned` (abandon), `deleted` (delete) |
/// | `inactive`   | `complete` (reactivate), `incomplete` (retrieve), `deleted` (delete) |
/// | `abandoned`  | `incomplete` (retrieve), `deleted` (delete) |
/// | `deleted`    | `complete`/`incomplete` (revert) |
///
/// The machine draws self-`update` loops only on `complete` and `incomplete`, so
/// a same-state re-commit of `inactive`, `abandoned` or `deleted` content is not
/// a drawn transition; editing resumes through `reactivate`, `retrieve` or
/// `revert` first.
///
/// A transition outside this table is a `422` naming the state machine. The
/// import replay does not call this, preserving source history verbatim
/// (master06 §Copying), and the logical-delete path targets `deleted` from any
/// live state (master06 §Logical Deletion, matching the diagram's four `delete`
/// edges).
pub(crate) fn validate_transition(from: Option<&str>, to: &str) -> Result<(), ServiceError> {
    use state::{ABANDONED, COMPLETE, DELETED, INACTIVE, INCOMPLETE};

    let allowed = match from {
        // A first version can only be authored `complete` or `incomplete`
        // (create_final / create_draft).
        None => matches!(to, COMPLETE | INCOMPLETE),
        Some(COMPLETE) => matches!(to, COMPLETE | INCOMPLETE | INACTIVE | DELETED),
        Some(INCOMPLETE) => matches!(to, INCOMPLETE | COMPLETE | ABANDONED | DELETED),
        Some(INACTIVE) => matches!(to, COMPLETE | INCOMPLETE | DELETED),
        Some(ABANDONED) => matches!(to, INCOMPLETE | DELETED),
        // The two `revert` edges out of `deleted` (restoration, change
        // type `816`).
        Some(DELETED) => matches!(to, COMPLETE | INCOMPLETE),
        // An unknown stored state (should be impossible past the CHECK) is
        // treated permissively — the terminology guard is the real gate.
        Some(_) => true,
    };
    if allowed {
        return Ok(());
    }
    let (from_code, from_rubric) = match from {
        Some(c) => (c, lifecycle_rubric(c)),
        None => ("(new)", "new".to_owned()),
    };
    Err(ServiceError::content_invalid(
        Violation::new(format!(
            "illegal version lifecycle transition {from_code}|{from_rubric}| -> {to}|{}|",
            lifecycle_rubric(to),
        ))
        .with_invariant("RM common master06 §Version Lifecycle state machine"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every named constant is a real group member, and the constants cover the
    /// COMPLETE `version_lifecycle_state` group — a code added to the TERM
    /// bundle without a constant here fails this test. (The same guard the
    /// `audit_change_type` and `composition_category` constant sets carry;
    /// master06 §Version Lifecycle names exactly these five values.)
    #[test]
    fn lifecycle_constants_are_the_complete_group() {
        let all = [
            state::COMPLETE,
            state::INCOMPLETE,
            state::DELETED,
            state::INACTIVE,
            state::ABANDONED,
        ];
        let t = openehr();
        for code in all {
            assert!(t.is_valid_version_lifecycle_state(code), "code {code}");
            // `code_string` must be numeric (the group's wire form).
            assert!(code.chars().all(|c| c.is_ascii_digit()), "code {code}");
        }
        let mut group: Vec<String> = t
            .concepts_in_group(VERSION_LIFECYCLE_STATE)
            .iter()
            .map(|c| c.id.clone())
            .collect();
        group.sort();
        let mut named: Vec<String> = all.iter().map(|c| (*c).to_owned()).collect();
        named.sort();
        assert_eq!(group, named, "constants must mirror the full TERM group");
    }

    #[test]
    fn lifecycle_state_code_accepts_code_or_rubric_and_rejects_non_members() {
        assert_eq!(lifecycle_state_code("532").as_deref(), Some("532"));
        assert_eq!(lifecycle_state_code("complete").as_deref(), Some("532"));
        assert_eq!(lifecycle_state_code("incomplete").as_deref(), Some("553"));
        assert_eq!(lifecycle_state_code("Deleted").as_deref(), Some("523"));
        assert_eq!(lifecycle_state_code("inactive").as_deref(), Some("800"));
        assert_eq!(lifecycle_state_code("abandoned").as_deref(), Some("801"));
        // Out-of-group tokens rejected (ORIGINAL_VERSION.Lifecycle_state_valid).
        assert_eq!(lifecycle_state_code("not-a-state"), None);
        assert_eq!(lifecycle_state_code("249"), None); // change-type code, wrong group
    }

    #[test]
    fn lifecycle_rubric_specpr51() {
        // SPECPR-51: version lifecycle 532 → "complete" (not "completed").
        assert_eq!(lifecycle_rubric(state::COMPLETE), "complete");
        assert_eq!(lifecycle_rubric(state::DELETED), "deleted");
    }

    #[test]
    fn resolve_defaults_to_complete() {
        assert_eq!(resolve_lifecycle(None).unwrap(), state::COMPLETE);
        assert_eq!(
            resolve_lifecycle(Some("incomplete".into())).unwrap(),
            state::INCOMPLETE
        );
        // The refusal is asserted as DATA: the path and the named invariant,
        // not a substring of the rendered sentence.
        match resolve_lifecycle(Some("nonsense".into())) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("lifecycle_state"));
                assert_eq!(
                    v.invariant(),
                    Some("ORIGINAL_VERSION.Lifecycle_state_valid")
                );
                assert!(v.detail().contains("\"nonsense\""), "{v}");
            }
            other => panic!("an out-of-group lifecycle token must be 422, got {other:?}"),
        }
    }

    /// master06 §Logical Deletion couples the `deleted` state to the data-Void
    /// in ONE act, so `523` is refused on a data-carrying commit — twinned
    /// against every other state, which passes unchanged.
    #[test]
    fn deleted_state_is_refused_on_a_data_carrying_commit() {
        // The refusal, asserted as DATA (path + named spec rule).
        match reject_deleted_with_data(state::DELETED) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert_eq!(v.path(), Some("lifecycle_state"));
                assert_eq!(v.invariant(), Some("RM common master06 §Logical Deletion"));
            }
            other => panic!("523 on a data-carrying version must be 422, got {other:?}"),
        }
        // The accepting twins: every other lifecycle state carries data.
        for code in [
            state::COMPLETE,
            state::INCOMPLETE,
            state::INACTIVE,
            state::ABANDONED,
        ] {
            assert!(
                reject_deleted_with_data(code).is_ok(),
                "state {code} carries data legitimately"
            );
        }
    }

    /// the master06 §Version Lifecycle state machine — legal transitions
    /// pass, illegal ones are rejected.
    #[test]
    fn transition_state_machine() {
        use state::{ABANDONED, COMPLETE, DELETED, INACTIVE, INCOMPLETE};

        // First version: only complete / incomplete.
        assert!(validate_transition(None, COMPLETE).is_ok());
        assert!(validate_transition(None, INCOMPLETE).is_ok());
        assert!(validate_transition(None, INACTIVE).is_err());
        assert!(validate_transition(None, ABANDONED).is_err());
        assert!(validate_transition(None, DELETED).is_err());

        // The drawn transitions of the formal machine
        // (RM-version_lifecycle.svg, designated by §Version Lifecycle).
        assert!(validate_transition(Some(COMPLETE), INACTIVE).is_ok()); // deactivate
        assert!(validate_transition(Some(INACTIVE), COMPLETE).is_ok()); // reactivate
        assert!(validate_transition(Some(INACTIVE), INCOMPLETE).is_ok()); // retrieve
        assert!(validate_transition(Some(INCOMPLETE), COMPLETE).is_ok()); // complete
        assert!(validate_transition(Some(INCOMPLETE), ABANDONED).is_ok()); // abandon
        assert!(validate_transition(Some(ABANDONED), INCOMPLETE).is_ok()); // retrieve
        assert!(validate_transition(Some(ABANDONED), DELETED).is_ok()); // delete
        assert!(validate_transition(Some(DELETED), COMPLETE).is_ok()); // revert
        assert!(validate_transition(Some(COMPLETE), INCOMPLETE).is_ok()); // update (drawn complete -> incomplete)

        // Self-`update` loops are drawn ONLY on complete and incomplete.
        assert!(validate_transition(Some(COMPLETE), COMPLETE).is_ok());
        assert!(validate_transition(Some(INCOMPLETE), INCOMPLETE).is_ok());
        assert!(validate_transition(Some(INACTIVE), INACTIVE).is_err());
        assert!(validate_transition(Some(ABANDONED), ABANDONED).is_err());

        // Illegal transitions. The first is asserted as DATA — the refusal
        // names the state machine it enforces, readable off the value.
        match validate_transition(Some(COMPLETE), ABANDONED) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => assert_eq!(
                v.invariant(),
                Some("RM common master06 §Version Lifecycle state machine")
            ),
            other => panic!("complete -> abandoned must be refused, got {other:?}"),
        }
        assert!(validate_transition(Some(COMPLETE), ABANDONED).is_err());
        assert!(validate_transition(Some(ABANDONED), COMPLETE).is_err());
        assert!(validate_transition(Some(ABANDONED), INACTIVE).is_err());
        assert!(validate_transition(Some(INACTIVE), ABANDONED).is_err());
    }
}
