// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! The one nesting limit of every RM reader, and the smallest thread stack
//! that reads RM.
//!
//! No openEHR specification bounds document nesting: our own design (#5997).
//! The RM is recursive (`SECTION.items` holds sections, `CLUSTER.items` holds
//! clusters, `FOLDER.folders` holds folders), and each generated reader takes
//! one stack frame per nested RM object. A stack overflow aborts the process,
//! which no panic handler can catch, so every reader refuses a document nested
//! deeper than [`MAX_RM_NESTING`] RM objects. The canonical-JSON reader, the
//! canonical-XML reader and the web-template formats, which build their RM
//! through those readers, count the same objects against the same limit.
//!
//! [`MIN_RM_READ_STACK_BYTES`] is the stack a reader needs for a document at
//! the limit, in a debug build, with room left for its caller. Every thread
//! that reads RM runs on at least this much; the release and debug tests of
//! `openehr-its` read a document at the limit on a thread of exactly this
//! size.

use std::cell::Cell;

/// The deepest nesting of RM objects a document may carry.
///
/// The deepest real structure, `COMPOSITION` → `SECTION`s → `OBSERVATION` →
/// `HISTORY` → `EVENT` → `ITEM_TREE` → nested `CLUSTER`s → `ELEMENT` →
/// `DV_CODED_TEXT` → `CODE_PHRASE` → `TERMINOLOGY_ID`, is under 30 objects in
/// the most elaborate published templates. The limit also sits under the
/// 128-level recursion limit of `serde_json` (one RM object and one array per
/// level at most), so the canonical-JSON reader refuses a deep document here
/// and never on that limit.
pub const MAX_RM_NESTING: u32 = 48;

/// The smallest stack, in bytes, of any thread that reads RM: the platform's
/// default thread stack of 2 MiB, which a document at [`MAX_RM_NESTING`] fits
/// in a debug build.
pub const MIN_RM_READ_STACK_BYTES: usize = 2 * 1024 * 1024;

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// One RM object being read; dropping it leaves the object.
#[derive(Debug)]
#[must_use = "the guard counts the object only while it is held"]
pub struct NestingGuard(());

impl Drop for NestingGuard {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// A document nested deeper than [`MAX_RM_NESTING`] RM objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestingTooDeep;

impl std::fmt::Display for NestingTooDeep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RM object nesting exceeds the {MAX_RM_NESTING}-level limit"
        )
    }
}

impl std::error::Error for NestingTooDeep {}

/// Enter one RM object on this thread.
///
/// # Errors
/// [`NestingTooDeep`] when the object would be nested deeper than
/// [`MAX_RM_NESTING`].
pub fn enter() -> Result<NestingGuard, NestingTooDeep> {
    DEPTH.with(|d| {
        let next = d.get().saturating_add(1);
        if next > MAX_RM_NESTING {
            return Err(NestingTooDeep);
        }
        d.set(next);
        Ok(NestingGuard(()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_one_past_the_limit_and_recovers() {
        let mut guards = Vec::new();
        for _ in 0..MAX_RM_NESTING {
            guards.push(enter().expect("within the limit"));
        }
        assert_eq!(enter().map(|_| ()), Err(NestingTooDeep));
        guards.pop();
        let again = enter();
        assert!(again.is_ok());
        drop(again);
        drop(guards);
        assert!(enter().is_ok());
    }
}
