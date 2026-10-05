// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    reason = "integration-test assertions outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
//! Parse time stays near-linear on invalid input.
//!
//! The parser reads attacker-typed query text, so a refusal must cost about
//! what the input length costs. The two `objectPath`-leading `nodePredicate`
//! alternatives (`AqlParser.g4` `nodePredicate`) share that prefix, and an
//! `objectPath`'s `pathPart` may carry another `pathPredicate` — so retrying
//! the alternatives instead of factoring the prefix doubles the work at every
//! bracket nesting level.

use openehr_query::parser::parse_str;

/// The 222-byte fuzz artifact: ~24 unclosed `[` predicate openers inside a
/// SELECT path expression. It is not valid AQL and must be refused; the point
/// of the test is how long the refusal takes.
const NESTED_PREDICATE_TIMEOUT: &str = "SELECT uuuucu0uat0ucu0u[uuuat0u[aT0u[uuuat0u[aT0v0valu[uuuat0u[aT0v0[uuuat0ucu0u[uuuat0u[aT0u[uuuat0u[aT0v0vacu0uat0ucu0u[uuuat0u[aT0u[uuuat0u[aT0v0valu[uuuat0u[aT0v0[uuuat0ucu0u[uuuat0u[aT0u[uuuat0u[alu[uuuat0u[aT0v0valu=";

/// A generous bound: measured on the pinned toolchain, the doubling refused
/// this input in 39.08 s in a debug build (and tripped the fuzz lane's
/// `-timeout` outright), while the factored grammar refuses in under a
/// millisecond — so the bound is nowhere near flaky in either direction.
const BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

#[test]
fn the_nested_predicate_artifact_is_refused_in_near_linear_time() {
    let started = std::time::Instant::now();
    let outcome = parse_str(NESTED_PREDICATE_TIMEOUT);
    let elapsed = started.elapsed();

    assert!(
        outcome.is_err(),
        "unclosed predicate brackets are not valid AQL, got {outcome:?}"
    );
    assert!(
        elapsed < BUDGET,
        "refusing the artifact took {elapsed:?}, budget {BUDGET:?} — the alternation is re-parsing its shared prefix per nesting level"
    );
}

/// The same property stated as a depth sweep, so a regression is caught even
/// if a future grammar change makes the recorded artifact fail earlier: 40
/// nesting levels are 2^40 alternative attempts under the doubling, which no
/// timeout could ever absorb.
#[test]
fn predicate_nesting_depth_does_not_multiply_parse_time() {
    let mut src = String::from("SELECT a");
    for _ in 0..40 {
        src.push_str("[b");
    }

    let started = std::time::Instant::now();
    let outcome = parse_str(&src);
    let elapsed = started.elapsed();

    assert!(
        outcome.is_err(),
        "forty unclosed predicate brackets are not valid AQL, got {outcome:?}"
    );
    assert!(
        elapsed < BUDGET,
        "refusing 40 nesting levels took {elapsed:?}, budget {BUDGET:?} — parse time is multiplying with depth"
    );
}
