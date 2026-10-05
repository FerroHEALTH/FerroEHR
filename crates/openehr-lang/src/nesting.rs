// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The nesting bound every recursive reader and walker in the openEHR
//! language stack honours.
//!
//! The ODIN and BEL readers here, and the cADL parser, flattener and OPT
//! transform built on them in `openehr-adl`, recurse over the structure they
//! read. Native recursion has no typed failure mode: past the thread's stack
//! it aborts the process, which no caller can catch. [`MAX_NESTING_DEPTH`] is
//! the one bound they all share, and [`Nesting`] is the counter a recursive
//! walk threads through itself so that crossing the bound is a typed refusal
//! ([`NestingExceeded`]) instead. No openEHR spec bounds nesting — this is an
//! implementation limit, set far above any published artefact.

/// The deepest nesting any reader or walker in this stack accepts.
///
/// The published openEHR archetypes and templates stay well under a hundred
/// levels; the bound is a refusal threshold for defective or hostile input,
/// not a design envelope. A walk reaches the bound before it refuses, so the
/// caller provides a stack sized for it: the CDR runs the engine on a
/// dedicated 256 MiB thread, and a debug build on a default 2 MiB thread
/// overflows first.
pub const MAX_NESTING_DEPTH: usize = 512;

/// A nesting level in a recursive walk, refusing to descend past
/// [`MAX_NESTING_DEPTH`].
///
/// Value semantics: [`Nesting::descend`] returns the deeper level for the
/// recursive call and leaves the caller's own level untouched, so unwinding
/// needs no bookkeeping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Nesting(usize);

impl Nesting {
    /// The outermost level.
    pub const ROOT: Self = Self(0);

    /// The current level, `0` at the root.
    #[must_use]
    pub const fn level(self) -> usize {
        self.0
    }

    /// The level one deeper.
    ///
    /// # Errors
    /// [`NestingExceeded`] when the deeper level would pass
    /// [`MAX_NESTING_DEPTH`].
    pub const fn descend(self) -> Result<Self, NestingExceeded> {
        if self.0 >= MAX_NESTING_DEPTH {
            return Err(NestingExceeded {
                limit: MAX_NESTING_DEPTH,
            });
        }
        Ok(Self(self.0 + 1))
    }

    /// The level one shallower, for walkers that keep one counter and unwind
    /// it by hand; saturates at the root.
    #[must_use]
    pub const fn ascend(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

/// A structure nests deeper than [`MAX_NESTING_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("nesting exceeds the limit of {limit} levels")]
pub struct NestingExceeded {
    /// The bound that was crossed.
    pub limit: usize,
}

/// The deepest bracket nesting in a token stream, counted by `opens` and
/// `closes`, refused when it passes [`MAX_NESTING_DEPTH`].
///
/// A combinator parser has no seam for a per-level counter, so its recursion
/// is bounded up front by the bracket structure that drives it: every
/// recursive production opens a bracket, so the bracket depth bounds the
/// recursion depth. Returns the index of the first token that crosses the
/// bound.
///
/// # Errors
/// [`NestingExceeded`] with the offending token's index attached by the
/// caller, when the nesting passes the bound.
pub fn check_bracket_nesting<T>(
    tokens: &[T],
    opens: impl Fn(&T) -> bool,
    closes: impl Fn(&T) -> bool,
) -> Result<(), (usize, NestingExceeded)> {
    let mut level = Nesting::ROOT;
    for (index, token) in tokens.iter().enumerate() {
        if opens(token) {
            level = level.descend().map_err(|e| (index, e))?;
        } else if closes(token) {
            level = level.ascend();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descend_stops_exactly_at_the_bound() {
        let mut level = Nesting::ROOT;
        for _ in 0..MAX_NESTING_DEPTH {
            level = level.descend().expect("within the bound");
        }
        assert_eq!(level.level(), MAX_NESTING_DEPTH);
        assert_eq!(
            level.descend(),
            Err(NestingExceeded {
                limit: MAX_NESTING_DEPTH
            })
        );
        assert_eq!(level.ascend().level(), MAX_NESTING_DEPTH - 1);
        assert_eq!(Nesting::ROOT.ascend(), Nesting::ROOT);
    }

    #[test]
    fn bracket_nesting_reports_the_first_crossing_token() {
        let opens = |c: &char| *c == '(';
        let closes = |c: &char| *c == ')';
        let balanced: Vec<char> = "(()())".chars().collect();
        assert_eq!(check_bracket_nesting(&balanced, opens, closes), Ok(()));

        let mut deep: Vec<char> = std::iter::repeat_n('(', MAX_NESTING_DEPTH).collect();
        deep.extend(std::iter::repeat_n(')', MAX_NESTING_DEPTH));
        assert_eq!(check_bracket_nesting(&deep, opens, closes), Ok(()));

        deep.insert(MAX_NESTING_DEPTH, '(');
        assert_eq!(
            check_bracket_nesting(&deep, opens, closes),
            Err((
                MAX_NESTING_DEPTH,
                NestingExceeded {
                    limit: MAX_NESTING_DEPTH
                }
            ))
        );
    }
}
