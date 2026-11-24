// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy and definition revisions a request's own statements read
//! (#6251).
//!
//! A statement that judges an EHR selects its settings revision from
//! `access_revision_of`, and with it the policy and definition revisions, at
//! the snapshot of the rows it reads. A request whose policy copies are
//! confirmed after it ran takes from here the newest revisions its statements
//! saw, and its answer leaves only once the copies hold them. A write judges
//! its gate read's revisions before it writes, and a run that committed a
//! write is never run again (#6256). A party statement, whose schema holds no
//! policy, reads them through `clinical_door.policy_revisions()` where the two
//! domains share a database (#6320). No openEHR spec governs the copies: our
//! own design.

use std::cell::Cell;
use std::future::Future;

/// The newest policy and definition revisions a request's statements read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed {
    /// The newest `policy_revision` read.
    pub policy: i64,
    /// The newest `definition_revision` read.
    pub definitions: i64,
}

/// What a request's run left: the newest revisions its statements read, and
/// whether it committed a write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    /// The newest revisions read; `None` when no statement read them.
    pub observed: Option<Observed>,
    /// Whether the run committed a write, which must never run again.
    pub wrote: bool,
}

tokio::task_local! {
    static SEEN: Cell<Seen>;
    /// Set while a request's statements run on the party pool of a service
    /// whose party roles may run `clinical_door.policy_revisions()` (#6320).
    static DOOR: ();
}

/// Runs `fut` gathering the revisions its statements read and whether it
/// committed a write, and returns them with its output.
pub async fn scope<F: Future>(fut: F) -> (F::Output, Seen) {
    SEEN.scope(Cell::new(Seen::default()), async move {
        let output = fut.await;
        (output, SEEN.with(Cell::get))
    })
    .await
}

/// Runs `fut`, statements on the party pool, through the clinical door when
/// `open`.
///
/// Each statement that confirms a request's policy copies then reads the
/// policy and definition revisions through `clinical_door.policy_revisions()`:
/// the two domains share a database and the party roles may run it (#6320).
pub async fn through_clinical_door<F: Future>(open: bool, fut: F) -> F::Output {
    if open {
        DOOR.scope((), fut).await
    } else {
        fut.await
    }
}

/// Whether a statement of the running request reads the revisions through
/// the clinical door.
///
/// It does on the party pool behind an open door ([`through_clinical_door`]),
/// inside a [`scope`] that confirms the copies after it ran.
#[must_use]
pub fn reads_clinical_door() -> bool {
    DOOR.try_with(|()| ()).is_ok() && SEEN.try_with(|_| ()).is_ok()
}

/// The newest revisions the running request's statements read so far;
/// `None` outside a [`scope`] or before any statement read them.
pub(crate) fn current() -> Option<Observed> {
    SEEN.try_with(Cell::get).ok().and_then(|seen| seen.observed)
}

/// Notes that the running request's write landed nothing after all: its
/// guarded statement refused it (#6256), so the request may run again.
pub(crate) fn not_written() {
    let _outside = SEEN.try_with(|seen| {
        seen.set(Seen {
            wrote: false,
            ..seen.get()
        });
    });
}

/// Notes that the running request committed a write; outside a [`scope`]
/// nothing is noted.
pub(crate) fn wrote() {
    // NOTE: tokio LocalKey::try_with, outside a scope no request confirms after it ran.
    let _outside = SEEN.try_with(|seen| {
        seen.set(Seen {
            wrote: true,
            ..seen.get()
        });
    });
}

/// Notes that a statement of the running request read `policy` and
/// `definitions`; outside a [`scope`] nothing is noted.
pub(crate) fn observe(policy: i64, definitions: i64) {
    // NOTE: tokio LocalKey::try_with, outside a scope no request confirms after it ran.
    let _outside = SEEN.try_with(|seen| {
        let held = seen.get();
        let next = match held.observed {
            Some(held) => Observed {
                policy: held.policy.max(policy),
                definitions: held.definitions.max(definitions),
            },
            None => Observed {
                policy,
                definitions,
            },
        };
        seen.set(Seen {
            observed: Some(next),
            ..held
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_newest_revisions_read_are_kept() {
        let ((), seen) = scope(async {
            observe(3, 7);
            observe(5, 6);
        })
        .await;
        assert_eq!(
            seen.observed,
            Some(Observed {
                policy: 5,
                definitions: 7
            })
        );
        assert!(!seen.wrote, "nothing was written");
    }

    #[tokio::test]
    async fn a_write_is_kept_beside_the_revisions_read() {
        let ((), seen) = scope(async {
            observe(2, 3);
            assert_eq!(
                current(),
                Some(Observed {
                    policy: 2,
                    definitions: 3
                })
            );
            wrote();
        })
        .await;
        assert!(seen.wrote, "the write is kept");
        assert_eq!(current(), None, "outside a scope nothing is read");
    }

    #[tokio::test]
    async fn nothing_read_is_none_and_outside_a_scope_nothing_is_kept() {
        observe(1, 1);
        let ((), seen) = scope(async {}).await;
        assert_eq!(seen, Seen::default());
    }
}
