// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stages of a request whose thread CPU the hot-path budget reads (#6321).
//!
//! With the `stage-cpu` feature each mark is a `trace` span under [`TARGET`],
//! named for its stage, which the budget's layer times with the thread's CPU
//! clock. Without it, the feature off as in every server build, [`enter`]
//! returns a value of no size and [`instrument`] returns the future it was
//! given, so a mark compiles to nothing.
//!
//! No openEHR specification governs this: our own design.

use std::future::Future;

/// The target of every stage span.
pub const TARGET: &str = "ferroehr::stage";

/// A stage of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Authenticating the caller and reading its binding and grants.
    Authentication,
    /// The access decision: RBAC, SMART scopes, ABAC, care relationships and
    /// caseload.
    Access,
    /// Reading the request body into its typed form.
    Decode,
    /// The handler and its store calls, the stages it runs left out.
    Handler,
    /// Encoding the answer.
    Encode,
    /// Building, encoding and digesting the access record and its
    /// access-log entry.
    Entry,
    /// Encoding and sending the gathered access-log intake statement.
    Gather,
}

impl Stage {
    /// Every stage, in the order a request meets them.
    pub const ALL: [Self; 7] = [
        Self::Authentication,
        Self::Access,
        Self::Decode,
        Self::Handler,
        Self::Encode,
        Self::Entry,
        Self::Gather,
    ];

    /// The stage's name, which is also its span's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Access => "access",
            Self::Decode => "decode",
            Self::Handler => "handler",
            Self::Encode => "encode",
            Self::Entry => "entry",
            Self::Gather => "gather",
        }
    }

    /// The stage a span name names, when it names one.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.name() == name)
    }

    /// The stage's span.
    #[cfg(feature = "stage-cpu")]
    fn span(self) -> tracing::Span {
        match self {
            Self::Authentication => tracing::trace_span!(target: TARGET, "authentication"),
            Self::Access => tracing::trace_span!(target: TARGET, "access"),
            Self::Decode => tracing::trace_span!(target: TARGET, "decode"),
            Self::Handler => tracing::trace_span!(target: TARGET, "handler"),
            Self::Encode => tracing::trace_span!(target: TARGET, "encode"),
            Self::Entry => tracing::trace_span!(target: TARGET, "entry"),
            Self::Gather => tracing::trace_span!(target: TARGET, "gather"),
        }
    }
}

/// A stage entered on the running thread until it is dropped.
#[cfg(feature = "stage-cpu")]
pub type Entered = tracing::span::EnteredSpan;

/// A stage entered on the running thread until it is dropped; nothing
/// without the `stage-cpu` feature.
#[cfg(not(feature = "stage-cpu"))]
#[derive(Debug)]
#[must_use = "the stage is left when this is dropped"]
pub struct Entered;

/// Enters `stage` on the running thread until the returned value is dropped.
///
/// Only for synchronous code: a value held across an `.await` would charge
/// the stage with whatever the thread polls meanwhile; a future takes
/// [`instrument`].
#[cfg(feature = "stage-cpu")]
#[inline]
pub fn enter(stage: Stage) -> Entered {
    stage.span().entered()
}

/// Enters `stage` on the running thread until the returned value is dropped;
/// nothing without the `stage-cpu` feature.
#[cfg(not(feature = "stage-cpu"))]
#[inline]
pub fn enter(_: Stage) -> Entered {
    Entered
}

/// `future` with `stage` entered while it is polled.
#[cfg(feature = "stage-cpu")]
#[inline]
pub fn instrument<F: Future>(stage: Stage, future: F) -> impl Future<Output = F::Output> {
    tracing::Instrument::instrument(future, stage.span())
}

/// `future` itself: without the `stage-cpu` feature a stage marks nothing.
#[cfg(not(feature = "stage-cpu"))]
#[inline]
pub fn instrument<F: Future>(_: Stage, future: F) -> F {
    future
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_is_named_once() {
        for stage in Stage::ALL {
            assert_eq!(Stage::named(stage.name()), Some(stage));
        }
        assert_eq!(Stage::named("not a stage"), None);
    }

    /// Without the feature a mark holds nothing and wraps nothing, so a server
    /// build pays for none (#6321).
    #[cfg(not(feature = "stage-cpu"))]
    #[test]
    fn a_mark_costs_nothing_without_the_feature() {
        assert_eq!(size_of::<Entered>(), 0);
        let future = std::future::ready(7_u8);
        let size = size_of_val(&future);
        let marked = instrument(Stage::Handler, future);
        assert_eq!(size_of_val(&marked), size);
        let _: std::future::Ready<u8> = marked;
    }
}
