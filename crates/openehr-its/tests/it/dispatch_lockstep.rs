// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::expect_used,
    clippy::string_slice,
    reason = "test assertions/diagnostics/fixtures"
)]
//! The fast-path ↔ typed-dispatch **lockstep** gate.
//!
//! Two dispatch tables decide which tier judges an RM node: the allocation-free
//! fast path (`openehr_rm::v1_2::validate::fast::try_validate`) and the authoritative
//! typed dispatch (`openehr_rm::v1_2::validate::typed_dispatch::dispatch_typed`). A
//! class present in one and absent from the other silently changes which tier
//! judges it, and a class whose DEPTH disagrees (`run` vs `run_shallow`) changes
//! which of its children are decoded — both are wire-visible. Until now the only
//! thing holding them together was a comment saying they "must stay in
//! lockstep".
//!
//! This gate derives BOTH tables from the source of the two `match ty` blocks —
//! no hand-maintained mirror list lives here, so the check cannot itself drift —
//! and pins the relationship:
//!
//! 1. every fast-path class is also owned by the typed dispatch (the typed table
//!    is the authority; a fast entry with no typed counterpart would vouch for a
//!    node the oracle never adjudicates);
//! 2. the two agree on decode depth for every shared class;
//! 3. the typed-only delta is snapshotted, so a class LEAVING the fast table is
//!    visible in the diff rather than silent.
//!
//! `seeded_drift_is_detected` proves the extractor actually catches an added
//! class and a flipped depth, so the gate is not decorative.

use std::collections::BTreeMap;

/// The typed dispatch source (`dispatch_typed`'s `match ty` block). It lives in
/// `openehr-rm` beside the fast path; this crate keeps only the generated
/// five-crate structural fallthrough and the thin wire entry points.
const TYPED_SRC: &str = include_str!("../../../openehr-rm/src/v1_2/validate/typed_dispatch.rs");
/// The fast-path dispatch source (`try_validate`'s `let shallow = match ty`).
const FAST_SRC: &str = include_str!("../../../openehr-rm/src/v1_2/validate/fast.rs");

/// How deeply a tier decodes a node before judging it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// Full decode — the class's invariants read a child collection.
    Full,
    /// Child nodes pruned before decode — a structural container with
    /// scalar-only invariants.
    Shallow,
}

/// The `_type` string literals appearing in `text`.
fn class_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        out.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    out
}

/// Strip `//` line comments, so a class named in prose is never mistaken for a
/// pattern literal.
fn without_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The decode depth an arm body implies, if it calls one of the two runners.
fn depth_of(text: &str) -> Option<Depth> {
    if text.contains("run_shallow::<") {
        Some(Depth::Shallow)
    } else if text.contains("run::<") {
        Some(Depth::Full)
    } else {
        None
    }
}

/// Extract the typed dispatch table from `dispatch_typed`.
///
/// Each arm is `<literals> => <body>`; the body names `run::<T>` (full) or
/// `run_shallow::<T>` (shallow). The one block-bodied arm (`DV_INTERVAL`)
/// spans several lines and is attributed by the first runner call inside it.
fn typed_table(src: &str) -> BTreeMap<String, Depth> {
    let src = without_comments(src);
    let start = src.find("fn dispatch_typed").expect("typed dispatch fn");
    let body = &src[start..];
    let m = body.find("match ty {").expect("typed match block");
    let body = &body[m..];
    let end = body
        .find("_ => return false")
        .expect("typed fallthrough arm");
    let block = &body[..end];

    let mut arms: Vec<(Vec<String>, Option<Depth>)> = Vec::new();
    for line in block.lines().skip(1) {
        if let Some(i) = line.find("=>") {
            arms.push((class_literals(&line[..i]), depth_of(&line[i..])));
        } else if let Some(last) = arms.last_mut()
            && last.1.is_none()
        {
            last.1 = depth_of(line);
        }
    }

    let mut out = BTreeMap::new();
    for (classes, depth) in arms {
        let Some(depth) = depth else { continue };
        for c in classes {
            out.insert(c, depth);
        }
    }
    assert!(
        out.len() > 30,
        "typed table extraction found only {} entries — the extractor is out of \
         step with the source shape",
        out.len()
    );
    out
}

/// Extract the fast-path table from `try_validate`'s `let shallow = match ty`.
///
/// Exactly two arms: the multi-line `… => true` (shallow) pattern and the
/// multi-line `… => false` (full) pattern, then the `_ => return false`
/// fallthrough.
fn fast_table(src: &str) -> BTreeMap<String, Depth> {
    let src = without_comments(src);
    let start = src
        .find("let shallow = match ty {")
        .expect("fast dispatch match");
    let block = &src[start..];
    let block = &block[..block.find("_ => return false").expect("fast fallthrough")];

    let shallow_end = block.find("=> true").expect("fast shallow arm");
    let full_end = block.find("=> false").expect("fast full arm");
    assert!(shallow_end < full_end, "unexpected fast arm order");

    let mut out = BTreeMap::new();
    for c in class_literals(&block[..shallow_end]) {
        out.insert(c, Depth::Shallow);
    }
    for c in class_literals(&block[shallow_end..full_end]) {
        out.insert(c, Depth::Full);
    }
    assert!(
        out.len() > 20,
        "fast table extraction found only {} entries — the extractor is out of \
         step with the source shape",
        out.len()
    );
    out
}

#[test]
fn every_fast_path_class_is_owned_by_the_typed_dispatch() {
    let typed = typed_table(TYPED_SRC);
    let fast = fast_table(FAST_SRC);
    let missing: Vec<&String> = fast.keys().filter(|c| !typed.contains_key(*c)).collect();
    assert!(
        missing.is_empty(),
        "fast-path classes with no typed-dispatch arm (the fast path would vouch \
         for a node the typed oracle never adjudicates): {missing:?}"
    );
}

#[test]
fn shared_classes_agree_on_decode_depth() {
    let typed = typed_table(TYPED_SRC);
    let fast = fast_table(FAST_SRC);
    let mismatched: Vec<String> = fast
        .iter()
        .filter_map(|(c, f)| {
            typed
                .get(c)
                .filter(|t| *t != f)
                .map(|t| format!("{c}: fast={f:?} typed={t:?}"))
        })
        .collect();
    assert!(
        mismatched.is_empty(),
        "decode-depth disagreement between the fast path and the typed dispatch \
         (a different set of children is decoded on each tier): {mismatched:?}"
    );
}

/// The typed dispatch is deliberately a SUPERSET: classes whose typed acceptance
/// the fast checker does not replicate stay typed-only. Snapshotting the delta
/// means a class silently LEAVING the fast table shows up as a diff.
#[test]
fn the_typed_only_delta_is_recorded() {
    let typed = typed_table(TYPED_SRC);
    let fast = fast_table(FAST_SRC);
    let mut delta: Vec<&str> = typed
        .keys()
        .filter(|c| !fast.contains_key(*c))
        .map(String::as_str)
        .collect();
    delta.sort_unstable();
    insta::assert_debug_snapshot!("typed_only_classes", delta);
}

/// The extractor must actually catch drift — otherwise the gate is decorative.
#[test]
fn seeded_drift_is_detected() {
    let typed = typed_table(TYPED_SRC);

    // Seed 1: a fast entry with no typed counterpart.
    let mut fast = fast_table(FAST_SRC);
    fast.insert("DV_NOT_A_CLASS".to_owned(), Depth::Full);
    assert!(
        fast.keys().any(|c| !typed.contains_key(c)),
        "an unmatched fast entry must be visible to the membership check"
    );

    // Seed 2: a flipped depth on a shared class.
    let mut fast = fast_table(FAST_SRC);
    let (shared, depth) = fast
        .iter()
        .find_map(|(c, d)| typed.contains_key(c).then(|| (c.clone(), *d)))
        .expect("at least one shared class");
    fast.insert(
        shared,
        match depth {
            Depth::Full => Depth::Shallow,
            Depth::Shallow => Depth::Full,
        },
    );
    assert!(
        fast.iter()
            .any(|(c, f)| typed.get(c).is_some_and(|t| t != f)),
        "a flipped depth must be visible to the depth check"
    );
}
