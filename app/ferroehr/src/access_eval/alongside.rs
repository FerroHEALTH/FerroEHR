// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A read a request hands to its own access decision, to run while the
//! decision waits on the party store (#5959).
//!
//! A care-decided verdict waits on one indexed read of the store's revisions
//! (`LiveGrants::behind`, owner decision on #5891). The read the request
//! makes once admitted, its version or its EHR, goes to the clinical store.
//! The two can run together, but never on one store: a request that holds a
//! connection of a pool never asks it for a second one (#5879). So the
//! request registers its read with [`register`], and the decision runs it
//! with [`beside`] next to its party read and nowhere else, when the request
//! will use it. A decision that makes no such wait leaves it registered, and
//! the request runs it with [`run`] once admitted, as it would have before.
//! Nothing the read finds leaves the request until the decision admits it:
//! the registered future only stores its result where the request alone
//! reads it. No openEHR spec governs this: our own design.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;

/// A registered read: it runs to its end and keeps what it found itself.
pub type Read = Pin<Box<dyn Future<Output = ()> + Send>>;

/// When an admitted request uses the read it registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// Whenever the request is admitted.
    Admitted,
    /// Only when the EHR's verdict depends on the item, which the read's
    /// own row then judges; otherwise the request reads by another path.
    ItemJudged,
}

tokio::task_local! {
    static READ: RefCell<Option<(Use, Read)>>;
}

/// Runs `body` with a slot for one registered read.
pub async fn scope<F: Future>(body: F) -> F::Output {
    READ.scope(RefCell::new(None), body).await
}

/// Registers `read`, used as `uses` says, for the decision to run beside its
/// party read; outside a [`scope`], or with a read already registered, it is
/// returned for the caller to run.
#[must_use = "a read the slot did not take must still be run"]
pub fn register(uses: Use, read: impl Future<Output = ()> + Send + 'static) -> Option<Read> {
    let read: Read = Box::pin(read);
    let mut read = Some(read);
    // NOTE: our own design (#5959), outside a scope there is no decision to run the read.
    let _taken = READ.try_with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut()
            && slot.is_none()
        {
            *slot = read.take().map(|read| (uses, read));
        }
    });
    read
}

/// Runs the registered read beside the decision's party read.
///
/// It runs only a read the request will use: always for [`Use::Admitted`],
/// and for [`Use::ItemJudged`] when the verdict being confirmed judges items
/// (`item_judged`). A read it leaves stays registered for [`run`].
pub async fn beside(item_judged: bool) {
    let wanted = |uses: Use| uses == Use::Admitted || item_judged;
    if let Some(read) = take(wanted) {
        read.await;
    }
}

/// Runs the registered read, if any, for the request once admitted.
pub async fn run() {
    if let Some(read) = take(|_| true) {
        read.await;
    }
}

/// Takes the registered read out of the slot when `wanted` takes its use.
fn take(wanted: impl FnOnce(Use) -> bool) -> Option<Read> {
    READ.try_with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        let (uses, _) = slot.as_ref()?;
        if !wanted(*uses) {
            return None;
        }
        slot.take().map(|(_, read)| read)
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counting(runs: &Arc<AtomicUsize>) -> impl Future<Output = ()> + Send + 'static {
        let runs = Arc::clone(runs);
        async move {
            runs.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn a_registered_read_runs_once_by_the_decision() {
        let runs = Arc::new(AtomicUsize::new(0));
        scope(async {
            assert!(register(Use::Admitted, counting(&runs)).is_none());
            beside(false).await;
            run().await;
        })
        .await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_read_judged_on_its_item_runs_beside_an_item_verdict_alone() {
        let runs = Arc::new(AtomicUsize::new(0));
        scope(async {
            assert!(register(Use::ItemJudged, counting(&runs)).is_none());
            beside(false).await;
            assert_eq!(
                runs.load(Ordering::SeqCst),
                0,
                "a verdict on the EHR alone leaves it"
            );
            beside(true).await;
            assert_eq!(runs.load(Ordering::SeqCst), 1, "a verdict on items runs it");
            run().await;
        })
        .await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn outside_a_scope_the_read_comes_back() {
        let runs = Arc::new(AtomicUsize::new(0));
        let back = register(Use::Admitted, counting(&runs));
        beside(true).await;
        run().await;
        assert_eq!(runs.load(Ordering::SeqCst), 0, "no decision ran it");
        back.expect("the read comes back").await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
