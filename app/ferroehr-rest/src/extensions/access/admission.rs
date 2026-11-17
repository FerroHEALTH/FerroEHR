// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A read whose store round trip overlaps its access decision (#5959).
//!
//! The access middleware ([`super::ehr_access::enforce`] with the delay and
//! PEP checks) stays the one place that decides a request. For an operation
//! in [`OVERLAPPED`], `guarded_dispatch` runs that decision and the handler
//! side by side in one task, the handler polled first: the handler hands its
//! clinical read to the decision ([`read_once_admitted`]), which runs it
//! beside its party read (`ferroehr::access_eval::alongside`), and waits on
//! [`admitted`] before it uses the read for anything. `guarded_dispatch`
//! answers the decision's refusal whenever there is one, and drops whatever
//! the handler produced, so nothing of the read leaves the server before the
//! decision admits it. Every decision still rests on its own exact indexed
//! read (owner decision on #5891); only the waiting overlaps. No openEHR spec
//! governs this: our own design.

use std::future::Future;

use ferroehr::access_eval::alongside;
use openehr_its::rest::runtime::ApiError;
use tokio::sync::{oneshot, watch};

use crate::overview::error::RestError;

/// The operations whose handler reads the store while the decision is made:
/// each reads before [`admitted`] only what it would read once admitted, and
/// records, notes and returns nothing until then.
const OVERLAPPED: [&str; 2] = ["composition_get", "ehr_get_by_id"];

tokio::task_local! {
    static ADMISSION: watch::Receiver<Decided>;
}

/// What the decision answered, as the handler sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decided {
    /// The decision is still being made.
    Pending,
    /// The decision admitted the request.
    Admitted,
    /// The decision refused the request, or failed: the handler stops.
    Refused,
}

/// Whether `op` overlaps its handler's read with its decision.
pub(crate) fn overlaps(op: &str) -> bool {
    OVERLAPPED.contains(&op)
}

/// The decision's side of one overlapped request: it says once what the
/// decision answered. Dropped unsaid, it refuses.
#[derive(Debug)]
struct Decision(watch::Sender<Decided>);

impl Decision {
    /// Tells the handler whether the decision admitted the request.
    fn say(self, admitted: bool) {
        let decided = if admitted {
            Decided::Admitted
        } else {
            Decided::Refused
        };
        self.0.send_replace(decided);
    }
}

impl Drop for Decision {
    fn drop(&mut self) {
        // NOTE: our own design (#5959), a decision that ends without an answer fails closed.
        self.0.send_if_modified(|decided| {
            let pending = *decided == Decided::Pending;
            if pending {
                *decided = Decided::Refused;
            }
            pending
        });
    }
}

/// The handler's side of one overlapped request.
#[derive(Debug)]
struct Admission(watch::Receiver<Decided>);

impl Admission {
    /// Runs `handler` where [`admitted`] waits on this request's decision.
    fn scope<F: Future>(self, handler: F) -> impl Future<Output = F::Output> {
        ADMISSION.scope(self.0, handler)
    }
}

/// A pending decision and the admission its handler waits on.
fn pending() -> (Decision, Admission) {
    let (said, heard) = watch::channel(Decided::Pending);
    (Decision(said), Admission(heard))
}

/// Runs `decide` and `handler` side by side in this task, `handler` polled
/// first and where [`admitted`] waits on `decide`: `decide`'s refusal when it
/// refuses, whatever `handler` produced dropped unused, else `handler`'s
/// output.
///
/// # Errors
/// The refusal `decide` answers.
pub(crate) async fn overlap<D, H, R>(decide: D, handler: H) -> Result<H::Output, R>
where
    D: Future<Output = Result<(), R>>,
    H: Future,
{
    let (decision, waiting) = pending();
    let decide = async move {
        let decided = decide.await;
        decision.say(decided.is_ok());
        decided
    };
    let overlapped = async move {
        let (handled, decided) = tokio::join!(biased; waiting.scope(handler), decide);
        decided.map(|()| handled)
    };
    alongside::scope(overlapped).await
}

/// `read` handed to the access decision, once the request is admitted: the
/// decision runs it beside its party read when it makes one, and
/// [`Ahead::read`] runs it when the decision did not and the request needs it.
///
/// # Errors
/// [`admitted`]'s error when the request is not admitted.
pub(crate) async fn read_once_admitted<T: Send + 'static>(
    read: impl Future<Output = T> + Send + 'static,
) -> Result<Ahead<T>, RestError> {
    let (found, answer) = oneshot::channel();
    let left = alongside::register(async move {
        // NOTE: our own design (#5959), a request refused meanwhile has dropped its receiver.
        let _refused = found.send(read.await);
    });
    admitted().await?;
    Ok(Ahead { left, answer })
}

/// A read handed to the access decision of an admitted request.
pub(crate) struct Ahead<T> {
    /// The read the slot did not take, outside an overlapped request.
    left: Option<alongside::Read>,
    answer: oneshot::Receiver<T>,
}

impl<T> std::fmt::Debug for Ahead<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ahead").finish_non_exhaustive()
    }
}

impl<T> Ahead<T> {
    /// What the read found, read now when the decision did not run it.
    ///
    /// # Errors
    /// A `500` when the read ended without an answer.
    pub(crate) async fn read(self) -> Result<T, RestError> {
        match self.left {
            Some(read) => read.await,
            None => alongside::run().await,
        }
        self.answer.await.map_err(|_| {
            RestError::from(ApiError::Internal(
                "the read made beside the access decision ended unanswered".to_owned(),
            ))
        })
    }
}

/// Waits until the access decision of this request admits it.
///
/// Outside an overlapped request the decision was made before the handler
/// ran, so this answers at once.
///
/// # Errors
/// A `500` when the decision refused the request or ended without an
/// answer. `guarded_dispatch` answers the decision's own refusal in its
/// place, so the error only stops the handler.
pub(crate) async fn admitted() -> Result<(), RestError> {
    let Ok(mut heard) = ADMISSION.try_with(watch::Receiver::clone) else {
        return Ok(());
    };
    let decided = match heard.wait_for(|decided| *decided != Decided::Pending).await {
        Ok(decided) => *decided,
        Err(_) => Decided::Refused,
    };
    match decided {
        Decided::Admitted => Ok(()),
        Decided::Pending | Decided::Refused => Err(RestError::from(ApiError::Internal(
            "the request was not admitted".to_owned(),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// The clinical read of a handler: `read` long, counted in `reads`.
    async fn clinical(read: Duration, reads: Arc<AtomicUsize>) -> &'static str {
        tokio::time::sleep(read).await;
        reads.fetch_add(1, Ordering::SeqCst);
        "the clinical data"
    }

    /// A handler that hands its read to the decision and serves it once
    /// admitted; `served` says whether it got that far.
    async fn handler(
        read: Duration,
        reads: Arc<AtomicUsize>,
        served: Arc<AtomicBool>,
    ) -> Result<&'static str, RestError> {
        let data = read_once_admitted(clinical(read, reads)).await?.read().await?;
        served.store(true, Ordering::SeqCst);
        Ok(data)
    }

    /// A decision whose party read takes `wait`, the registered read beside
    /// it as `care_in_force` runs it when `party` holds, then `answer`.
    async fn decide(
        wait: Duration,
        party: bool,
        answer: Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if party {
            tokio::join!(tokio::time::sleep(wait), alongside::run());
        } else {
            tokio::time::sleep(wait).await;
        }
        answer
    }

    struct Seen {
        reads: Arc<AtomicUsize>,
        served: Arc<AtomicBool>,
    }

    impl Seen {
        fn new() -> Self {
            Self {
                reads: Arc::new(AtomicUsize::new(0)),
                served: Arc::new(AtomicBool::new(false)),
            }
        }

        fn handler(&self, read: Duration) -> impl Future<Output = Result<&'static str, RestError>> {
            handler(read, Arc::clone(&self.reads), Arc::clone(&self.served))
        }
    }

    const MS: Duration = Duration::from_millis(1);

    #[tokio::test]
    async fn outside_an_overlapped_request_admits_at_once() {
        assert!(admitted().await.is_ok());
    }

    #[tokio::test]
    async fn an_admitted_read_is_read_once_and_served() {
        for party in [true, false] {
            let seen = Seen::new();
            let out = overlap(decide(5 * MS, party, Ok(())), seen.handler(5 * MS)).await;
            assert_eq!(out.expect("admitted").expect("served"), "the clinical data");
            assert!(seen.served.load(Ordering::SeqCst));
            assert_eq!(seen.reads.load(Ordering::SeqCst), 1, "party read: {party}");
        }
    }

    #[tokio::test]
    async fn a_refused_read_never_serves_what_it_read() {
        for party in [true, false] {
            let seen = Seen::new();
            let out = overlap(decide(5 * MS, party, Err("403")), seen.handler(5 * MS)).await;
            assert_eq!(out.err(), Some("403"), "the refusal is the answer");
            assert!(!seen.served.load(Ordering::SeqCst), "the handler stopped at admission");
            let reads = usize::from(party);
            assert_eq!(seen.reads.load(Ordering::SeqCst), reads, "read only beside a party read");
        }
    }

    #[tokio::test]
    async fn a_decision_that_fails_fails_closed() {
        let seen = Seen::new();
        let out = overlap(
            decide(5 * MS, true, Err("503 the settings store did not answer")),
            seen.handler(5 * MS),
        )
        .await;
        assert_eq!(out.err(), Some("503 the settings store did not answer"));
        assert!(!seen.served.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_decision_dropped_unsaid_fails_closed() {
        let (decision, waiting) = pending();
        let (_, handled) = tokio::join!(async { drop(decision) }, waiting.scope(admitted()));
        assert!(handled.is_err());
    }

    #[tokio::test]
    async fn the_read_and_the_party_read_overlap() {
        // Timers, not CPU: the margin below is the other wait, which a serial run would add.
        let each = 300 * MS;
        let seen = Seen::new();
        let started = tokio::time::Instant::now();
        let out = overlap(decide(each, true, Ok(())), seen.handler(each)).await;
        let took = started.elapsed();
        assert!(out.is_ok_and(|served| served.is_ok()));
        assert!(took >= each, "the slower of the two bounds the request: {took:?}");
        assert!(took < each * 2 - each / 4, "the waits overlap, never add up: {took:?}");
    }

    #[test]
    fn only_the_two_reads_overlap() {
        assert!(overlaps("composition_get"));
        assert!(overlaps("ehr_get_by_id"));
        assert!(!overlaps("composition_create"));
        assert!(!overlaps("query_aql_post"));
    }
}
