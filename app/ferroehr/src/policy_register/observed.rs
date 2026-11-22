// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy and definition revisions a request's own statements read
//! (#6251).
//!
//! A statement that judges an EHR selects its settings revision from
//! `access_revision_of`, and with it the policy and definition revisions, at
//! the snapshot of the rows it reads. A request whose policy copies are
//! confirmed after it ran takes from here the newest revisions its statements
//! saw, and its answer leaves only once the copies hold them. No openEHR spec
//! governs the copies: our own design.

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

tokio::task_local! {
    static SEEN: Cell<Option<Observed>>;
}

/// Runs `fut` gathering the revisions its statements read, and returns them
/// with its output; `None` when no statement read them.
pub async fn scope<F: Future>(fut: F) -> (F::Output, Option<Observed>) {
    SEEN.scope(Cell::new(None), async move {
        let output = fut.await;
        (output, SEEN.with(Cell::get))
    })
    .await
}

/// Notes that a statement of the running request read `policy` and
/// `definitions`; outside a [`scope`] nothing is noted.
pub(crate) fn observe(policy: i64, definitions: i64) {
    // NOTE: tokio LocalKey::try_with, outside a scope no request confirms after it ran.
    let _outside = SEEN.try_with(|seen| {
        let next = match seen.get() {
            Some(held) => Observed {
                policy: held.policy.max(policy),
                definitions: held.definitions.max(definitions),
            },
            None => Observed {
                policy,
                definitions,
            },
        };
        seen.set(Some(next));
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
            seen,
            Some(Observed {
                policy: 5,
                definitions: 7
            })
        );
    }

    #[tokio::test]
    async fn nothing_read_is_none_and_outside_a_scope_nothing_is_kept() {
        observe(1, 1);
        let ((), seen) = scope(async {}).await;
        assert_eq!(seen, None);
    }
}
