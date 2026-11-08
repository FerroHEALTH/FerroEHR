// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lifecycle of a SCIM User: the version a write commits, as a function
//! of the head's state, the request's `active` and whether the write changes
//! anything.
//!
//! Every transition is one the RM common `master06-change_control_package.adoc`
//! §Version Lifecycle state machine draws, which the commit's own guard
//! enforces again: `complete` and `incomplete` update, `complete` deactivates
//! to `inactive`, `incomplete` is abandoned, `inactive` is reactivated or
//! retrieved, and `abandoned` is retrieved. "Transitions through a number of
//! successive states entails a number of commits, each recording one
//! transition" (§Version Lifecycle), so `abandoned` to `complete` is two
//! commits. No write moves an `inactive` or `abandoned` `PERSON` to a state a
//! login binds without `active = true`. Mapping SCIM `active` to the lifecycle
//! is our own design.

use crate::versioning::lifecycle::state;

/// The versions a User write commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// No new version: the request changes nothing.
    Unchanged,
    /// One version in this `version_lifecycle_state`.
    One(&'static str),
    /// A version that retrieves the User to `incomplete`, then a second that
    /// completes it.
    RetrieveThenComplete,
}

/// Why a User write is refused (RFC 7644 §3.12 `invalidValue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// The head is inactive or abandoned and the request changes attributes
    /// without reactivating the User.
    #[error("the User is not active: reactivate the User first, with active = true")]
    ReactivateFirst,
    /// The head is in a state no SCIM write starts from.
    #[error("the User's PERSON is in a lifecycle state no SCIM write starts from")]
    State,
}

/// The versions a write of a User whose head is in `head` commits.
///
/// `active` is the request's (`None` when it leaves it out), `changed` says
/// whether it changes an attribute, and `fits` whether the new body fits its
/// template in full.
///
/// # Errors
/// [`Refused::ReactivateFirst`] for an attribute change to an inactive or
/// abandoned User that does not set `active = true`; [`Refused::State`] for a
/// head in any other state, such as deleted.
pub fn transition(
    head: &str,
    active: Option<bool>,
    changed: bool,
    fits: bool,
) -> Result<Transition, Refused> {
    let updated = if fits {
        state::COMPLETE
    } else {
        state::INCOMPLETE
    };
    match (head, active) {
        (state::COMPLETE, Some(false)) => Ok(Transition::One(state::INACTIVE)),
        (state::INCOMPLETE, Some(false)) => Ok(Transition::One(state::ABANDONED)),
        (state::COMPLETE | state::INCOMPLETE, _) | (state::INACTIVE, Some(true)) => {
            Ok(Transition::One(updated))
        }
        (state::INACTIVE | state::ABANDONED, Some(false) | None) if !changed => {
            Ok(Transition::Unchanged)
        }
        (state::INACTIVE | state::ABANDONED, Some(false) | None) => Err(Refused::ReactivateFirst),
        (state::ABANDONED, Some(true)) if fits => Ok(Transition::RetrieveThenComplete),
        (state::ABANDONED, Some(true)) => Ok(Transition::One(state::INCOMPLETE)),
        _ => Err(Refused::State),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row: the head, `active`, whether attributes change, whether the
    /// body fits, and the outcome.
    type Row = (
        &'static str,
        Option<bool>,
        bool,
        bool,
        Result<Transition, Refused>,
    );

    /// Each row of the lifecycle table of #5014, after the first version.
    #[test]
    fn every_row_of_the_lifecycle_table() {
        use Transition::{One, RetrieveThenComplete, Unchanged};
        let rows: [Row; 18] = [
            // complete, active true or absent: complete when it fits, else incomplete.
            ("532", Some(true), true, true, Ok(One("532"))),
            ("532", None, true, false, Ok(One("553"))),
            // complete, active false: deactivate.
            ("532", Some(false), false, true, Ok(One("800"))),
            ("532", Some(false), true, false, Ok(One("800"))),
            // incomplete, active true or absent: complete when it fits.
            ("553", None, true, true, Ok(One("532"))),
            ("553", Some(true), true, false, Ok(One("553"))),
            // incomplete, active false: abandon.
            ("553", Some(false), false, false, Ok(One("801"))),
            // inactive, active false with a change: refused, the state stays.
            (
                "800",
                Some(false),
                true,
                true,
                Err(Refused::ReactivateFirst),
            ),
            // inactive, active true: reactivate, or retrieve when it does not fit.
            ("800", Some(true), false, true, Ok(One("532"))),
            ("800", Some(true), true, false, Ok(One("553"))),
            // abandoned, active false with a change: refused.
            (
                "801",
                Some(false),
                true,
                true,
                Err(Refused::ReactivateFirst),
            ),
            // abandoned, active true: retrieve, then complete as a second commit.
            ("801", Some(true), false, true, Ok(RetrieveThenComplete)),
            ("801", Some(true), true, false, Ok(One("553"))),
            // inactive or abandoned, active absent with a change: refused.
            ("800", None, true, true, Err(Refused::ReactivateFirst)),
            ("801", None, true, true, Err(Refused::ReactivateFirst)),
            // inactive or abandoned, active false and no change: no new version.
            ("800", Some(false), false, true, Ok(Unchanged)),
            ("801", Some(false), false, false, Ok(Unchanged)),
            // a deleted head starts no write.
            ("523", Some(true), true, true, Err(Refused::State)),
        ];
        for (head, active, changed, fits, expected) in rows {
            assert_eq!(
                transition(head, active, changed, fits),
                expected,
                "{head} active={active:?} changed={changed} fits={fits}"
            );
        }
    }

    /// No request moves an inactive or abandoned `PERSON` to a state a login
    /// binds (`complete` or `incomplete`) without `active = true`.
    #[test]
    fn nothing_but_active_true_makes_an_inactive_user_bindable() {
        for head in ["800", "801"] {
            for active in [None, Some(false)] {
                for changed in [false, true] {
                    for fits in [false, true] {
                        let outcome = transition(head, active, changed, fits);
                        assert!(
                            !matches!(
                                outcome,
                                Ok(Transition::One("532" | "553")
                                    | Transition::RetrieveThenComplete)
                            ),
                            "{head} active={active:?} changed={changed} fits={fits}: {outcome:?}"
                        );
                    }
                }
            }
        }
    }
}
